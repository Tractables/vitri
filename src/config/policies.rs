//! The policy types a [`RunConfig`] is assembled from, and the chain each
//! mode runs.

use super::*;

/// Whether a formula is split into its independent components before vtree
/// construction.
///
/// [`ComponentPolicy::token`] spells each variant as the `--components` flag
/// writes it and [`ComponentPolicy::parse`] reads one back, so an embedder
/// offering the flag does not have to restate the vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentPolicy {
    /// Split the formula into connected components, build a vtree per component
    /// under a pro-rata share of the budget, and graft them into one whole-formula
    /// vtree. The default: a smaller graph gives each component a better
    /// decomposition.
    ///
    /// The numbering-only baselines ([`crate::spec::baseline_spec_names`])
    /// ignore this — they gain nothing from a per-component graph — as do
    /// single-component formulas. Neither is refused the way an inert knob is:
    /// this is the default policy, so a run under a baseline spec never asked
    /// for a split, and [`ComponentPolicy::Whole`] under one is a request those
    /// specs already meet.
    Split,
    /// Build ONE vtree over the whole formula, whatever its component
    /// structure. Required when an externally supplied vtree must span the
    /// entire variable space, and available to a consumer that wants a single
    /// monolithic tree.
    Whole,
}

impl ComponentPolicy {
    /// Every policy, in the order a message or a `--help` line offers them.
    ///
    /// The vocabulary itself is [`Self::token`]'s match, which the compiler
    /// keeps exhaustive; this fixes the ORDER and is what [`Self::names`] and
    /// [`Self::parse`] read.
    const ALL: &'static [ComponentPolicy] = &[ComponentPolicy::Split, ComponentPolicy::Whole];

    /// The `--components` token naming this policy, the exact inverse of
    /// [`Self::parse`].
    pub fn token(self) -> &'static str {
        match self {
            ComponentPolicy::Split => "split",
            ComponentPolicy::Whole => "whole",
        }
    }

    /// Parses a `--components` token: any [`Self::names`] entry. The inverse of
    /// [`Self::token`] by construction — it is that spelling looked up.
    pub fn parse(token: &str) -> Option<Self> {
        ComponentPolicy::ALL
            .iter()
            .copied()
            .find(|p| p.token() == token)
    }

    /// Every `--components` token, in table order — for a shell over this crate
    /// that offers the vocabulary it will accept rather than keeping a copy.
    pub fn names() -> impl Iterator<Item = &'static str> {
        ComponentPolicy::ALL.iter().map(|p| p.token())
    }

    /// Whether one vtree must span the whole variable space rather than one per
    /// component.
    pub fn is_whole(self) -> bool {
        matches!(self, ComponentPolicy::Whole)
    }
}

/// Which preprocessing stages [`crate::bundle::preprocess`] runs.
///
/// Turning a stage off does not select a different code path — it configures the
/// one path to do nothing at that step (a no-op simplify configuration,
/// a skipped Arjun call), so the bundle's record stays exactly as truthful about
/// what ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreprocessStages {
    /// This crate's own simplify chain, whose stages `docs/preprocessing.md`
    /// lists in order.
    pub simplify: bool,
    /// Arjun. Always linked in; this switch is the only way to skip the stage.
    pub arjun: bool,
}

impl Default for PreprocessStages {
    /// Everything on — the production configuration.
    fn default() -> Self {
        PreprocessStages {
            simplify: true,
            arjun: true,
        }
    }
}

impl PreprocessStages {
    /// Which of these toggles preprocessing for `mode` actually reads. A `false`
    /// field names a stage that mode's chain does not have, so setting it either
    /// way changes nothing.
    ///
    /// The command line is the caller of this: a stage flag the resolved mode
    /// would ignore is refused there rather than accepted and dropped. The
    /// answer is the chain's, read through `Chain`, so which stages a mode
    /// reads and which chain runs it are one statement.
    #[must_use]
    pub fn read_under(mode: crate::cnf::Mode) -> Self {
        Chain::for_mode(mode).stages_read()
    }
}

/// How much work one enabled definite-variable-elimination pass may do.
///
/// This is nested in [`SimplifyPolicy::dve`], so `None` disables the pass and
/// `Some` always means it is armed. An armed policy with zero rounds or zero
/// milliseconds is rejected by [`RunConfig::validate`] rather than silently
/// doing no work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DvePolicy {
    /// Maximum number of elimination rounds.
    pub rounds: usize,
    /// Wall-clock budget for all rounds and their vivification, in milliseconds.
    pub budget_ms: u64,
}

/// Which clock preprocessing's budget decisions read.
///
/// Wall-clock mode preserves the ordinary deadline and terminator behavior.
/// Deterministic mode converts the same millisecond policies to charged work,
/// making the preprocessing decisions a function of the formula and this
/// configuration rather than of machine load. The policy is per run: Vitri
/// never reads an embedding application's environment to select it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreprocessClock {
    /// Measure preprocessing budgets against elapsed wall time. The default for
    /// existing callers.
    #[default]
    WallClock,
    /// Measure preprocessing budgets against deterministic work charges.
    Deterministic {
        /// The configured whole-run wall, used only to clamp each phase's
        /// nominal millisecond allowance before converting it to work units.
        /// `None` leaves every nominal phase allowance unchanged.
        configured_wall_ms: Option<u64>,
    },
}

impl Default for DvePolicy {
    fn default() -> Self {
        DvePolicy {
            rounds: 30,
            budget_ms: 3_000,
        }
    }
}

/// Policy for Vitri's one simplify path.
///
/// The defaults are the production policy. Count-preserving modes may use every
/// field. Function-preserving `compile` uses the backbone and equivalence
/// budgets but caps gate detection and DVE off because their eliminations are
/// not reconstructible. Projected modes use their separate projection-safe
/// chain and therefore reject a non-default simplify policy as inert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimplifyPolicy {
    /// Budget for SAT backbone/equivalence probing and backbone stripping.
    /// `None` skips those probing-specific steps while retaining ordinary
    /// equivalence iteration and the configured gate/DVE tail. Disable the
    /// whole simplify chain through [`PreprocessStages::simplify`].
    pub backbone_budget_ms: Option<u64>,
    /// Budget for SAT equivalence probes inside the backbone prefix. This must
    /// be `None` when [`Self::backbone_budget_ms`] is `None`; syntactic
    /// equivalence handling remains enabled independently.
    pub equivalence_budget_ms: Option<u64>,
    /// Detect syntactic gates before DVE. Count-preserving modes only.
    pub detect_gates: bool,
    /// DVE work, or `None` to disable DVE. Count-preserving modes only.
    pub dve: Option<DvePolicy>,
}

impl Default for SimplifyPolicy {
    fn default() -> Self {
        SimplifyPolicy {
            backbone_budget_ms: Some(300_000),
            equivalence_budget_ms: Some(300),
            detect_gates: true,
            dve: Some(DvePolicy::default()),
        }
    }
}

impl SimplifyPolicy {
    /// Whether a caller changed a count-only knob from the production policy.
    pub(super) fn customizes_count_only(self) -> bool {
        let default = Self::default();
        self.detect_gates != default.detect_gates || self.dve != default.dve
    }
}

/// How the Arjun stage obtains its wall-clock budget.
///
/// This selects the budget in the existing preprocessing pipeline; it does not
/// select a different Arjun entry point. In either case the run's absolute
/// [`RunConfig::deadline`] remains the final bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArjunBudget {
    /// Derive Arjun's share from [`RunConfig::budget_ms`] using the crate's
    /// normal ratio, floor and cap. The default and the historical behaviour.
    #[default]
    Derived,

    /// Give Arjun exactly this duration, without applying the derived ratio,
    /// floor or cap. An earlier [`RunConfig::deadline`] still clamps it.
    ///
    /// Refused when the Arjun stage is off or the resolved mode's chain has no
    /// Arjun stage, because accepting it there would silently discard a caller's
    /// explicit budget.
    Exact(Duration),
}

/// Whether a sound Arjun reduction that grew the clause count is exported.
///
/// This controls only the count-preserving chain's `NotSmaller` quality gate.
/// Every correctness gate remains mandatory whichever policy is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ArjunClauseGrowth {
    /// Discard a clause-growing reduction and export the pre-Arjun formula.
    #[default]
    Reject,
    /// Keep a sound clause-growing reduction.
    KeepSound,
    /// Discard a candidate whose clause count exceeds this caller-provided
    /// baseline.
    ///
    /// This lets an embedding caller give Arjun one formula while judging its
    /// result against a different count-preserving formula it will compile.
    /// It is valid only for `mc`/`wmc` with the Arjun stage enabled.
    RejectAgainst(usize),
}

impl ArjunClauseGrowth {
    /// Whether this policy needs the count-preserving Arjun stage to make a
    /// clause-growth decision.
    pub(super) fn requires_count_arjun(self) -> bool {
        !matches!(self, Self::Reject)
    }

    /// The clause-count baseline for an Arjun input with `input_clauses`
    /// clauses. `KeepSound` still names the input count; the shared stage gate
    /// is what bypasses its `NotSmaller` verdict.
    pub(crate) fn clause_count_baseline(self, input_clauses: usize) -> usize {
        match self {
            Self::Reject | Self::KeepSound => input_clauses,
            Self::RejectAgainst(baseline) => baseline,
        }
    }
}

/// How much of the projection-preserving preprocessing chain runs.
///
/// This policy is meaningful only under `pmc`/`pwmc`. It is separate from
/// [`ArjunClauseGrowth`]: projected Arjun is judged by whether it minimized the
/// projection, not by whether its clause count grew.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProjectionPolicy {
    /// Run the complete projection chain: Arjun, then show-frozen
    /// strengthening and strict no-growth projected BVE.
    #[default]
    Full,
    /// Run only projected Arjun, with the named policy for a sound result that
    /// did not minimize the projection.
    ArjunOnly(ProjectionNoGain),
}

/// Whether projected Arjun exports a sound result that did not shrink the
/// projection set.
///
/// Every correctness gate, including injectivity of the variable map, remains
/// mandatory under both policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProjectionNoGain {
    /// Discard a result that did not minimize the projection.
    #[default]
    Reject,
    /// Keep the sound result despite the absence of projection-set gain.
    KeepSound,
}

/// Which preprocessing chain a mode runs.
///
/// [`crate::bundle::preprocess`] has three of them, and the five modes partition
/// across them. That partition decides two things — which chain the instance
/// goes down, and which stage toggles are live on the way — and this is where it
/// is stated, so a chain that gains or loses a stage cannot leave a refusal
/// message describing the chain it used to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Chain {
    /// Count-preserving: `mc` and `wmc`.
    Count,
    /// Projection-preserving: `pmc` and `pwmc`.
    Projection,
    /// Function-preserving: `compile`, which is not a counting track at all.
    Compile,
}

impl Chain {
    /// The chain `mode` runs. Exhaustive, so a new mode fails to compile until
    /// it names the chain that answers for it.
    pub(crate) fn for_mode(mode: crate::cnf::Mode) -> Self {
        use crate::cnf::Mode;
        match mode {
            Mode::Mc | Mode::Wmc => Chain::Count,
            Mode::Pmc | Mode::Pwmc => Chain::Projection,
            Mode::Compile => Chain::Compile,
        }
    }

    /// The stage toggles this chain reads. The struct literals are exhaustive,
    /// so a new stage fails to compile until every chain answers for it.
    pub(crate) fn stages_read(self) -> PreprocessStages {
        match self {
            Chain::Count => PreprocessStages {
                simplify: true,
                arjun: true,
            },
            // The projected chain is Arjun's projection-set minimization and the
            // show-frozen projected reduction, and nothing else: the simplify
            // chain's `2^k` lift charges ×2 for a variable a projection retires
            // at ×1, so it has no place there.
            Chain::Projection => PreprocessStages {
                simplify: false,
                arjun: true,
            },
            // Arjun eliminates on the strength of an independent support, and a
            // reconstruction entry names a literal rather than a function, so
            // `compile` runs the simplify chain alone.
            Chain::Compile => PreprocessStages {
                simplify: true,
                arjun: false,
            },
        }
    }
}
