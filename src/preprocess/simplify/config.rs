//! What a simplification was asked to do: which stages to run, what to
//! spend on the expensive one, and the purpose the whole thing is
//! serving — which is what decides the defaults.

/// How much work the definite-variable-elimination stage may do. Carrying the
/// rounds and the time budget together in the `Option` that also switches the
/// stage on makes "DVE armed with no budget" — an inert stage that silently does
/// nothing — unrepresentable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DveBudget {
    pub rounds: usize,
    pub budget_ms: u64,
}

/// The variable-eliminating stages one preprocessing contract permits.
///
/// Fields are crate-private and the only two ways to obtain a `StageSet` are
/// `SimplifyPurpose::stages` and `StageSet::none` — a caller can pass one
/// along or ask for another, never hand-construct or edit one, so no chain
/// can assemble a stage combination its contract forbids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StageSet {
    /// Whether the equivalence REDUCTION runs: keep one representative per
    /// class, drop the partners, renumber into a smaller space.
    ///
    /// Not to be confused with the equivalence-preserving substitution inside
    /// preprocessing, which runs under every contract — it rewrites clauses onto
    /// representatives but RETAINS each partner as a constrained variable
    /// (`x ≡ y` is re-added as two binary clauses), so it drops nothing a
    /// consumer would have to recover.
    pub(crate) reduce_equivalences: bool,
    /// Whether syntactic gate detection runs — count-only, in the same class
    /// as DVE: its output feeds DVE as known-defined short-circuits.
    pub(crate) gates: bool,
    /// Count-only: a DVE-eliminated variable is never re-introduced, and its
    /// value is not recoverable from a model of the reduced formula.
    pub(crate) dve: Option<DveBudget>,
}

impl StageSet {
    /// No variable-eliminating stage at all — the identity configuration a caller
    /// asks for when nothing may drop a variable.
    pub(crate) fn none() -> StageSet {
        StageSet {
            reduce_equivalences: false,
            gates: false,
            dve: None,
        }
    }
}

/// The one typed prefix selected before the shared equivalence/gate/DVE tail.
///
/// `Disabled` is reserved for the public simplify-stage switch. Omitting only
/// the SAT-backbone budget selects `EqIter`, so no budget sentinel can
/// accidentally turn off unrelated simplification work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SimplifyPrefix {
    Disabled,
    EqIter,
    Backbone {
        budget_ms: u64,
        equivalence_budget_ms: Option<u64>,
    },
}

/// What simplification phases to run.
pub(crate) struct SimplifyConfig {
    /// The variable-eliminating stages this run may use — the contract's own
    /// list; call sites treat it as opaque rather than editing individual flags.
    pub stages: StageSet,
    /// The prefix plan. This is the sole owner of whether preprocessing is
    /// disabled, ordinary equivalence iteration runs, or SAT backbone probing
    /// precedes it.
    pub prefix: SimplifyPrefix,
    /// Whole-run wall-clock deadline. Preprocess phase budgets are clamped to
    /// the remaining budget at each phase start; `None` = no clamp.
    pub deadline: Option<std::time::Instant>,
    /// Variable ids DVE must never eliminate.
    pub frozen_vars: rustc_hash::FxHashSet<crate::cnf::VarId>,
}

/// What the preprocessing this configuration describes must PRESERVE — the single
/// owner of which variable-eliminating stages may run.
///
/// One variant per soundness contract [`simplify`](super::simplify) can serve. The
/// projection-preserving contract is deliberately absent: its stages are a
/// different chain, and leaving it unnameable here is what stops a projected
/// run from being configured with the count-preserving stage list, every one
/// of which has a documented way to be wrong under a projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SimplifyPurpose {
    /// Count-preserving: the model count survives, the function need not.
    /// Count-only stages (gates, DVE) are sound because the output is consumed
    /// as a count with the `2^k` lift applied.
    Count,
    /// Count-preserving under WEIGHTS: same stage list as [`Count`](Self::Count),
    /// with the caller additionally freezing the unequal-weight variables out of
    /// DVE ([`SimplifyConfig::frozen_vars`]) so every elimination it does make is
    /// one a scalar factor can pay for.
    WeightedCount,
    /// Function-preserving: the reduced formula plus the caller's record must
    /// reconstruct the original Boolean function, so a stage may run only if its
    /// effect is written down. The equivalence reduction qualifies — a dropped
    /// partner is one signed literal of its surviving representative, which is
    /// what [`SimplifiedFormula::original_fates`](super::SimplifiedFormula::original_fates) states and the record writes
    /// down. Gate detection and DVE do not: each removes a variable determined
    /// by a FUNCTION of the survivors rather than by one of their literals, and
    /// no record field can name that.
    Function,
}

impl SimplifyPurpose {
    /// The stages this contract permits — THE definition of each chain's stage
    /// list, and the only place a stage is turned on.
    pub(crate) fn stages(self) -> StageSet {
        let count_only = matches!(
            self,
            SimplifyPurpose::Count | SimplifyPurpose::WeightedCount
        );
        let dve = crate::config::DvePolicy::default();
        StageSet {
            reduce_equivalences: true,
            gates: count_only,
            dve: count_only.then_some(DveBudget {
                rounds: dve.rounds,
                budget_ms: dve.budget_ms,
            }),
        }
    }
}

impl SimplifyConfig {
    /// The base configuration for `purpose`: its shared-tail stage list plus a
    /// disabled prefix. Call sites express only their intentional prefix,
    /// deadline, and policy reductions via struct-update syntax over it.
    ///
    /// `keep_all_vars` forces every variable-eliminating tail stage off
    /// regardless of what `purpose` would otherwise allow — it can only turn
    /// stages off, never on. The caller separately selects `prefix`; the public
    /// stage-off resolver pairs this tail veto with [`SimplifyPrefix::Disabled`].
    pub(crate) fn for_purpose(purpose: SimplifyPurpose, keep_all_vars: bool) -> SimplifyConfig {
        SimplifyConfig {
            stages: if keep_all_vars {
                StageSet::none()
            } else {
                purpose.stages()
            },
            prefix: SimplifyPrefix::Disabled,
            deadline: None,
            frozen_vars: rustc_hash::FxHashSet::default(),
        }
    }
}
