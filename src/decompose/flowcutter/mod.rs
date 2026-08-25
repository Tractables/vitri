//! In-process FlowCutter backend: tree-decomposition construction
//! (`td_compute*`) through the vendored C++ library at `vendor/treedecomp/`,
//! and the conversion of what it returns into a vtree.
//!
//! The other half of FlowCutter — one top-level balanced separator, with no
//! decomposition around it — is the pure-Rust port in
//! [`super::flowcutter_rs`]; nothing here crosses into it.
//!
//! **Determinism:** with `timeout_ms = 0` and `patience_ms = 0` this path is
//! deterministic for a given graph (no wall-clock dependence).
//! Wall-clock-based termination (nonzero `timeout_ms` or `patience_ms`)
//! introduces non-determinism because different runs may complete different
//! numbers of iterations before the deadline. A [`WallCapMode::BoundOnly`] cap
//! is the exception: it changes nothing about the search, so a build that
//! finishes well inside such a cap is as deterministic as the untimed one.
//!
//! # Safety
//!
//! Every `unsafe` call below goes into the vendored builder, so they share one
//! argument; the per-site comments add only what is specific to a call.
//!
//! * **The handle.** [`TdHandle`] wraps what one of the `td_compute*` entry
//!   points returned, and [`TdHandle::compute`] is the only way to build one —
//!   it rejects a null before wrapping, so every readback has a live result. It
//!   is released exactly once, in `Drop`; the type is neither `Copy` nor
//!   `Clone`, so no second owner can free it or read through it afterwards, and
//!   its raw pointer field keeps the type off `Send`/`Sync`, so the handle
//!   stays on the thread that built it.
//! * **The edge buffer** is described by a pointer and a count taken from the
//!   same live slice in the same expression, and holds exactly two `c_int`s per
//!   edge. The C side reads `edges[2*i]` and `edges[2*i+1]` for `i` below the
//!   count it is given, so every read stays inside the buffer — a count that
//!   did not fit in the `c_int` parameter can only ask for fewer edges than are
//!   there, never more. The builder copies what it needs during the call and
//!   keeps no pointer to it, so nothing has to outlive the call.
//! * **Buffers read back** are allocated with exactly the length the matching
//!   size query returned an instant earlier, which is what the C API asks of
//!   the caller, and every bag index handed to a readback is below the bag
//!   count the same handle just reported. Nothing in between can change either,
//!   because the readback calls only borrow the decomposition. The counts are
//!   C++ container sizes, so they are non-negative and the casts preserve them.

use std::os::raw::c_int;
use std::sync::Arc;

use crate::cnf::CnfFormula;
use crate::score::{BUILT_FROM_THIS_FORMULA, vtree_cost};
use crate::vtree::Vtree;

use super::best::BestBy;
use super::*;

mod treedecomp_ffi {
    use std::os::raw::c_int;

    #[repr(C)]
    pub(super) struct TdResult {
        _private: [u8; 0],
    }

    // SAFETY: hand-written mirror of the C declarations in
    // `vendor/treedecomp/ffi.h`, which is vendored here and compiled by
    // `build.rs` from that same header — the two sides move together. The
    // `td_compute*` entry points hand back ownership of the result, which the
    // caller returns through `td_free`; the readback calls borrow it and copy
    // into caller-provided buffers, so nothing they return has to be freed.
    // The header's `int` is `c_int` here; its `int64_t` is `i64`, which is that
    // type on every target rather than a C spelling of it.
    unsafe extern "C" {
        pub(super) fn td_compute(
            num_nodes: c_int,
            num_edges: c_int,
            edges: *const c_int,
            steps: i64,
            iters: c_int,
        ) -> *mut TdResult;
        pub(super) fn td_compute_timed_patience(
            num_nodes: c_int,
            num_edges: c_int,
            edges: *const c_int,
            steps: i64,
            iters: c_int,
            timeout_ms: i64,
            patience_ms: i64,
            tight_gates: c_int,
        ) -> *mut TdResult;
        pub(super) fn td_num_bags(td: *const TdResult) -> c_int;
        pub(super) fn td_bag_size(td: *const TdResult, bag_idx: c_int) -> c_int;
        pub(super) fn td_bag_vertices(td: *const TdResult, bag_idx: c_int, out: *mut c_int);
        pub(super) fn td_bag_num_neighbors(td: *const TdResult, bag_idx: c_int) -> c_int;
        pub(super) fn td_bag_neighbors(td: *const TdResult, bag_idx: c_int, out: *mut c_int);
        pub(super) fn td_free(td: *mut TdResult);
    }
}

/// The vendored builder's result: the one foreign resource this crate owns,
/// and the only thing here that has to be released by hand.
///
/// A handle exists only if the builder produced one (§ Safety), so nothing
/// downstream re-checks for a null, and it is freed by going out of scope
/// rather than at a `td_free` some path can miss.
struct TdHandle(*mut treedecomp_ffi::TdResult);

impl Drop for TdHandle {
    fn drop(&mut self) {
        // SAFETY: live handle (§ Safety), freed exactly here.
        unsafe { treedecomp_ffi::td_free(self.0) };
    }
}

impl TdHandle {
    /// Decompose the graph `flat_edges` describes over `num_vertices` vertices,
    /// under `budget`. `None` when the builder produced no result.
    ///
    /// `flat_edges` holds each edge as two consecutive endpoints.
    fn compute(num_vertices: u32, flat_edges: &[c_int], budget: FcBudget) -> Option<Self> {
        let num_edges = flat_edges.len() / 2;
        let raw = match budget {
            FcBudget::Timed {
                timeout_ms,
                patience_ms,
                iters,
                steps,
                cap_mode,
            } => {
                let steps = match cap_mode {
                    WallCapMode::Tight => steps,
                    WallCapMode::BoundOnly => scaled_steps(steps, num_edges),
                };
                // SAFETY: edge buffer (§ Safety); the returned handle becomes ours.
                unsafe {
                    treedecomp_ffi::td_compute_timed_patience(
                        num_vertices as c_int,
                        num_edges as c_int,
                        flat_edges.as_ptr(),
                        steps,
                        iters,
                        timeout_ms,
                        patience_ms,
                        cap_mode.as_ffi(),
                    )
                }
            }
            FcBudget::Steps { steps, iters } => {
                // SAFETY: as the timed call above (§ Safety).
                unsafe {
                    treedecomp_ffi::td_compute(
                        num_vertices as c_int,
                        num_edges as c_int,
                        flat_edges.as_ptr(),
                        scaled_steps(steps, num_edges),
                        iters,
                    )
                }
            }
        };
        (!raw.is_null()).then_some(TdHandle(raw))
    }
}

/// The step budget scaled to graph size: small graphs converge quickly and do
/// not need 1M+ steps.
///
/// Dropped only under a [`WallCapMode::Tight`] wall, where the clock stops the
/// search long before the ceiling matters. A bound-only wall keeps the clamp,
/// so arming such a wall leaves the step budget the untimed one.
fn scaled_steps(steps: i64, num_edges: usize) -> i64 {
    steps.min(10_000i64.max(50 * num_edges as i64))
}

/// What a non-zero FlowCutter wall cap MEANS, which is orthogonal to how large
/// it is.
///
/// The vendored library's timed entry is not a superset of its step-budgeted
/// one: under a deadline it also tightens the pre-loop heuristic node gates
/// (min-degree 50 000 → 2 000, min-shortcut 10 000 → 1 000) and drops the step
/// clamp. Those land whether or not the cap is ever reached, so keying them on
/// the mere presence of a deadline makes every deadline-armed build search less
/// patiently, in service of bounding the few that overrun.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WallCapMode {
    /// The cap is an outer bound the caller does not expect to reach. The search
    /// keeps its untimed gates and step clamp, so the decompositions it
    /// considers are exactly the unbounded ones and the deadline only stops it
    /// once the wall has genuinely passed.
    BoundOnly,
    /// The cap is expected to bite: a short window in which finishing matters
    /// more than searching. The pre-loop heuristics take their tight gates,
    /// because a single ordering pass on a large graph can consume the whole
    /// window.
    Tight,
}

impl WallCapMode {
    /// FFI encoding: nonzero = tight.
    fn as_ffi(self) -> c_int {
        match self {
            WallCapMode::BoundOnly => 0,
            WallCapMode::Tight => 1,
        }
    }
}

/// How hard one FlowCutter run looks for a decomposition — one variant per mode
/// the search has, so a caller cannot name a wall clock and a step budget that
/// contradict each other.
#[derive(Clone, Copy)]
pub(crate) enum FcBudget {
    /// Stop on the wall clock.
    Timed {
        /// Wall-clock limit in milliseconds.
        timeout_ms: i64,
        /// Milliseconds without a treewidth improvement before giving up early;
        /// `0` waits out the whole clock.
        patience_ms: i64,
        /// Source/sink pairs tried per level.
        iters: i32,
        /// Ceiling on the search the clock is meant to reach first:
        /// [`FC_TIMED_STEPS`] for a run whose only limit is time, and the count
        /// it was going to spend anyway for a caller capping a run mid-build.
        steps: i64,
        /// Whether `timeout_ms` is expected to bite. See [`WallCapMode`].
        cap_mode: WallCapMode,
    },
    /// Stop after a fixed number of computation steps, which is what makes the
    /// run deterministic (see the module docs).
    Steps {
        /// Computation-step budget for the search, scaled down to the graph in
        /// [`TdHandle::compute`].
        steps: i64,
        /// Source/sink pairs tried per level.
        iters: i32,
    },
}

/// The step ceiling a purely timed run carries — high enough that the wall
/// clock is what ends the search.
const FC_TIMED_STEPS: i64 = 1_000_000;

/// The wall clock a bare `flowcutter-*` spec means.
pub(crate) const FC_BARE_TIMEOUT_MS: i64 = 200;
/// Patience for a bare `flowcutter-*` spec, whose clock is short enough that it
/// gives up on a stalled search sooner than a spec that names its own timeout.
pub(crate) const FC_PATIENCE_MS_BARE: i64 = 100;
/// Patience for a spec that names a timeout but no patience.
pub(crate) const FC_PATIENCE_MS_PARAMETRIZED: i64 = 150;
/// Source/sink pairs a timed spec tries per level when it names no count.
pub(crate) const FC_DEFAULT_ITERS: i32 = 100_000;
/// Source/sink pairs a step-budgeted spec tries per level when it names no
/// count — far below the timed default, because nothing stops the search early.
pub(crate) const FC_DEFAULT_STEPS_ITERS: i32 = 900;

impl FcBudget {
    /// The budget a timed spec asks for: the clock decides, so the step ceiling
    /// is the one it will not reach.
    ///
    /// A timeout that is not positive is no clock at all, and what it names is
    /// the step-budgeted search — the tree that spelling has always built.
    ///
    /// The clock this constructor is given is the one the caller expects to run
    /// out, so it is a [`WallCapMode::Tight`] one. A caller arming a wall it
    /// does not expect to reach builds [`FcBudget::Timed`] directly and says so.
    pub(crate) const fn timed(timeout_ms: i64, patience_ms: i64, iters: i32) -> Self {
        if timeout_ms > 0 {
            FcBudget::Timed {
                timeout_ms,
                patience_ms,
                iters,
                steps: FC_TIMED_STEPS,
                cap_mode: WallCapMode::Tight,
            }
        } else {
            FcBudget::Steps {
                steps: FC_TIMED_STEPS,
                iters,
            }
        }
    }
}

/// What turns the decomposition FlowCutter found into the vtree returned.
///
/// The decomposition is the same whichever of these is asked for; they differ
/// only in how the tree is read off it, which is why they are an argument to one
/// entry point rather than four entry points.
pub(crate) enum Conversion<'a> {
    /// Rank the conversion's own candidates and keep the best-scoring vtree.
    Best,
    /// One conversion, under exactly this configuration.
    Configured(&'a TdToVtreeConfig),
    /// Two item orderings from the one decomposition, lower cost score wins.
    DualOrdering,
    /// The hybrid TD + bisection assembly over the decomposition.
    Hybrid,
}

/// THE FlowCutter construction: decompose one graph view of `formula` at
/// `budget`, then read a vtree off the result the way `conversion` says.
///
/// Every `--vtree` spec in the FlowCutter family lands here. The spec grammar
/// varies exactly three things — which graph, how hard to look, how to convert —
/// so those are the three arguments, and there is nothing left for a per-spec
/// wrapper to encode in its name. `effort_scale` is the fourth thing a run
/// carries, and it belongs to the conversion rather than to the search: it is
/// how wide a sweep the run's wall-clock hint pays for
/// ([`crate::budget::vtree_effort_scale`]), where `budget` bounds the hunt for
/// the decomposition itself.
pub(crate) fn flowcutter_vtree(
    formula: &CnfFormula,
    kind: GraphKind,
    budget: FcBudget,
    conversion: Conversion<'_>,
    effort_scale: f64,
) -> Result<TdConversion, String> {
    let td = flowcutter_td(formula, kind, budget)?;
    match conversion {
        Conversion::Best => Ok(built_from_td_best(formula, &td, effort_scale, None)),
        Conversion::Configured(config) => Ok(built_from_td(formula, &td, config, effort_scale)),
        Conversion::DualOrdering => Ok(dual_ordering_from_td(formula, &td, effort_scale)),
        Conversion::Hybrid => super::hybrid::hybrid_from_incidence_td(formula, &td, effort_scale),
    }
}

/// Convert a decomposition this module (or a caller holding one already) has in
/// hand into the [`TdConversion`] a construction returns: one traced conversion
/// under `config`, carrying the winner's bag metadata.
///
/// THE one place that pairing is written, so every construction below returns
/// its conversion and its metadata together and none can forget the second.
pub(super) fn built_from_td(
    formula: &CnfFormula,
    td: &TreeDecomposition,
    config: &TdToVtreeConfig,
    effort_scale: f64,
) -> TdConversion {
    let (vtree, td_info) = td_to_vtree_configured_traced(
        ConversionInput {
            td,
            num_vars: formula.num_vars,
            formula: Some(formula),
            effort_scale,
        },
        config,
    );
    TdConversion {
        vtree: Arc::new(vtree),
        td: td_info,
    }
}

/// The same pairing under the conversion that tries both orderings and keeps
/// the cheaper ([`td_to_vtree_best_traced`]) — what a backend converts with when
/// it has no configuration of its own to impose, which is most of them.
///
/// `deadline` bounds how much of the sweep runs, never whether it returns a
/// tree; a backend with no deadline in hand passes `None`.
pub(super) fn built_from_td_best(
    formula: &CnfFormula,
    td: &TreeDecomposition,
    effort_scale: f64,
    deadline: Option<std::time::Instant>,
) -> TdConversion {
    let (vtree, td_info) =
        td_to_vtree_best_traced(td, formula.num_vars, formula, effort_scale, deadline);
    TdConversion {
        vtree: Arc::new(vtree),
        td: td_info,
    }
}

/// Refuse a graph the vendored builder cannot be handed at all.
///
/// `TWD::Graph::init` allocates an n×n bitset adjacency matrix, and the vendored
/// `Bitset` constructor does not check its allocation, so an oversized graph is
/// a SIGSEGV inside the FFI rather than an error anything here could return.
/// Checked before the edge list is built, so a formula that cannot be
/// decomposed this way costs nothing to reject.
fn vendor_size_guard(kind: GraphKind, formula: &CnfFormula) -> Result<(), String> {
    match kind {
        GraphKind::Primal => {
            // At 1.1M variables the matrix is ~155 GB, which exhausts the
            // address space. 500K matches PORTFOLIO_HEAVY_MAX_VARS in
            // portfolio.rs; the matrix at n=500K is 29 GiB, fitting under the
            // 32 GiB address-space limit.
            const MAX_PRIMAL_VERTICES: u32 = 500_000;
            let num_vars = formula.num_vars;
            if num_vars > MAX_PRIMAL_VERTICES {
                return Err(format!(
                    "primal graph too large ({} vertices; n×n adjacency matrix would exceed memory)",
                    num_vars
                ));
            }

            // Even when vertex count fits, the primal graph can be too dense: a single
            // 4258-wide clause contributes C(4258,2) ≈ 9 M raw pair-edges, and a
            // formula with millions of such clauses runs to hundreds of millions.
            // The C++ td_compute aborts via std::bad_alloc on inputs this dense and
            // the exception propagates through `extern "C"` as `terminate()`, killing
            // the whole process before flowcutter-primal's `Err` can be returned. Estimate the
            // raw pair count (without materialising it) and bail before we allocate
            // the edge list. 20 M is conservative — flowcutter-primal use in portfolio today
            // runs on ≤ 30k-vertex / ≤ 100k-clause sub-components where the raw count
            // is < 1 M.
            const MAX_PRIMAL_RAW_EDGES: u64 = 20_000_000;
            let raw_edges: u64 = formula
                .clauses
                .iter()
                .map(|c| {
                    let w = c.literals.len() as u64;
                    w.saturating_mul(w.saturating_sub(1)) / 2
                })
                .sum();
            if raw_edges > MAX_PRIMAL_RAW_EDGES {
                return Err(format!(
                    "primal graph too dense (~{raw_edges} raw pair-edges; C++ td_compute would exceed memory budget)"
                ));
            }
            Ok(())
        }
        GraphKind::Incidence => {
            // A large incidence graph (666k nodes → 55 TB) overflows RLIMIT_AS.
            const MAX_INCIDENCE_VERTICES: u32 = 100_000;
            let total_vertices = formula.num_vars + formula.clauses.len() as u32;
            if total_vertices > MAX_INCIDENCE_VERTICES {
                return Err(format!(
                    "incidence graph too large ({} vertices; n×n adjacency matrix would exceed memory)",
                    total_vertices
                ));
            }
            Ok(())
        }
    }
}

/// Run the vendored C++ FlowCutter over one graph view of `formula` and return
/// the raw decomposition.
///
/// The single place this crate calls into `vendor/treedecomp/`: both graph views
/// come through here, and everything that distinguishes them is settled by
/// `kind` before the call. Replacing the vendored builder with a port therefore
/// means replacing one function body, not auditing every backend that wanted a
/// decomposition.
///
/// Bags come back over `formula`'s variables in both views: an incidence run
/// decomposes clause vertices too, but the extracted decomposition is labelled
/// with `formula.num_vars`, exactly as the primal one is.
pub(super) fn flowcutter_td(
    formula: &CnfFormula,
    kind: GraphKind,
    budget: FcBudget,
) -> Result<TreeDecomposition, String> {
    vendor_size_guard(kind, formula)?;
    let PaceGraph {
        num_vertices: total_vertices,
        edges,
        ..
    } = kind.build(formula);

    let mut flat_edges: Vec<c_int> = Vec::with_capacity(edges.len() * 2);
    for &(u, v) in &edges {
        flat_edges.push(u as c_int);
        flat_edges.push(v as c_int);
    }

    let td = TdHandle::compute(total_vertices, &flat_edges, budget)
        .ok_or_else(|| "td_compute returned null".to_string())?;
    extract_td(&td, kind, formula.num_vars)
}

/// Copy a result out of the vendored builder into the crate's own type,
/// labelled with the view it decomposed and over `num_vars` variables.
fn extract_td(td: &TdHandle, kind: GraphKind, num_vars: u32) -> Result<TreeDecomposition, String> {
    use treedecomp_ffi::*;

    // SAFETY: live handle (§ Safety), read back under the probe-then-fill
    // protocol described there; every call below only borrows it.
    unsafe {
        let n = td_num_bags(td.0) as usize;
        if n == 0 {
            return Err("empty tree decomposition".into());
        }

        let mut bags: Vec<TdBag> = Vec::with_capacity(n);
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];

        for i in 0..n {
            let bag_sz = td_bag_size(td.0, i as c_int) as usize;
            let mut vertices = vec![0 as c_int; bag_sz];
            td_bag_vertices(td.0, i as c_int, vertices.as_mut_ptr());
            bags.push(TdBag {
                id: i,
                vertices: vertices.iter().map(|&v| v as u32).collect(),
            });

            let num_nb = td_bag_num_neighbors(td.0, i as c_int) as usize;
            let mut neighbors = vec![0 as c_int; num_nb];
            td_bag_neighbors(td.0, i as c_int, neighbors.as_mut_ptr());
            for &nb in &neighbors {
                let nb = nb as usize;
                if nb > i {
                    adj[i].push(nb);
                    adj[nb].push(i);
                }
            }
        }

        Ok(TreeDecomposition {
            kind,
            bags,
            adj,
            num_vars,
        })
    }
}

/// Try two TD→vtree orderings over one decomposition and keep the one with the
/// better cost score.
///
/// Orderings: ChildrenFirst (the original default, good for most benchmarks)
/// and ChildrenBySize. Both use FirstBag root and Deepest bag assignment.
/// Overhead: one extra O(clauses × depth) vtree construction + scoring.
fn dual_ordering_from_td(
    formula: &CnfFormula,
    td: &TreeDecomposition,
    effort_scale: f64,
) -> TdConversion {
    let num_vars = formula.num_vars;

    let configs = [
        TdToVtreeConfig {
            item_ordering: ItemOrdering::ChildrenFirst,
            ..Default::default()
        },
        TdToVtreeConfig {
            item_ordering: ItemOrdering::ChildrenBySize,
            ..Default::default()
        },
    ];

    // The metadata travels WITH its own conversion, so the winner carries its
    // own bag assignment and the runner-up's is dropped with the tree it
    // described.
    let mut best: BestBy<(Vtree, super::TdConversionMeta), f64> = BestBy::new();
    let cost = |v: &Vtree| vtree_cost(v, formula).expect(BUILT_FROM_THIS_FORMULA);
    for config in &configs {
        let built = td_to_vtree_configured_traced(
            ConversionInput {
                td,
                num_vars,
                formula: Some(formula),
                effort_scale,
            },
            config,
        );
        let score = cost(&built.0);
        best.offer(built, score);
    }

    let ((vtree, td_info), _) = best.into_best().expect("the config list is not empty");
    TdConversion {
        vtree: Arc::new(vtree),
        td: td_info,
    }
}

#[cfg(test)]
mod tests;
