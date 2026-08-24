//! Multi-seed / multi-config search shell.
//!
//! htd-inspired shape: enumerate a small set of `(config, seed)` pairs, build
//! a `TreeDecomposition` for each, and return all of them. Scoring/selection
//! happens one level up in `mod.rs`, which runs `td_to_vtree_best` on each TD
//! and picks by `vtree_cost`.

use std::collections::VecDeque;

use super::super::{GraphKind, TreeDecomposition};
use super::build_td::build_td_from_steps;
use super::graph::Graph;
use super::minfill_core::{
    self, ElimExit, ElimSteps, ElimStop, eliminate_mindegree, eliminate_mindegree_sampling,
    eliminate_minfill, eliminate_minfill_sampling, exceeds_width_bound,
};
use super::nested_diss::{self, DEFAULT_BASE_THRESH};
use super::preprocess::{Reduced, preprocess};
use crate::budget::expired;
use crate::decompose::rng::{SEED_OFFSET, Xorshift64};

/// Which algorithmic core to use for the post-preprocessing elimination order.
///
/// The two sampling cores carry the per-vertex weight they sample tie sets
/// with, so naming one of them and having a weight to hand are the same act.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Config<'a> {
    /// Preprocess → min-fill on the reduced graph.
    MinFill,
    /// Preprocess → min-degree on the reduced graph. Cheaper per step than
    /// min-fill; wins on some sparse graphs where degree ordering dominates.
    MinDegree,
    /// Preprocess → nested dissection via `multilevel_bisect` with a
    /// König-Egerváry minimum vertex cover as the separator (flow-theoretic).
    /// The earlier smaller-boundary variant is dropped because min-cover is
    /// provably `≤` smaller-boundary at every level.
    NestedDissMinCover,
    /// htd-style min-fill: priority = fill only (no `degree` secondary key),
    /// ties broken by **JW-weighted sampling** from the full min-fill tie set.
    /// The shape is `htd::MinFillOrderingAlgorithm`'s; the sampling weight
    /// carries the SAT signal (see `to_sample_weight`, which inverts
    /// `sat_score::compute_weight` into the sampler's convention).
    MinFillSampleJW {
        /// Per-vertex SAT weight, one entry per vertex of the graph this core
        /// is run over.
        weight: &'a [u32],
    },
    /// htd-style min-degree: priority = degree only, ties broken by the same
    /// JW-weighted sampling from the tie set.
    MinDegreeSampleJW {
        /// Per-vertex SAT weight, as [`Config::MinFillSampleJW`] carries it.
        weight: &'a [u32],
    },
}

impl<'a> Config<'a> {
    /// The weight this core samples tie sets with, or `None` for the
    /// deterministic cores, which read none.
    pub(super) fn sat_weight(self) -> Option<&'a [u32]> {
        match self {
            Config::MinFillSampleJW { weight } | Config::MinDegreeSampleJW { weight } => {
                Some(weight)
            }
            Config::MinFill | Config::MinDegree | Config::NestedDissMinCover => None,
        }
    }

    /// The same core reading `weight` instead — how a caller that re-indexed
    /// the vertices (a component solved on its own numbering) points the
    /// sampling at the slice that matches.
    pub(super) fn with_sat_weight<'b>(self, weight: &'b [u32]) -> Config<'b> {
        match self {
            Config::MinFill => Config::MinFill,
            Config::MinDegree => Config::MinDegree,
            Config::NestedDissMinCover => Config::NestedDissMinCover,
            Config::MinFillSampleJW { .. } => Config::MinFillSampleJW { weight },
            Config::MinDegreeSampleJW { .. } => Config::MinDegreeSampleJW { weight },
        }
    }
}

/// One run of `(config, seed)`: the produced decomposition, and how the
/// elimination that built it ended.
pub(crate) struct ConfigRun {
    pub td: TreeDecomposition,
    /// How the elimination ended. `Natural` is a clean run. `DeadlineBailed`
    /// and `WidthAborted` both mean the elimination stopped early and the
    /// returned `td` is incomplete (partial bags) — the mod.rs schedule MUST
    /// record a `td: None` stub in either case, unless it asked for
    /// `force_emit` and the exit is `DeadlineBailed`, in which case the
    /// emergency path fill has already completed the decomposition.
    pub exit: ElimExit,
}

/// Graph + preprocessing result, shared across every config in a schedule.
/// `Graph::from_edges` and `preprocess` are deterministic (no salt/seed), so
/// running them once and cloning per config replaces 5 builds with 1 build +
/// 4 clones when the schedule runs to completion.
pub(super) struct Prebuilt {
    reduced: Reduced,
    /// Cached CC count from the preprocessed residual (BFS on adj).
    num_components: usize,
    /// Pre-computed fill counts for each residual vertex.
    /// Bitset mode: via `fill_count_of_bs`; adj mode: via `FillScratch::fill_count_of`.
    /// Amortized across all refinement seeds: O(n·d²) computed once instead of
    /// once per slot.
    initial_fill: Vec<u64>,
}

/// Build graph + preprocess once for reuse across every config in a schedule.
pub(super) fn prebuild(total_vertices: u32, edges: &[(u32, u32)]) -> Prebuilt {
    let graph = Graph::from_edges(total_vertices, edges);
    let reduced = preprocess(graph);
    let num_components = find_connected_components(&reduced.graph).len();
    let initial_fill: Vec<u64> = if reduced.graph.bitset_words > 0 {
        (0..reduced.graph.len())
            .map(|v| {
                if reduced.graph.active[v] {
                    reduced.graph.fill_count_of_bs(v as u32)
                } else {
                    0
                }
            })
            .collect()
    } else {
        minfill_core::compute_initial_fill(&reduced.graph)
    };
    Prebuilt {
        reduced,
        num_components,
        initial_fill,
    }
}

impl Prebuilt {
    /// Number of active vertices in the preprocessed residual graph.
    pub(super) fn num_active(&self) -> usize {
        self.reduced.graph.num_active
    }
}

/// One slot of the search: which core to run, on which RNG stream, under which
/// cutoffs. It travels unchanged from the schedule down to the elimination
/// core, so a path that needs a variation — the per-component solver, with its
/// own seed and its own slice of the weights — re-points those fields and
/// passes the rest straight through.
///
#[derive(Clone, Copy)]
pub(super) struct RunSpec<'a> {
    /// Which algorithmic core produces the elimination order, carrying the
    /// sampling weight if it is one that reads one.
    pub(super) config: Config<'a>,
    /// Selects the RNG stream for salt and tie-set sampling.
    pub(super) seed: u64,
    /// When the elimination must stop — handed to the core whole.
    pub(super) stop: ElimStop,
    /// Whether a hard-deadline bail must still leave a complete TD behind.
    pub(super) force_emit: bool,
}

/// Run a single spec using a pre-built / pre-preprocessed graph. Clones
/// `prebuilt.reduced` so the caller can reuse it.
pub(super) fn run_config_prebuilt(
    kind: GraphKind,
    num_vars: u32,
    prebuilt: &Prebuilt,
    spec: RunSpec<'_>,
) -> ConfigRun {
    let single_component_bitset =
        prebuilt.num_components == 1 && prebuilt.reduced.graph.bitset_words > 0;
    let reduced = if single_component_bitset {
        Reduced {
            graph: prebuilt.reduced.graph.clone_bitset_only(),
            prefix: prebuilt.reduced.prefix.clone(),
        }
    } else {
        prebuilt.reduced.clone()
    };
    // Pass cached fill counts for all single-component graphs (bitset and adj
    // mode). Multi-component graphs use local indexing per component and can't
    // share the global initial_fill.
    let initial_fill = if prebuilt.num_components == 1 {
        Some(prebuilt.initial_fill.as_slice())
    } else {
        None
    };
    run_config_on_reduced(
        kind,
        num_vars,
        reduced,
        single_component_bitset,
        initial_fill,
        spec,
    )
}

/// BFS connected-component finder on the active residual. Returns one Vec<u32>
/// per component; each vec contains the original vertex ids in visit order.
/// Uses `collect_live_nbrs_into` (bitset-aware): after `preprocess` runs
/// `eliminate_with_nbrs_bs`, `adj` is no longer maintained and reading it
/// directly would produce stale neighbour lists (missing fill edges, extra
/// eliminated entries) and partition the graph incorrectly.
fn find_connected_components(graph: &Graph) -> Vec<Vec<u32>> {
    let n = graph.len();
    let mut visited = vec![false; n];
    let mut components: Vec<Vec<u32>> = Vec::new();
    let mut nbrs_buf: Vec<u32> = Vec::new();
    for start in 0..n {
        if !graph.active[start] || visited[start] {
            continue;
        }
        let mut comp: Vec<u32> = Vec::new();
        let mut queue = VecDeque::new();
        visited[start] = true;
        queue.push_back(start as u32);
        while let Some(v) = queue.pop_front() {
            comp.push(v);
            nbrs_buf.clear();
            graph.collect_live_nbrs_into(v, &mut nbrs_buf);
            for &u in &nbrs_buf {
                let ui = u as usize;
                if graph.active[ui] && !visited[ui] {
                    visited[ui] = true;
                    queue.push_back(u);
                }
            }
        }
        components.push(comp);
    }
    components
}

/// Run elimination on an already-preprocessed residual and return the raw bags
/// and rank_pairs (in the graph's own vertex indices).
///
/// The one place that maps a `Config` onto an elimination core — both the
/// whole-residual and per-component callers go through it. Neither gets a
/// `ConfigRun` back — the whole-residual caller runs `finalize` on the raw
/// output, the per-component caller first translates component-local indices
/// back to originals and concatenates into the global flat list that
/// `build_td_from_steps` consumes.
///
/// `initial_fill` is the caller's cached per-vertex fill count for this exact
/// graph; the min-fill sampling cores are the only ones that read it. The
/// per-component caller has none to offer: each component is re-indexed from 0,
/// so the whole-residual counts do not apply to it.
fn run_elimination_raw(
    reduced: Reduced,
    salt: &[u32],
    initial_fill: Option<&[u64]>,
    spec: RunSpec<'_>,
) -> (ElimSteps, ElimExit) {
    let mut steps = reduced.prefix;
    let mut g = reduced.graph;

    let exit = match spec.config {
        Config::MinFill => eliminate_minfill(&mut g, salt, steps.sink(), spec.stop),
        Config::MinDegree => eliminate_mindegree(&mut g, salt, steps.sink(), spec.stop),
        Config::MinFillSampleJW { weight } => {
            let sw = to_sample_weight(weight);
            eliminate_minfill_sampling(
                &mut g,
                &sw,
                spec.seed,
                steps.sink(),
                ElimStop {
                    deadline: None,
                    ..spec.stop
                },
                initial_fill,
            )
        }
        Config::MinDegreeSampleJW { weight } => {
            let sw = to_sample_weight(weight);
            eliminate_mindegree_sampling(
                &mut g,
                &sw,
                spec.seed,
                steps.sink(),
                ElimStop {
                    deadline: None,
                    ..spec.stop
                },
            )
        }
        Config::NestedDissMinCover => {
            let active: Vec<u32> = (0..g.len() as u32)
                .filter(|&v| g.active[v as usize])
                .collect();
            // Read neighbours via `collect_live_nbrs_into` (bitset-aware)
            // rather than `g.adj[v]` directly. Two ways `adj` lies here:
            // `preprocess` may have run bitset-mode eliminations that leave it
            // stale, and `clone_bitset_only` (the `single_component_bitset`
            // path) leaves it empty with only `bitset` describing the residual.
            // Reading `adj` produced wrong edges — in the second case zero
            // edges, hence an order over isolated vertices and singleton bags,
            // width 0 on any graph — and a malformed TD that leaks clause
            // vertices into the vtree (incidence graph).
            let mut nbrs_buf: Vec<u32> = Vec::new();
            let mut edges: Vec<(u32, u32)> = Vec::new();
            for &v in &active {
                nbrs_buf.clear();
                g.collect_live_nbrs_into(v, &mut nbrs_buf);
                for &u in &nbrs_buf {
                    if u > v && g.active[u as usize] {
                        edges.push((v, u));
                    }
                }
            }
            let order = nested_diss::nd_order(
                &active,
                &edges,
                &nested_diss::NdParams {
                    salt,
                    base_thresh: DEFAULT_BASE_THRESH,
                    max_imbalance: 0.2,
                    hard_deadline: spec.stop.hard_deadline,
                    base_seed: spec.seed,
                },
                0,
            );
            let mut sink = steps.sink();
            let mut check_counter = 0u32;
            let mut exit = ElimExit::Natural;
            for v in order {
                if !g.active[v as usize] {
                    continue;
                }
                check_counter += 1;
                if check_counter >= 64 {
                    check_counter = 0;
                    if expired(spec.stop.hard_deadline) {
                        exit = ElimExit::DeadlineBailed;
                        break;
                    }
                }
                nbrs_buf.clear();
                g.collect_live_nbrs_into(v, &mut nbrs_buf);
                let mut bag = Vec::with_capacity(nbrs_buf.len() + 1);
                bag.push(v);
                bag.extend_from_slice(&nbrs_buf);
                let bag_len = bag.len();
                g.eliminate_with_nbrs(v, &nbrs_buf);
                sink.record(v, bag);
                if exceeds_width_bound(bag_len, spec.stop.width_bound) {
                    // No emergency_path_decomp here — the caller drops this
                    // slot as WidthAborted.
                    exit = ElimExit::WidthAborted;
                    break;
                }
            }
            exit
        }
    };

    maybe_fill_emergency(spec.force_emit, exit, &g, &mut steps);
    (steps, exit)
}

/// Solve the preprocessed residual one connected component at a time, then
/// stitch all bags (prefix + per-component) into a single flat list and run
/// `build_td_from_steps`. Components are vertex-disjoint (guaranteed by
/// connectivity), so each vertex is eliminated exactly once and `global_rank`
/// is written without conflicts.
///
/// Key invariant: prefix bags use original vertex ids; per-component bags use
/// component-local ids that must be translated back to originals before
/// appending to `all_bags`.
fn run_config_per_component(
    kind: GraphKind,
    num_vars: u32,
    reduced: Reduced,
    components: Vec<Vec<u32>>,
    salt: &[u32],
    spec: RunSpec<'_>,
) -> ConfigRun {
    let n = reduced.graph.len();
    let mut all_bags: Vec<Vec<u32>> = reduced.prefix.bags;
    let mut global_rank: Vec<u32> = vec![u32::MAX; n];

    // Prefix ranks: (vertex, step) where step == bag index.
    for &(v, s) in &reduced.prefix.rank_pairs {
        global_rank[v as usize] = s as u32;
    }

    let mut combined_exit = ElimExit::Natural;
    let mut nbrs_buf: Vec<u32> = Vec::new();
    for (comp_idx, comp) in components.iter().enumerate() {
        let comp_n = comp.len() as u32;
        let mut local_of = vec![u32::MAX; n];
        for (i, &v) in comp.iter().enumerate() {
            local_of[v as usize] = i as u32;
        }
        // Extract component edges in local indexing (bitset-aware: adj may
        // be stale after preprocess).
        let mut comp_edges: Vec<(u32, u32)> = Vec::new();
        for &v in comp {
            nbrs_buf.clear();
            reduced.graph.collect_live_nbrs_into(v, &mut nbrs_buf);
            for &u in &nbrs_buf {
                if reduced.graph.active[u as usize] && local_of[u as usize] != u32::MAX && u > v {
                    comp_edges.push((local_of[v as usize], local_of[u as usize]));
                }
            }
        }

        let sub_graph = Graph::from_edges(comp_n, &comp_edges);
        let sub_reduced = preprocess(sub_graph);

        let sub_salt: Vec<u32> = comp.iter().map(|&v| salt[v as usize]).collect();
        // This component is re-indexed from 0, so a sampling core's weight is
        // re-indexed with it; a deterministic core has none to re-index.
        let sub_weight: Option<Vec<u32>> = spec
            .config
            .sat_weight()
            .map(|w| comp.iter().map(|&v| w[v as usize]).collect());
        // Vary seed per component so sampling configs get independent randomness.
        let sub_spec = RunSpec {
            seed: spec.seed.wrapping_add(comp_idx as u64 + 1),
            config: match sub_weight.as_deref() {
                Some(w) => spec.config.with_sat_weight(w),
                None => spec.config,
            },
            ..spec
        };

        let (comp_steps, comp_exit) = run_elimination_raw(sub_reduced, &sub_salt, None, sub_spec);
        // Any component going WidthAborted poisons the whole slot: the overall
        // TD would be missing that component's elimination entirely. Deadline
        // bails leave a valid (wide) component TD so they degrade rather than
        // invalidate.
        combined_exit = match (combined_exit, comp_exit) {
            (ElimExit::WidthAborted, _) | (_, ElimExit::WidthAborted) => ElimExit::WidthAborted,
            (ElimExit::DeadlineBailed, _) | (_, ElimExit::DeadlineBailed) => {
                ElimExit::DeadlineBailed
            }
            _ => ElimExit::Natural,
        };

        comp_steps.append_reindexed(comp, &mut all_bags, &mut global_rank);
    }

    ConfigRun {
        td: build_td_from_steps(kind, num_vars, all_bags, &global_rank),
        exit: combined_exit,
    }
}

pub(super) fn run_config_on_reduced(
    kind: GraphKind,
    num_vars: u32,
    reduced: Reduced,
    single_component_bitset: bool,
    initial_fill: Option<&[u64]>,
    spec: RunSpec<'_>,
) -> ConfigRun {
    let n = reduced.graph.len();
    // `+ SEED_OFFSET` avoids xorshift64's zero fixed point.
    let mut rng = Xorshift64::from_state(spec.seed.wrapping_add(SEED_OFFSET));
    let salt: Vec<u32> = (0..n).map(|_| rng.next_u32()).collect();

    // Solve each connected component independently. Components arise
    // naturally after preprocessing removes low-degree vertices.
    // Skip BFS when caller already confirmed single-component (bitset mode:
    // adj is empty in clone_bitset_only, so find_connected_components would
    // produce n singletons — the BFS reads adj which is emptied to save allocs).
    if !single_component_bitset {
        let components = find_connected_components(&reduced.graph);
        if components.len() > 1 {
            return run_config_per_component(kind, num_vars, reduced, components, &salt, spec);
        }
    }

    // Only the whole-residual path checks this: the per-component path
    // re-indexes each component and remaps the weight alongside, so its
    // lengths are its own business.
    if let Some(w) = spec.config.sat_weight() {
        assert_eq!(w.len(), n, "sat_weight length must match total_vertices");
    }

    let (steps, exit) = run_elimination_raw(reduced, &salt, initial_fill, spec);
    finalize(kind, num_vars, steps, n, exit)
}

/// If the caller needs a guaranteed-valid TD and the elimination bailed on the
/// hard deadline, append an `emergency_path_decomp` chain over the residual.
/// This preserves whatever progress the elim heuristic made (so width ≤
/// max(partial_width, remaining_active − 1)) rather than forcing width =
/// num_vars-1.
/// The main schedule loop only sets `force_emit = true` while no slot has
/// produced a TD yet — so at most one emergency fill runs per schedule.
fn maybe_fill_emergency(force_emit: bool, exit: ElimExit, g: &Graph, steps: &mut ElimSteps) {
    if force_emit && exit == ElimExit::DeadlineBailed {
        minfill_core::emergency_path_decomp(g, &mut steps.sink());
    }
}

fn finalize(
    kind: GraphKind,
    num_vars: u32,
    steps: ElimSteps,
    n: usize,
    exit: ElimExit,
) -> ConfigRun {
    let mut rank = vec![u32::MAX; n];
    for (v, r) in steps.rank_pairs {
        rank[v as usize] = r as u32;
    }
    ConfigRun {
        td: build_td_from_steps(kind, num_vars, steps.bags, &rank),
        exit,
    }
}

/// Max residual size for which `nd_order` (via `multilevel_bisect`) can
/// reasonably complete within a per-config slot of the hard-deadline budget.
/// Exported so the schedule driver can recognize a large residual and skip
/// the slots that would overshoot it — every non-min-degree slot after the
/// first — instead of spending the budget on an elimination that will not
/// finish.
pub(super) const NESTED_DISS_MAX_ACTIVE: usize = 10_000;

/// Invert argmin-convention weight (smaller = eliminated first) into sample-
/// convention weight (higher = more likely to sample). Direction
/// remains baked in by `sat_score::compute_weight`; this just flips the ordering
/// so a `P(v) ∝ w+1` sampler eliminates the low-JW vertices first, keeping the
/// high-JW ones alive for the top of the vtree.
fn to_sample_weight(argmin_weight: &[u32]) -> Vec<u32> {
    argmin_weight.iter().map(|&w| u32::MAX - w).collect()
}

/// The schedule a default run executes: five slots, run through the refined
/// vtree entry points (`--vtree portfolio`'s goatd candidate, and
/// the same `goatd` spec named directly).
///
/// Ordered by ascending per-step cost so cheap algorithms run first and give
/// the best chance for multiple configs to fit inside the wall-clock budget:
///   slot 0: MinDegreeSampleJW — htd-style min-degree: degree-only priority
///                                with JW-weighted random sampling from the
///                                tie set, O(deg·log n) per step
///   slot 1: NestedDiss        — O(n log n) hierarchical, fast on dense graphs
///   slot 2: MinFillSampleJW   — htd-style min-fill: fill-only priority with
///                                JW-weighted random sampling from the tie set
///   slot 3: MinDegreeSampleJW+42 — second seed for diversity
///   slot 4: NestedDiss+42     — second seed for diversity
///
/// Sampling (rather than deterministic `(secondary, salt)` tie-breaking) lets
/// the `+42` seed produce a genuinely different elimination, not just a
/// re-shuffled tie-break. Slot 0 is exempt from every between-slot skip in
/// the budget-aware runner, which is what guarantees the schedule always
/// returns a TD even when the wall-clock deadline elapses early.
pub(super) fn td_bench_schedule(base_seed: u64, weight: &[u32]) -> Vec<(Config<'_>, u64)> {
    vec![
        (Config::MinDegreeSampleJW { weight }, base_seed),
        (Config::NestedDissMinCover, base_seed),
        (Config::MinFillSampleJW { weight }, base_seed),
        (
            Config::MinDegreeSampleJW { weight },
            base_seed.wrapping_add(42),
        ),
        (Config::NestedDissMinCover, base_seed.wrapping_add(42)),
    ]
}

/// The `goatd-primal` schedule: a single `MinFillSampleJW` slot. Reachable
/// only by asking for that spec by name — no default run executes it.
/// Refinement samples (`COMPILE_MAX_REFINE_SLOTS`) provide the diversity the
/// 5-slot default schedule gets from its slots. Measured on the development
/// corpus as strictly better than a 5-slot / 200-sample / quality-selector
/// configuration on every axis: more instances decomposed well, smaller
/// realized diagrams, and a faster build.
pub(super) fn compile_schedule(base_seed: u64, weight: &[u32]) -> Vec<(Config<'_>, u64)> {
    // The schedule deadline is 1 s soft / 2 s hard, so adding a second slot
    // eats roughly half the post-schedule refinement budget.
    vec![(Config::MinFillSampleJW { weight }, base_seed)]
}
