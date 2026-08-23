//! The goatd schedule executor: run a schedule of elimination configs under a
//! wall-clock budget, keep per-slot outcomes, and pick a winner.
//!
//! The slots themselves come from [`super::width_opt`]; the constructions the
//! `--vtree` vocabulary names one at a time live in [`super::elimination`],
//! which this module does not call.

use std::time::{Duration, Instant};

use crate::budget::expired;
use crate::cnf::CnfFormula;
use crate::score::{BUILT_FROM_THIS_FORMULA, vtree_cost};

use super::super::best::select_first_min;
use super::super::flowcutter::built_from_td_best;
use super::super::{GraphKind, PaceGraph};
use super::super::{TdConversion, TreeDecomposition};
use super::minfill_core::{ElimExit, ElimStop};
use super::{refine, sat_score, width_opt};

/// Build a vtree with goatd on the primal graph, trying every default
/// `(config, seed)` pair and picking the result with the best cost score.
pub(crate) fn vtree_from_goatd_best(
    formula: &CnfFormula,
    seed: u64,
    effort_scale: f64,
) -> Result<TdConversion, String> {
    best_vtree_over_schedule(
        formula,
        GraphKind::Primal,
        seed,
        ModeConfig::compile(),
        effort_scale,
    )
}

/// What the goatd schedule is configured with, beyond the caller's budget.
///
/// One value rather than a loose parameter, so the portfolio's goatd candidate
/// and the `goatd` spec that names the same construction cannot end up
/// configured differently.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GoatdKnobs {
    /// Explicit budget in milliseconds for the refine schedule, overriding the
    /// share of the construction budget the portfolio would otherwise give it.
    /// `None` (the default) takes that share; with no budget anywhere the
    /// schedule runs to completion.
    pub refine_budget_ms: Option<u64>,
}

impl GoatdKnobs {
    /// Fill the knobs from the `VITRI_*` process environment: a variable that is
    /// set overrides the knob it names, an unset one leaves the caller's value.
    ///
    /// # Errors
    ///
    /// [`VitriError`](crate::error::VitriError) naming the offending variable
    /// and the form it expects.
    pub(in crate::decompose) fn with_env_defaults(self) -> Result<Self, crate::error::VitriError> {
        Ok(GoatdKnobs {
            refine_budget_ms: refine_budget_ms(
                crate::env::env_raw("VITRI_GOATD_REFINE_BUDGET_MS", REFINE_BUDGET_FORM)?.as_deref(),
            )?
            .or(self.refine_budget_ms),
        })
    }
}

/// Incidence-graph variant of the refined-best vtree entry point.
///
/// `budget_ms` is the caller's wall-clock ceiling for this one goatd build (the
/// portfolio's fair share of the vtree-construction budget). `None` = run to
/// completion; every non-portfolio caller passes it.
pub(crate) fn vtree_from_goatd_incidence_refined_best(
    formula: &CnfFormula,
    seed: u64,
    budget_ms: Option<u64>,
    knobs: GoatdKnobs,
    effort_scale: f64,
) -> Result<TdConversion, String> {
    refined_best_vtree_over_schedule(
        formula,
        GraphKind::Incidence,
        seed,
        budget_ms,
        knobs,
        effort_scale,
    )
}

/// Shared body for the refined-best entry points: runs the goatd schedule,
/// picks the best TD by `(width, total_bag_size)`, applies the FlowCutter-cut
/// refinement, then converts to a vtree.
///
/// `budget_ms = None` runs to completion (compilation mode). `Some(n)` bounds
/// both: the schedule gets an `n`-ms soft / `2n`-ms hard deadline, and
/// refinement gets the same absolute deadline, which also arms `refine`'s
/// large-graph gate — an incidence graph with over 100k vertices, where one
/// uninterruptible FlowCutter iteration can run for seconds, is skipped
/// rather than entered. Both are anytime: the lex-min picker keeps the best
/// TD found so far, and a skipped refinement returns the TD unchanged, so a
/// bounded build still yields a vtree.
///
/// Runs no per-slot refinement — only a post-process FC pass on the winner.
fn refined_best_vtree_over_schedule(
    formula: &CnfFormula,
    view: GraphKind,
    seed: u64,
    budget_ms: Option<u64>,
    knobs: GoatdKnobs,
    effort_scale: f64,
) -> Result<TdConversion, String> {
    // `goatd_schedule_tds` sorts by `refined_select_key` with
    // first-occurrence-wins ties, so `[0]` is the minimum; only it gets
    // FC-refined here. `GraphKind::build` is deterministic, so rebuilding it below
    // is safe. The refinement deadline is absolute and measured from here, so
    // a schedule that already spent its whole budget leaves refinement no
    // time instead of doubling the goatd candidate's cost.
    let refine_deadline = budget_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
    let PaceGraph {
        num_vertices: total_vertices,
        edges,
        ..
    } = view.build(formula);
    let (td, _width, _tbs) = goatd_schedule_tds(formula, view, seed, budget_ms, knobs)
        .into_iter()
        .next()
        .expect("run_schedule's slot 0 always produces a TD");

    let all_vars: Vec<u32> = (0..total_vertices).collect();
    let refined = refine::refine_td_with_flowcutter_cut(td, &all_vars, &edges, refine_deadline);
    Ok(built_from_td_best(formula, &refined, effort_scale))
}

/// All goatd schedule TDs from `td_bench_schedule`, unrefined, each paired
/// with its `(width, total_bag_size)`, sorted ascending by
/// `refined_select_key` with first-occurrence-wins ties (stable sort).
fn goatd_schedule_tds(
    formula: &CnfFormula,
    view: GraphKind,
    seed: u64,
    budget_ms: Option<u64>,
    knobs: GoatdKnobs,
) -> Vec<(TreeDecomposition, u32, usize)> {
    let PaceGraph {
        num_vertices: total_vertices,
        edges,
        ..
    } = view.build(formula);
    let results = run_schedule(
        formula,
        view,
        &edges,
        total_vertices,
        seed,
        width_opt::td_bench_schedule,
        ModeConfig::compile_refined(budget_ms, knobs),
    );
    let mut tds: Vec<(TreeDecomposition, u32, usize)> = results
        .into_iter()
        .filter_map(|r| match r {
            SlotResult::Produced {
                td,
                width,
                total_bag_size,
            } => Some((td, width, total_bag_size)),
            SlotResult::Empty => None,
        })
        .collect();
    // Stable sort by the refined-path key ⇒ `[0]` is the first-occurrence min,
    // matching `select_first_min` semantics exactly.
    tds.sort_by_key(|(_, w, tbs)| refined_select_key(*w, *tbs));
    tds
}

const FC_SLOT_MAX_VERTICES: u32 = 100_000;
/// Cap on the FC slot's wall-clock budget. Picked to keep overhead bounded
/// within the `[soft_deadline, hard_deadline]` margin (hard_deadline = 2×soft)
/// without starving any downstream work.
const FC_SLOT_CAP_MS: i64 = 2_000;
/// Exit early if FlowCutter hasn't improved treewidth for this long. Caps
/// per-CNF overhead where FC converges fast.
const FC_SLOT_PATIENCE_MS: i64 = 500;

/// Per-slot soft cap on `Config::MinFill` (eager fill-count recompute is
/// 10–20× more expensive per step than the lazy variants). Applied
/// unconditionally so compilation mode (no schedule deadline) still bounds
/// MinFill at 1 s.
const MINFILL_SLOT_MAX_MS: u64 = 1_000;

/// Cap on refinement-phase seeds when there is no deadline. 100 is the knee
/// measured across benchmark CNFs: fewer leaves quality on the table, more
/// costs construction time without improving the vtree.
///
/// Used as-is regardless of the budget: refine slots deliberately do not scale
/// with construction effort — scaling them measured as dead weight, no
/// quality gain for a materially longer build.
const COMPILE_MAX_REFINE_SLOTS: u64 = 100;

/// Consolidated per-caller config for `run_schedule`. Every field that differs
/// between the shipped single-slot caller and the TD-quality benchmark caller
/// lives here, typed — so a new caller must make an explicit choice and silent
/// cross-mode inheritance is structurally impossible.
#[derive(Clone, Copy)]
pub(crate) struct ModeConfig {
    /// Soft schedule deadline. `None` = compilation mode (no between-slot
    /// deadline). `Some(N)` = bench mode: soft at `start+N`, hard at `start+2N`.
    pub(crate) timeout_ms: Option<u64>,
    /// Cap on refinement-phase sampling seeds. 100 on the shipped path — the
    /// measured knee.
    refine_cap: u64,
    /// Vanilla-FC trailing schedule slot wall-clock cap. `None` = slot
    /// disabled. `Some(cap_ms)` = slot runs up to
    /// `min(remaining_hard_budget, cap_ms)` ms. The slot is primal-only
    /// (incidence A/B was net-negative), so a factory whose path runs on the
    /// incidence graph says `None` rather than carrying a cap its own graph
    /// view can never spend.
    fc_slot_cap_ms: Option<i64>,
}

impl ModeConfig {
    /// `goatd-primal`'s config: 1-slot schedule (`compile_schedule`), 1 s soft
    /// / 2 s hard deadline, 100 refine samples, no per-slot FC refinement,
    /// vanilla-FC trailing slot enabled (this path is primal).
    fn compile() -> Self {
        Self {
            timeout_ms: Some(COMPILE_TIMEOUT_MS),
            refine_cap: COMPILE_MAX_REFINE_SLOTS,
            fc_slot_cap_ms: Some(FC_SLOT_CAP_MS),
        }
    }

    /// The refined goatd schedule — 5 slots (`td_bench_schedule`) with
    /// post-process FC refinement on the winner. No per-slot FC refinement
    /// (regressed on primal).
    ///
    /// Runs up to `COMPILE_MAX_REFINE_SLOTS` (100) full eliminations with no
    /// between-slot deadline by default — unbounded on a large graph. The
    /// schedule deadline is resolved here from two inputs: (1)
    /// [`GoatdKnobs::refine_budget_ms`] wins if set, else (2)
    /// `caller_budget_ms`. The result is a soft N-ms / hard 2N-ms deadline: a
    /// cheap formula still runs the full schedule (deadline never trips,
    /// output byte-identical) while an expensive one bails early instead of
    /// grinding all 100 samples. Both inputs absent ⇒ `None` ⇒ run to
    /// completion, unchanged.
    ///
    /// No vanilla-FC slot — this schedule runs on the incidence graph, where
    /// the slot is disabled (primal-only).
    pub(crate) fn compile_refined(caller_budget_ms: Option<u64>, knobs: GoatdKnobs) -> Self {
        Self {
            timeout_ms: knobs.refine_budget_ms.or(caller_budget_ms),
            refine_cap: COMPILE_MAX_REFINE_SLOTS,
            fc_slot_cap_ms: None,
        }
    }
}

/// What `VITRI_GOATD_REFINE_BUDGET_MS` accepts, quoted in its error message.
const REFINE_BUDGET_FORM: &str = "milliseconds of budget for the goatd refine \
     schedule (0 = take the caller's share instead)";

/// Parse an explicit budget override for the refined-best schedule — the
/// one place that knob's spellings live. `N > 0` ⇒ `Some(N)` soft / `2N` hard
/// schedule deadline; absent or `0` ⇒ `None`, in which case
/// [`ModeConfig::compile_refined`] falls back to the caller's budget (and, with
/// no caller budget either, to running to completion).
pub(crate) fn refine_budget_ms(v: Option<&str>) -> Result<Option<u64>, crate::error::VitriError> {
    let ms = crate::env::parse_value("VITRI_GOATD_REFINE_BUDGET_MS", v, 0u64, REFINE_BUDGET_FORM)?;
    Ok((ms > 0).then_some(ms))
}

fn is_mindegree_variant(c: width_opt::Config<'_>) -> bool {
    matches!(
        c,
        width_opt::Config::MinDegree | width_opt::Config::MinDegreeSampleJW { .. }
    )
}

/// One slot's outcome inside one run of `run_schedule`.
enum SlotResult {
    /// A decomposition and the two measurements taken on it. They are one
    /// value, so a reader of the statistics cannot be holding a different
    /// decomposition than the one they describe.
    Produced {
        td: TreeDecomposition,
        width: u32,
        total_bag_size: usize,
    },
    /// A slot that produced nothing: skipped, out of time, or over the width
    /// bound. There is no decomposition, so there is nothing to measure.
    Empty,
}

impl SlotResult {
    /// A slot that produced `td`, measured here so that the statistics describe
    /// exactly the decomposition they are stored beside.
    fn from_td(td: TreeDecomposition) -> Self {
        SlotResult::Produced {
            width: td.width(),
            total_bag_size: td.total_bag_size(),
            td,
        }
    }

    /// The width this slot reached, or the maximum for a slot that produced
    /// nothing — the value that loses against every real width.
    fn width_or_max(&self) -> u32 {
        match self {
            SlotResult::Produced { width, .. } => *width,
            SlotResult::Empty => u32::MAX,
        }
    }
}

/// Fill remaining schedule slots with empty stubs starting at index
/// `from_idx`. Called after `hard_deadline` trips so all downstream slots get
/// a recorded stub without re-entering elimination.
fn emit_skipped_stubs(
    results: &mut Vec<SlotResult>,
    schedule: &[(width_opt::Config<'_>, u64)],
    from_idx: usize,
) {
    let skipped = schedule.len().saturating_sub(from_idx);
    results.extend(std::iter::repeat_with(|| SlotResult::Empty).take(skipped));
}

/// Intentionally does not touch `best_width` — the width bound exists to cut
/// elimination runs short, and FC is not one.
fn run_fc_slot(
    formula: &CnfFormula,
    view: GraphKind,
    cap_ms: i64,
    hard_deadline: Option<Instant>,
    results: &mut Vec<SlotResult>,
) {
    let remaining_ms: i64 = hard_deadline
        .map(|hd| crate::budget::remaining(hd).as_millis() as i64)
        .unwrap_or(cap_ms);
    let fc_timeout = remaining_ms.max(1).min(cap_ms);
    // Skip windows too small to seed useful FC iterations — FFI overhead
    // alone eats tens of ms on small graphs.
    if fc_timeout < 50 {
        return;
    }
    let res = super::super::flowcutter::flowcutter_td(
        formula,
        view,
        super::super::FcBudget::timed(fc_timeout, FC_SLOT_PATIENCE_MS, 50),
    );
    match res {
        Ok(fc_td) => {
            results.push(SlotResult::from_td(fc_td));
        }
        Err(_) => {
            results.push(SlotResult::Empty);
        }
    }
}

/// Run goatd's default schedule: large-residual skip on non-MinDegree configs,
/// 1 s soft cap on `Config::MinFill`, per-slot stats. Returns one `SlotResult`
/// per slot, plus one per refinement sample and the trailing FlowCutter slot
/// where those run.
///
/// At least one of them always carries a TD: slot 0 is exempt from both
/// between-slot skips, runs with no `width_bound` (so it cannot abort on
/// width) and with `force_emit` set (so a deadline bail is completed by an
/// emergency path decomposition rather than discarded).
///
/// `timeout_ms = None` runs to completion with no deadline (compilation mode).
/// `timeout_ms = Some(N)` enforces an N-ms soft deadline + 2N-ms hard deadline;
/// the elimination core emergency-bails to a path decomposition once the hard
/// deadline passes.
/// What one schedule slot's elimination left behind.
enum SlotOutcome {
    /// A decomposition, recorded and folded into the best width so far.
    Produced,
    /// A bag passed the width bound, so nothing usable came back. That bound
    /// comes from a slot that already produced one, so a winner exists.
    WidthAborted,
    /// The hard deadline stopped the elimination and no emergency fill was
    /// asked for, so the partial bags are not a decomposition.
    Bailed,
}

/// Record what one slot came back with — a decomposition, or the stub that
/// stands for a slot with nothing to offer — and fold a produced width into
/// `best_width`.
///
/// `force_emit` is what the slot asked of the elimination: with it, a
/// hard-deadline bail still leaves a complete (wide) decomposition behind, and
/// so counts as produced.
fn record_slot(
    run: width_opt::ConfigRun,
    force_emit: bool,
    results: &mut Vec<SlotResult>,
    best_width: &mut Option<u32>,
) -> SlotOutcome {
    match run.exit {
        ElimExit::WidthAborted => {
            results.push(SlotResult::Empty);
            SlotOutcome::WidthAborted
        }
        ElimExit::DeadlineBailed if !force_emit => {
            results.push(SlotResult::Empty);
            SlotOutcome::Bailed
        }
        ElimExit::Natural | ElimExit::DeadlineBailed => {
            let slot = SlotResult::from_td(run.td);
            let w = slot.width_or_max();
            *best_width = Some(best_width.map_or(w, |b| b.min(w)));
            results.push(slot);
            SlotOutcome::Produced
        }
    }
}

/// Builds a run's slot list for a seed, given the weight vector.
type ScheduleBuilder = for<'w> fn(u64, &'w [u32]) -> Vec<(width_opt::Config<'w>, u64)>;

/// `schedule` is the slot list to run, named as the function that builds it:
/// the sampling cores carry the SAT weight vector, which is computed here, once
/// per run, and shared by every slot and every refinement sample.
fn run_schedule(
    formula: &CnfFormula,
    view: GraphKind,
    edges: &[(u32, u32)],
    total_vertices: u32,
    seed: u64,
    schedule: ScheduleBuilder,
    cfg: ModeConfig,
) -> Vec<SlotResult> {
    let start = Instant::now();
    let deadline: Option<Instant> = cfg.timeout_ms.map(|ms| start + Duration::from_millis(ms));
    // Twice the soft timeout: enough headroom past it for the emergency bail to
    // assemble a decomposition and return it.
    let hard_deadline: Option<Instant> = cfg
        .timeout_ms
        .map(|ms| start + Duration::from_millis(ms.saturating_mul(2)));
    let prebuilt = width_opt::prebuild(total_vertices, edges);
    let jw_q = sat_score::compute_weight(formula, total_vertices);
    let schedule = schedule(seed, &jw_q);
    let large_residual = prebuilt.num_active() > width_opt::NESTED_DISS_MAX_ACTIVE;
    let mut results: Vec<SlotResult> = Vec::with_capacity(5);

    // Width of the best TD seen so far, and the bound every later slot is held
    // to. `None` means nothing has been produced yet, which is what makes slot
    // 0 run unbounded and what asks a slot for an emergency fill.
    let mut best_width: Option<u32> = None;

    // Anytime early-exit flag: set when any slot's elimination
    // emergency-bailed on `hard_deadline`, when we observe `hard_deadline`
    // expired between slots, or when slot 0 itself emergency-bailed. Breaks
    // out of the main + refinement loops so remaining slots don't re-enter
    // elimination only to immediately emergency-bail again.
    let mut hard_deadline_tripped = false;

    for (i, (config, s)) in schedule.iter().copied().enumerate() {
        // Honour the deadline between configs (when set), but always run
        // config 0 so we return something even on huge graphs that would
        // otherwise time out inside the first config.
        if i > 0 && expired(deadline) {
            results.push(SlotResult::Empty);
            continue;
        }
        // On large residuals, only min-degree variants reliably complete —
        // NestedDiss and MinFill variants can overshoot by seconds.
        if i > 0 && large_residual && !is_mindegree_variant(config) {
            results.push(SlotResult::Empty);
            continue;
        }
        let slot_start = Instant::now();
        // MinFill cap: in compilation (deadline = None) this is the only
        // MinFill bound; in bench mode it tightens MinFill to
        // min(slot_start + 1 s, schedule deadline).
        let soft_deadline = if config == width_opt::Config::MinFill {
            let cap = slot_start + Duration::from_millis(MINFILL_SLOT_MAX_MS);
            Some(deadline.map_or(cap, |d| d.min(cap)))
        } else {
            deadline
        };
        // Force an `emergency_path_decomp` fill only while no slot has
        // produced a usable TD yet. Once one has, a later slot's emergency
        // fill would be wasted work: its (wide) TD would lose lex-min
        // (width, tbs) to the existing winner anyway.
        let force_emit = best_width.is_none();
        let run = width_opt::run_config_prebuilt(
            view,
            formula.num_vars,
            &prebuilt,
            width_opt::RunSpec {
                config,
                seed: s,
                stop: ElimStop {
                    deadline: soft_deadline,
                    hard_deadline,
                    width_bound: best_width,
                },
                force_emit,
            },
        );
        hard_deadline_tripped = match record_slot(run, force_emit, &mut results, &mut best_width) {
            // Out of time, and the bags this slot holds are not a
            // decomposition — nothing later will fare better.
            SlotOutcome::Bailed => true,
            // Nothing usable from this slot, but the schedule is still inside
            // its budget.
            SlotOutcome::WidthAborted => false,
            SlotOutcome::Produced => expired(hard_deadline),
        };
        if hard_deadline_tripped {
            emit_skipped_stubs(&mut results, &schedule, i + 1);
            break;
        }
    }
    // Refinement phase: sample additional seeds of the htd-style sampling
    // config with any remaining budget. Measured ≥79% of min-fill pops have
    // ≥2 tied candidates, so different seeds explore genuinely different
    // elimination orders and can lower width on small/medium graphs where the
    // base schedule returns in tens of ms. Falls back to MinDegreeSampleJW on
    // large residuals, matching the main loop's skip rule. Bench mode stops
    // at `deadline`; compilation mode stops at `COMPILE_MAX_REFINE_SLOTS`.
    let refine_config = if large_residual {
        width_opt::Config::MinDegreeSampleJW { weight: &jw_q }
    } else {
        width_opt::Config::MinFillSampleJW { weight: &jw_q }
    };
    let max_refine = cfg.refine_cap;
    let mut refine_k: u64 = 0;
    // Refinement hard-deadline guard: normally `deadline` fires before
    // `hard_deadline` (deadline + timeout_ms = hard_deadline), but we also
    // short-circuit when a prior slot emergency-bailed so refinement doesn't
    // waste budget re-running the same bail path.
    while refine_k < max_refine
        && !hard_deadline_tripped
        && !expired(deadline)
        && !expired(hard_deadline)
    {
        let refine_seed = seed.wrapping_add(100 + refine_k.wrapping_mul(7919));
        // Refinement only runs after plateau, which means at least one prior
        // slot won — no emergency fill needed here.
        let run = width_opt::run_config_prebuilt(
            view,
            formula.num_vars,
            &prebuilt,
            width_opt::RunSpec {
                config: refine_config,
                seed: refine_seed,
                stop: ElimStop {
                    deadline,
                    hard_deadline,
                    width_bound: best_width,
                },
                force_emit: false,
            },
        );
        match record_slot(run, false, &mut results, &mut best_width) {
            // No time left for more refine slots.
            SlotOutcome::Bailed => break,
            // A width-aborted seed keeps the refinement going: another seed
            // explores a different elimination order.
            SlotOutcome::Produced | SlotOutcome::WidthAborted => refine_k += 1,
        }
    }
    // Runs vanilla FlowCutter once as a final schedule candidate the lex-min
    // picker can take wherever it dominates. Placed after the
    // sampling-refinement loop so it runs in the remaining hard-deadline
    // margin without starving sampling — under a typical `timeout_ms`
    // contract, `hard_deadline` = 2×`soft_deadline`, leaving up to
    // `soft_deadline` of slack here. The FC TD is already FC-native, so no
    // further refinement is applied.
    //
    // Primal only: on incidence, the post-process FlowCutter-cut refinement
    // can shrink some goatd TDs by orders of magnitude. An FC slot there
    // would displace those refinable TDs with one whose structure is already
    // locked in, losing that recovery — measured net negative.
    if let Some(cap_ms) = cfg.fc_slot_cap_ms.filter(|_| {
        matches!(view, GraphKind::Primal)
            && !hard_deadline_tripped
            && total_vertices <= FC_SLOT_MAX_VERTICES
            && !expired(hard_deadline)
    }) {
        run_fc_slot(formula, view, cap_ms, hard_deadline, &mut results);
    }
    results
}

// ── Selector orderings ──────────────────────────────────────────────────
//
// goatd has two winner-picking paths that both choose one candidate out of a
// schedule's results by lex-min over a sort key, sharing the tie-break
// mechanism ([`select_first_min`], first-occurrence-wins) but not the key:
//
//   • the scored path: key = `(width, cost, bagsize)`, built inline at the
//     pick site — it builds a vtree per candidate, so `vtree_cost` is
//     available.
//   • the refined path: key = `refined_select_key` = `(width,
//     total_bag_size)`. This path picks a tree decomposition before any
//     vtree is built, so no cost score exists to break ties on.
//
// Unifying the two keys would flip the winning vtree on one path — do not
// collapse them without a bench round.

/// Winner key for the refined path: `(width, total_bag_size)`. See the
/// selector-orderings note above for why this deliberately omits `vtree_cost`
/// (no vtree exists at pick time) and why it is not unified with the compile
/// path's key.
pub(crate) fn refined_select_key(width: u32, total_bag_size: usize) -> (u32, usize) {
    (width, total_bag_size)
}

/// Soft deadline (ms) handed to `run_schedule` on the shipped path. The hard
/// deadline inside the elimination core is `2×` this. Measured knee across
/// benchmark CNFs: sub-second budgets give a worse decomposition on hard
/// instances, and budgets above ~2 s do not improve it further.
const COMPILE_TIMEOUT_MS: u64 = 1000;

/// Run goatd over [`width_opt::compile_schedule`]. Single-slot, width-then-cost
/// selection, 100 refinement samples and a 1 s timeout is the configuration
/// measured best for `goatd-{primal,incidence}`.
fn best_vtree_over_schedule(
    formula: &CnfFormula,
    view: GraphKind,
    seed: u64,
    cfg: ModeConfig,
    effort_scale: f64,
) -> Result<TdConversion, String> {
    let PaceGraph {
        num_vertices: total_vertices,
        edges,
        ..
    } = view.build(formula);
    let results = run_schedule(
        formula,
        view,
        &edges,
        total_vertices,
        seed,
        width_opt::compile_schedule,
        cfg,
    );
    // Each candidate's bag metadata rides with its own vtree, so the winner
    // carries the assignment that describes it and every runner-up's is dropped
    // with the tree it described.
    let best = select_first_min(
        results.iter().filter_map(|r| {
            let SlotResult::Produced {
                td,
                width,
                total_bag_size,
            } = r
            else {
                return None;
            };
            let built = built_from_td_best(formula, td, effort_scale);
            let cost = vtree_cost(&built.vtree, formula).expect(BUILT_FROM_THIS_FORMULA);
            let key = (*width as u64, cost, *total_bag_size as u64);
            Some((built, key))
        }),
        |(_, key)| *key,
    )
    .map(|(built, _)| built);
    Ok(best.expect("run_schedule's slot 0 always produces a TD"))
}
