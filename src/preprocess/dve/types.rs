//! DVE result types: what happened to each variable, and the main result struct.

use crate::cnf::{Clause, CnfFormula, Literal};
use crate::preprocess::renumber::Renumber;

/// What DVE did with one variable of the formula it was given.
///
/// Provenance, not a static property of the residual: a variable's fate is
/// relative to the elimination order (in `y ↔ a∧b`, resolving `y` out leaves
/// `a` and `b` free even though both occur in the pre-DVE formula), so it
/// cannot be reconstructed by looking at the reduced formula afterwards.
///
/// The distinction is what the count is built on: a defined or merged variable
/// is fixed by the ones that remain and contributes ×1, while a free one
/// contributes ×2 (or ×(w⁻+w⁺) under weights).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DveFate {
    /// Still in the residual formula.
    Kept,
    /// Resolved away: the remaining variables determine its value.
    Defined,
    /// Left in no clause of the residual, so nothing constrains it any more.
    Free,
    /// Merged into an equivalent literal, which carries its value.
    Equiv {
        /// The literal it folds onto — `v ≡ rep` — over a variable of the
        /// DVE-INPUT space, the same space the fates are indexed by. That
        /// variable may itself be an `Equiv`, so a reader chases the chain to
        /// one that is not (see
        /// [`crate::preprocess::weighted_lift::dve_equiv_survivor`]).
        rep: Literal,
    },
}

impl DveFate {
    /// Whether the variable is gone from the residual formula.
    pub(crate) fn eliminated(self) -> bool {
        !matches!(self, DveFate::Kept)
    }

    /// The literal it folds onto, for a variable merged as an equivalence.
    pub(crate) fn as_equiv(self) -> Option<Literal> {
        match self {
            DveFate::Equiv { rep } => Some(rep),
            _ => None,
        }
    }
}

/// Ids of the variables a fate table leaves free, each contributing a factor of
/// 2 to the model count.
///
/// The one reading of "free" over a fate table. Two structs hold one: the
/// pass's own [`DveResult`], and the [`DveReduction`] the simplify chain keeps
/// after the pass is gone.
///
/// [`DveReduction`]: crate::preprocess::simplify::DveReduction
pub(crate) fn free_vars(fates: &[DveFate]) -> impl Iterator<Item = usize> + '_ {
    fates
        .iter()
        .enumerate()
        .filter(|(_, f)| matches!(f, DveFate::Free))
        .map(|(v, _)| v)
}

/// Result of DVE preprocessing: the reduced formula plus metadata needed to
/// re-introduce eliminated variables after compilation.
#[derive(Clone, Debug)]
pub(crate) struct DveResult {
    /// Reduced formula. Variables are renumbered 0..K-1 unless `keep_original_vars`.
    pub formula: CnfFormula,

    /// `definition_clauses[i]` = all clauses that mentioned the i-th
    /// eliminated variable, in elimination order — used to re-introduce
    /// variables in BVE mode after compilation.
    pub definition_clauses: Vec<Vec<Clause>>,

    /// What each variable of [`Self::formula`] is called in the space the pass
    /// was given, or `None` when the pass renumbered nothing — it eliminated
    /// no variable, handed the input formula back untouched, and its variables
    /// are already the input's.
    pub renumbering: Option<Renumber>,

    /// What became of each variable of the formula the pass was given, indexed
    /// by its id there. The one record of the elimination: every count and
    /// every fold below is read off it.
    pub fates: Vec<DveFate>,

    /// Wall time from this pass's construction through completed result assembly.
    pub elapsed_ms: u64,
}

impl DveResult {
    /// The result of a pass that eliminated nothing: the formula it was given,
    /// no definitions to re-introduce, and no renumbering, because the
    /// variables are still the caller's own.
    pub(crate) fn unchanged(formula: &CnfFormula, fates: Vec<DveFate>, elapsed_ms: u64) -> Self {
        DveResult {
            formula: formula.clone(),
            definition_clauses: Vec::new(),
            renumbering: None,
            fates,
            elapsed_ms,
        }
    }

    /// How many variables the pass was given.
    pub(crate) fn original_num_vars(&self) -> usize {
        self.fates.len()
    }

    pub(crate) fn num_defined(&self) -> usize {
        self.count(|f| matches!(f, DveFate::Defined))
    }

    pub(crate) fn num_equiv(&self) -> usize {
        self.count(|f| matches!(f, DveFate::Equiv { .. }))
    }

    /// Free variables, each contributing a factor of 2 to the model count.
    pub(crate) fn num_free(&self) -> usize {
        free_vars(&self.fates).count()
    }

    pub(crate) fn total_eliminated(&self) -> usize {
        self.count(|f| f.eliminated())
    }

    fn count(&self, pred: impl Fn(DveFate) -> bool) -> usize {
        self.fates.iter().filter(|&&f| pred(f)).count()
    }

    /// Check the two invariants relating [`Self::fates`] to the fields beside
    /// it, neither of which the types can carry on their own.
    pub(crate) fn debug_validate(&self) {
        if !cfg!(debug_assertions) {
            return;
        }
        let orig = self.original_num_vars();
        if let Some(renumbering) = &self.renumbering {
            for (new_id, &old_var) in renumbering.kept().iter().enumerate() {
                assert!(
                    old_var.idx() < orig,
                    "renumbering maps {} to {} but original_num_vars={}",
                    new_id,
                    old_var.0,
                    orig
                );
            }
        }
        let expected_defs = self.num_defined() + self.num_equiv();
        assert_eq!(
            self.definition_clauses.len(),
            expected_defs,
            "definition_clauses.len()={} but num_defined+num_equiv={}",
            self.definition_clauses.len(),
            expected_defs
        );
    }
}
