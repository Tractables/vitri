//! Time-budget helpers: scale internal sub-budgets as a fraction of the
//! per-run wall-clock hint.
//!
//! Every function here is pure. The hint itself is
//! [`RunConfig::budget_ms`](crate::config::RunConfig::budget_ms), carried to
//! each site as an argument — preprocessing passes
//! [`RunConfig::effective_budget_ms`](crate::config::RunConfig::effective_budget_ms)
//! into the stage that needs it, and vtree construction carries it in the
//! build limits `component::build_vtree` assembles. A run with no budget is
//! deliberately unbounded, not accidentally so.

/// How long there is until `deadline` — zero once it has passed, which every
/// consumer reads as "stop now" rather than as "unbounded".
///
/// "Now" is [`crate::decompose::meter::now`], which is the real clock unless a
/// deterministic construction budget is in force. That is what converts every
/// deadline expression in vtree construction at once: the deadlines themselves
/// are unchanged, and the clock they are compared against is the one the
/// budget named.
pub(crate) fn remaining(deadline: std::time::Instant) -> std::time::Duration {
    deadline.saturating_duration_since(crate::decompose::meter::now())
}

/// Whether `deadline` has passed. `None` is the unbounded run — it never
/// expires, the same reading of an absent deadline every function here takes.
///
/// The one spelling of the check, so a caller cannot ask the question in a way
/// that treats "no deadline" as "out of time".
pub(crate) fn expired(deadline: Option<std::time::Instant>) -> bool {
    deadline.is_some_and(|d| crate::decompose::meter::now() >= d)
}

/// One item's share of a `deadline` several items divide, as a deadline of
/// its own: `weight` out of `total_weight` of whatever time is left.
///
/// Pro-rata rather than even — an even split starves the one big item in a set
/// of otherwise tiny ones. The share is counted BACK from the shared deadline
/// rather than forward from a second clock reading, so an item's deadline is
/// never later than the one it divides, and re-reading the clock per item rolls
/// unspent time forward to the ones that follow.
///
/// Once `deadline` has passed the share is zero and the work receiving it
/// starts already expired, which is the caller's signal to stop.
pub(crate) fn pro_rata_deadline(
    deadline: std::time::Instant,
    weight: usize,
    total_weight: usize,
) -> std::time::Instant {
    let left = remaining(deadline);
    let share = left
        .mul_f64(weight as f64 / total_weight.max(1) as f64)
        .min(left);
    deadline - (left - share)
}

/// A stage budget cut down to what the run has left. Without a deadline the
/// stage keeps its own budget: a run with no deadline is deliberately unbounded.
pub(crate) fn clamp(
    budget: std::time::Duration,
    deadline: Option<std::time::Instant>,
) -> std::time::Duration {
    match deadline {
        Some(d) => budget.min(remaining(d)),
        None => budget,
    }
}

/// The budget-scaling rule every sub-budget below is expressed in: a fraction
/// of the hint, clamped, with an absolute default when there is no hint.
pub(crate) fn resolve_scaled(
    budget_ms: Option<u64>,
    ratio: f64,
    abs_default: u64,
    min: u64,
    max: u64,
) -> u64 {
    if let Some(t) = budget_ms {
        return ((t as f64 * ratio) as u64).clamp(min, max);
    }
    abs_default
}

/// The floor is low because preprocessing that runs out of time keeps whatever
/// Arjun completed within the window and records it — partial output is
/// still usable.
///
/// The short-window ratio was 1/6 as of the measurement below; it was 1/12
/// before that. 1/12 was tuned for a consumer whose single downstream stage
/// owned the rest of the wall, so every second spent reducing was a second that
/// stage lost. A consumer that slices its wall into several attempts is buying a
/// shorter first attempt and the attempts behind it, and 1/6 is the
/// corresponding re-sizing. Measured on a model-counting-competition board at a
/// two-minute timeout: +7 net solved instances, no count regressions. The 1/4
/// long-window branch, the clamp and the no-hint default were not part of that
/// change.
pub(crate) fn arjun_budget_ms(budget_ms: Option<u64>) -> u64 {
    let short_window = budget_ms.is_some_and(|t| t <= 300_000);
    let ratio = if short_window { 1.0 / 6.0 } else { 0.25 };
    resolve_scaled(
        budget_ms,
        ratio,
        ARJUN_BUDGET_ABS_DEFAULT_MS,
        ARJUN_BUDGET_FLOOR_MS,
        ARJUN_BUDGET_CAP_MS,
    )
}

const ARJUN_BUDGET_ABS_DEFAULT_MS: u64 = 600_000;
const ARJUN_BUDGET_FLOOR_MS: u64 = 5_000;
const ARJUN_BUDGET_CAP_MS: u64 = 600_000;

/// Wall-clock SAFETY NET for vtree CONSTRUCTION: how much of the remaining
/// per-CNF budget the whole portfolio candidate build (all candidates, all
/// components) may spend before it must hand back what it has.
/// `remaining_wall_ms` is the time left until the run's deadline at the moment
/// construction starts.
///
/// This is a SAFETY NET, not a tuning knob — deliberately generous:
/// - The ceiling of a single healthy candidate is well under the floor (goatd's
///   own doc measures its refinement loop at ~65 s worst case on a ~1k-var
///   formula), so no candidate that solves within this floor has its
///   construction truncated.
/// - The 90 s FLOOR keeps short budgets untouched: at a 120 s budget the budget is
///   the floor, which exceeds the budget itself, so the deadline is inert there
///   (the caller additionally clamps it to the run's deadline).
/// - The 900 s CAP is what actually bites: at an hour-long budget a pathological
///   build can otherwise spend most of it and hand the consumer nothing to
///   compile, so construction is cut to at most a quarter of the budget.
///
/// Enforcement is in the portfolio driver, and the deadline alone is not all of
/// it: the driver consults it between candidates, so a candidate that has
/// already started would otherwise run to completion however long it takes —
/// and that is the candidate which overruns the ceiling. Each candidate is
/// additionally capped at the time left when it starts
/// (`RunState::cand_wall_ms`).
///
/// The bound is soft, at the granularity of one FlowCutter restart iteration:
/// the vendored library checks its deadline between iterations rather than
/// inside one. Its two greedy pre-passes are abandoned at the deadline, but the
/// first multilevel partition of a build that holds no decomposition yet runs
/// unbounded, because returning nothing is worse than returning late.
///
/// No env override, by design: an escape hatch here would be a knob whose only
/// job is to disable a safety net. The individual construction knobs that feed
/// into how long a candidate takes (`vtree_effort_scale`,
/// `VITRI_GOATD_REFINE_BUDGET_MS`) keep their own overrides and compose with
/// this ceiling — a tighter goatd budget still wins, this only imposes a roof.
pub(crate) fn vtree_budget_ms(remaining_wall_ms: u64) -> u64 {
    (remaining_wall_ms / 3).clamp(VTREE_BUDGET_FLOOR_MS, VTREE_BUDGET_CAP_MS)
}

const VTREE_BUDGET_FLOOR_MS: u64 = 90_000;
const VTREE_BUDGET_CAP_MS: u64 = 900_000;

/// The absolute deadline a DETERMINISTIC construction budget names: `units` of
/// work, converted to the milliseconds the deadline machinery is written in and
/// counted forward from `epoch`, the instant the meter was armed at.
///
/// The run's own deadline plays no part, and this is the one construction policy
/// of which that is true. A deadline anchored before preprocessing leaves
/// construction however much of the wall preprocessing happened not to use,
/// which differs run to run — exactly the dependence a deterministic budget
/// exists to remove. What bounds construction here is the work it is allowed to
/// do, and nothing else.
///
/// `None` where the platform's clock cannot represent an instant that far ahead
/// — a budget nothing could spend bounds nothing, and the meter's own clock
/// saturates the same way, so an unbounded construction is what both ends agree
/// on. Returning it beats panicking on a number a caller is free to pass.
pub(crate) fn deterministic_deadline(
    units: u64,
    epoch: std::time::Instant,
) -> Option<std::time::Instant> {
    epoch.checked_add(std::time::Duration::from_millis(
        crate::decompose::meter::wall_ms_for_units(units),
    ))
}

/// [`vtree_budget_ms`] as a deadline: the share of `run_deadline` construction
/// gets when it starts at `now`, never later than `run_deadline` itself.
///
/// The share policy alone. Which policy a run uses is
/// [`ConstructionBudget`](crate::config::ConstructionBudget)'s to say, and
/// [`RunConfig::construction_deadline`](crate::config::RunConfig::construction_deadline)
/// is where they are told apart — including the run with no deadline at all,
/// which never reaches here.
pub(crate) fn vtree_share_deadline(
    run_deadline: std::time::Instant,
    now: std::time::Instant,
) -> std::time::Instant {
    let remaining_ms = run_deadline.saturating_duration_since(now).as_millis() as u64;
    let budget = std::time::Duration::from_millis(vtree_budget_ms(remaining_ms));
    (now + budget).min(run_deadline)
}

/// Construction-effort multiplier for `budget_ms`, relative to a calibration
/// baseline timeout. `None` (unbounded) is the baseline, `1.0`.
///
/// Two consumers scale by it: the portfolio driver's FlowCutter step and
/// iteration counts, and the multilevel hypergraph bisector's restart and
/// V-cycle counts. Both are calibrated for a short budget (~90s), where every
/// second of construction is a second the caller does not get back. Under a
/// long budget construction is a small share of the run by comparison, so these
/// counts grow with the declared budget.
///
/// The sub-linear exponent avoids an hour-long budget demanding a linear 40×
/// blowup while still scaling with the declared budget.
pub(crate) fn vtree_effort_scale(budget_ms: Option<u64>) -> f64 {
    match budget_ms {
        Some(t) => (t as f64 / EFFORT_BASELINE_MS as f64)
            .powf(EFFORT_EXPONENT)
            .clamp(EFFORT_SCALE_MIN, EFFORT_SCALE_MAX),
        None => 1.0,
    }
}

const EFFORT_BASELINE_MS: u64 = 90_000;
const EFFORT_EXPONENT: f64 = 0.5;
const EFFORT_SCALE_MIN: f64 = 1.0;
const EFFORT_SCALE_MAX: f64 = 8.0;
