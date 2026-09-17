//! What a preprocessing run reports about itself: per-stage outcomes,
//! telemetry and the deterministic decision trace.

use super::*;

/// What one preprocessing stage did.
///
/// Reported so a caller can act on the difference rather than read it in a log.
/// The distinction that carries the most weight is [`GaveUp`](Self::GaveUp)
/// against [`Discarded`](Self::Discarded): a stage that ran out of budget may
/// well produce something on a different budget, while a stage whose result was
/// rejected produces the same rejection again. A caller deciding whether to
/// preprocess the same formula a second time is deciding between those two.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StageOutcome {
    /// The stage ran and its result was kept.
    Ran,
    /// The stage did not run at all.
    Skipped(SkipReason),
    /// The stage produced nothing to keep inside the budget it was given.
    ///
    /// Not a failure — it is the anytime contract working. A budgeted stage
    /// stops STARTING work at its deadline and hands back the soundest
    /// checkpoint it has reached; this covers both a stage that had reached
    /// none and one whose checkpoint came back so late that it was dropped as
    /// bought with budget the caller no longer has. Either way a caller with
    /// more wall can call again and expect a different answer.
    ///
    /// How late is too late is per mode, because the modes do not lose the same
    /// thing by being strict: `mc` and `wmc` run the reduction in a forked child
    /// killed shortly after the deadline and discard a late result (`mc` keeps
    /// it if `VITRI_ARJUN_KEEP_OVERRUN` asks, which also runs it in this
    /// process), while `pmc` and `pwmc` keep their checkpoint however late,
    /// Arjun being their first stage and the rest of their chain cheap.
    GaveUp,
    /// The stage produced a result and it was then rejected.
    ///
    /// Calling again on the same formula with the same configuration produces
    /// the same rejection.
    Discarded(DiscardReason),
}

/// Why a stage did not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SkipReason {
    /// The configuration did not ask for it — see
    /// [`PreprocessStages`](crate::config::PreprocessStages), or
    /// [`ArjunOptions::sbva`](crate::preprocess::ArjunOptions::sbva) for
    /// bounded variable addition.
    NotRequested,
    /// There was nothing left for it to work on.
    NothingToDo,
}

/// Why a stage's result was rejected after it had been produced.
///
/// Every one of these is a property of the formula and the configuration, not
/// of the wall clock: the same call makes the same judgement again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiscardReason {
    /// The reduction grew the clause count. Fewer variables does not make a
    /// larger formula the better one to hand a compiler.
    NotSmaller,
    /// A projection-preserving reduction left the projection exactly as it
    /// found it, so it bought no counting benefit and can compile worse than
    /// the formula it was given.
    NoProjectionGain,
    /// A weighted reduction that dropped weighted mass its multiplier does not
    /// carry, or left the formula unchanged.
    WeightedUnusable,
    /// The variable map the reduction reported was not injective, so counts
    /// taken over the reduced formula could not be lifted back soundly. A
    /// refusal on correctness grounds rather than on quality.
    NonInjectiveMap,
}

impl DiscardReason {
    /// The phrase this reason is reported by, so what a caller reads and what
    /// the diagnostic line says are one string.
    pub(super) fn phrase(self) -> &'static str {
        match self {
            DiscardReason::NotSmaller => "it grew the clause count",
            DiscardReason::NoProjectionGain => "it did not minimize the projection",
            DiscardReason::WeightedUnusable => "lossy or inert",
            DiscardReason::NonInjectiveMap => "non-injective variable map",
        }
    }
}

/// What each stage of the chain did. See [`StageOutcome`].
///
/// A `None` field is a stage this mode's chain does not have — `compile` runs
/// no Arjun, and bounded variable addition has nothing to report when the
/// reduction around it never ran.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StageReport {
    /// This crate's own simplify chain.
    pub simplify: Option<StageOutcome>,
    /// The Arjun reduction.
    pub arjun: Option<StageOutcome>,
    /// Bounded variable addition, as part of the Arjun reduction.
    pub sbva: Option<StageOutcome>,
}

/// The cardinality lift, attributed to the stage that earned it.
///
/// A caller lifting one final count wants [`total_pow2`](Self::total_pow2) and
/// can ignore the split. A caller that re-reduces a formula *derived* from the
/// one this run reduced — a cofactor, a component, a conditioned branch — needs
/// the split: the reduction it is about to run applies to the formula the Arjun
/// stage was given ([`PreprocessBundle::arjun_input`]), so only the Arjun
/// stage's own exponent is the one to reconcile against. Folding in the simplify
/// chain's share would count it once per derived formula instead of once for the
/// run.
///
/// Both halves are zero under a weighted mode, where each eliminated variable
/// contributes an exact rational rather than a factor of two and the whole lift
/// is [`PreprocessRecord::weight_lift`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CountLift {
    /// Earned by this crate's own simplify chain.
    pub simplify_pow2: u32,
    /// Earned by the Arjun reduction.
    pub arjun_pow2: u32,
}

impl CountLift {
    /// The whole exponent: `count(original) == count(reduced) × 2^total_pow2`,
    /// which is [`PreprocessRecord::count_lift_pow2`].
    pub fn total_pow2(self) -> u32 {
        self.simplify_pow2 + self.arjun_pow2
    }
}

/// Wall-clock and probing telemetry from one preprocessing call.
///
/// A phase duration is `None` when that phase was not attempted. `Some(0)`
/// means it was attempted and completed in less than one millisecond. These
/// measurements describe work performed by the call, including a reduction
/// whose result was later discarded; [`PreprocessBundle::stages`] describes
/// the outcome of the stage instead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PreprocessTelemetry {
    /// Total wall time spent in preprocessing, including all enabled phases.
    pub total_ms: u64,
    /// The crate's own simplify chain, when that stage was attempted.
    pub simplify_ms: Option<u64>,
    /// SAT backbone probing inside the simplify chain, when attempted.
    pub backbone_ms: Option<u64>,
    /// SAT equivalence probing inside the simplify chain, when attempted.
    pub equivalence_ms: Option<u64>,
    /// Definability elimination inside the simplify chain, when attempted.
    pub dve_ms: Option<u64>,
    /// Arjun's opaque native reduction call, including SBVA when it participated.
    pub arjun_ms: Option<u64>,
    /// Backbone literals proved by the probing phase.
    pub backbone_found: usize,
    /// Backbone probes completed by the probing phase.
    pub backbone_probes: usize,
}

/// A preprocessing phase whose budget can be measured in deterministic work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PreprocessPhase {
    /// SAT backbone probing.
    Backbone,
    /// SAT literal-equivalence probing.
    Equivalence,
    /// Definite-variable elimination, including its fixed post-DVE pass.
    Dve,
}

/// Outcomes of the solver probes completed inside one deterministic phase.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProbeDecisionCounts {
    /// Total completed solver calls (`satisfiable + unsatisfiable + unknown`).
    pub completed: usize,
    /// Calls that produced a model.
    pub satisfiable: usize,
    /// Calls that proved the assumptions inconsistent.
    pub unsatisfiable: usize,
    /// Calls stopped by a deterministic solver limit.
    pub unknown: usize,
}

/// The deterministic budget and work spent by one preprocessing phase.
///
/// More than one [`PreprocessPhase::Dve`] entry is possible: the fixed
/// post-DVE pass shares the run-owned meter but has its own nominal allowance.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PreprocessPhaseTrace {
    /// Which calibrated work rate converted this phase's allowance.
    pub phase: PreprocessPhase,
    /// The nominal allowance after the deterministic configured-wall clamp.
    pub budget_ms: u64,
    /// `budget_ms` converted through the phase's fixed work rate.
    pub budget_units: u64,
    /// Work charged between this phase's start and finish.
    pub spent_units: u64,
    /// Solver outcomes observed inside the phase.
    pub probes: ProbeDecisionCounts,
}

/// Aggregate control-flow decisions made by all DVE passes in one simplify
/// chain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DveDecisionTrace {
    /// Main-loop rounds that started.
    pub rounds: usize,
    /// Aggressive-cascade passes that started.
    pub aggressive_passes: usize,
    /// Variables eliminated as defined across the main and post-DVE passes.
    pub defined_eliminated: usize,
    /// Variables eliminated by equivalence merging across those passes.
    pub equivalence_eliminated: usize,
    /// Whether any DVE pass stopped at its work allowance.
    pub budget_hit: bool,
}

/// Typed observability for deterministic preprocessing decisions.
///
/// Present on [`PreprocessBundle::decision_trace`] only when
/// [`PreprocessClock::Deterministic`](crate::config::PreprocessClock::Deterministic)
/// was selected. It intentionally contains no real elapsed time: wall time is
/// diagnostic telemetry, not part of the reproducible decision record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PreprocessDecisionTrace {
    /// Total charged work across the simplify chain.
    pub total_units: u64,
    /// Phase decisions in execution order.
    pub phases: Vec<PreprocessPhaseTrace>,
    /// Aggregate DVE decisions, including the fixed post-DVE pass.
    pub dve: DveDecisionTrace,
}

impl PreprocessTelemetry {
    /// Publish simplify's private measurements under the public stage-presence
    /// contract. The identity simplify call used for a disabled stage is not an
    /// attempted phase, even though it shares the same internal code path.
    pub(super) fn from_simplified(simplified: &SimplifiedFormula, attempted: bool) -> Self {
        let measured = simplified.telemetry;
        PreprocessTelemetry {
            simplify_ms: attempted.then_some(measured.total_ms),
            backbone_ms: measured.backbone_ms,
            equivalence_ms: measured.equivalence_ms,
            dve_ms: measured.dve_ms,
            backbone_found: measured.backbone_found,
            backbone_probes: measured.backbone_probes,
            ..PreprocessTelemetry::default()
        }
    }
}
