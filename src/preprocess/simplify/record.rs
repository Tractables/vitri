//! What a simplification produced: the reduced formula, what happened
//! to each original variable, and the lift that turns a count of the
//! reduced formula back into a count of the original.

use super::*;
use crate::preprocess::dve::types::{self, DveFate};

/// Measurements produced by the simplify chain's existing phase clocks.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SimplifyTelemetry {
    pub total_ms: u64,
    pub backbone_ms: Option<u64>,
    pub equivalence_ms: Option<u64>,
    pub dve_ms: Option<u64>,
    pub backbone_found: usize,
    pub backbone_probes: usize,
}

/// Result of unified simplification. Stores progressive preprocessing stages.
pub(crate) struct SimplifiedFormula {
    /// Original formula (after parsing, before any simplification).
    pub original: CnfFormula,

    /// After preprocessing + equivalence extraction.
    /// Equivalences are substituted: only representative vars remain in the formula.
    /// None if no preprocessing was run, no equivalences were found, or equiv
    /// reduction was skipped (e.g. when using a loaded vtree).
    pub equiv_reduced: Option<EquivReduction>,

    /// After definite variable elimination (only when the contract's
    /// [`StageSet`] carries a DVE budget — see [`SimplifyPurpose`]).
    /// Defined variables are eliminated permanently — they contribute factor 1,
    /// free variables contribute factor `2^num_free` to the model count.
    pub dve_reduced: Option<DveReduction>,

    /// The preprocessed formula (before equiv reduction).
    /// Used as compilation input when equiv_reduced is None but preprocessing ran.
    pub preprocessed: Option<CnfFormula>,

    /// What variable stripping produced, when it ran.
    pub stripped: Option<Stripped>,

    /// Work attempted by this invocation, including a DVE result later rejected
    /// by the meaningful-elimination or weighted-soundness gate.
    pub telemetry: SimplifyTelemetry,

    /// Deterministic control-flow decisions made by this simplify chain.
    pub decision_trace: Option<crate::bundle::PreprocessDecisionTrace>,
}

/// One variable stripping: the formula it left and the variables it took out.
/// Neither half means anything without the other — the formula is stated in a
/// variable space only the stripping can name — so they are one value.
pub(crate) struct Stripped {
    /// Formula after variable stripping but before equivalence reduction.
    /// Used for vtree construction when stripping ran but equiv reduction
    /// found no equivalences (or was skipped).
    pub formula: CnfFormula,

    /// Variables stripped before vtree construction (backbone + dead).
    /// Vtree and TDD must be expanded to restore these variables.
    pub removed: VariableStripping,
}

/// Result of variable stripping before vtree construction.
///
/// Two categories of variables are removed:
/// - **Backbone (forced)**: determined by unit clauses; contribute factor 1 to model count
/// - **Dead (eliminated)**: zero clause occurrences after preprocessing; contribute factor 2
///
/// Both are excluded from the primal graph to improve tree decomposition quality.
/// They are expanded back into the vtree and TDD after compilation.
pub(crate) struct VariableStripping {
    /// Forced literals: (original var, positive polarity).
    pub backbone: Vec<(VarId, bool)>,
    pub dead: Vec<VarId>,
    /// Original variable space → stripped variable space (only live vars kept).
    pub renumbering: Renumber,
}

/// Result of equivalence-based reduction.
pub(crate) struct EquivReduction {
    /// Formula with only representative variables (renumbered to contiguous IDs).
    pub formula: CnfFormula,
    /// Equivalence mapping: rep → equivs, var → rep, etc.
    pub mapping: EquivMapping,
    /// Stripped variable space → equiv-reduced space (only representatives kept).
    pub renumbering: Renumber,
}

/// Result of definite variable elimination.
///
/// DVE-eliminated variables are never re-introduced: they either are uniquely
/// determined by the remaining variables (contribute factor 1) or are free
/// (contribute factor `2^num_free`). Callers that build a TDD on `formula`
/// must multiply the model count by `2^num_free` to recover the count over
/// the pre-DVE variable space.
pub(crate) struct DveReduction {
    /// Formula after DVE elimination (renumbered to contiguous 0..K-1).
    pub formula: CnfFormula,
    /// DVE-input variable space → surviving space.
    pub renumbering: Renumber,
    /// What DVE did with each variable it was given, indexed by DVE-input id.
    /// The weighted path reads the [`DveFate::Equiv`] representative to FOLD the
    /// eliminated side's per-literal weights into it instead of reverting.
    pub fates: Vec<DveFate>,
}

impl DveReduction {
    /// Free variables — each contributes a factor of 2 to the model count.
    pub(crate) fn num_free(&self) -> usize {
        self.free_vars().count()
    }

    /// DVE-input ids of the variables DVE left free.
    pub(crate) fn free_vars(&self) -> impl Iterator<Item = usize> + '_ {
        types::free_vars(&self.fates)
    }
}

/// What preprocessing did to one ORIGINAL variable, as
/// [`SimplifiedFormula::original_fates`] reports it.
///
/// The three variants are exhaustive over a function-preserving run: a
/// variable either survives as some literal of a reduced variable, is fixed to a
/// constant, or is constrained by nothing at all. Reading an original assignment
/// off a reduced model is one lookup per original variable, with no chain to
/// follow — an equivalence partner reports the reduced index of its
/// REPRESENTATIVE, already resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OriginalFate {
    /// Equal to a literal of reduced variable `index` (0-based, in
    /// [`SimplifiedFormula::reduced_formula`]): the same literal when
    /// `same_polarity`, the negated one otherwise. A variable that survived
    /// untouched reports itself with `same_polarity: true`.
    Variable {
        /// 0-based variable index in the reduced formula.
        index: usize,
        /// `true` when the original equals that variable, `false` when it
        /// equals its negation.
        same_polarity: bool,
    },
    /// Fixed to a constant by preprocessing — a backbone literal. Every model of
    /// the original formula gives it this value.
    Forced(bool),
    /// Constrained by nothing: it occurs in no clause preprocessing kept, so
    /// both values extend every reduced model. These are the variables the
    /// `2^k` factor of [`count_lift_pow2`](SimplifiedFormula::count_lift_pow2)
    /// counts.
    Unconstrained,
}

impl SimplifiedFormula {
    /// Most reduced formula available.
    ///
    /// This must be consistent with the vtree: the formula a consumer takes
    /// its count over and the vtree built for it are the same variable space.
    pub(crate) fn reduced_formula(&self) -> &CnfFormula {
        if let Some(ref dve) = self.dve_reduced {
            return &dve.formula;
        }
        if let Some(ref eq) = self.equiv_reduced {
            &eq.formula
        } else if let Some(ref s) = self.stripped {
            &s.formula
        } else if let Some(ref pp) = self.preprocessed {
            pp
        } else {
            &self.original
        }
    }

    /// Exponent of the `2^k` count correction owed to *this formula's own*
    /// reductions: DVE free vars plus stripped dead vars. The count-preserving
    /// chain adds its caller-side exponent (Arjun's multiplier) on top via
    /// [`SimplifiedFormula::count_lift`].
    pub(crate) fn free_var_exp(&self) -> u32 {
        let dve_free = self.dve_reduced.as_ref().map(|d| d.num_free()).unwrap_or(0);
        let dead = self
            .stripped
            .as_ref()
            .map(|s| s.removed.dead.len())
            .unwrap_or(0);
        (dve_free + dead) as u32
    }

    /// The exponent of the `2^k` cardinality lift a chain publishes: this
    /// formula's own free-variable exponent plus `extra_pow2`, which is the
    /// Arjun stage's multiplier.
    ///
    /// Preprocessing removes variables in one count-affecting way: a free or
    /// unconstrained variable — DVE-free, stripped dead, Arjun-eliminated —
    /// doubles the count, and they are collected into this single factor. A
    /// determined variable (backbone, equivalence, DVE-defined) is factor 1 and
    /// does not appear here.
    pub(crate) fn count_lift_pow2(&self, extra_pow2: u32) -> u32 {
        self.free_var_exp() + extra_pow2
    }

    /// Stripped-space variable `sid` → index of the ORIGINAL variable it stands
    /// for. The identity when no stripping ran.
    ///
    /// This is the outer half of
    /// [`pre_dve_var_to_original`](Self::pre_dve_var_to_original) — callers
    /// that already hold a stripped-space id need exactly this half, without
    /// the equivalence peel in front of it.
    pub(crate) fn stripped_var_to_original(&self, sid: VarId) -> usize {
        match self.stripped.as_ref() {
            Some(s) => s.removed.renumbering.old_id(sid).idx(),
            None => sid.idx(),
        }
    }

    /// Pre-DVE variable index `j` (the equiv-reduced / stripped space) → index
    /// of the ORIGINAL variable it stands for.
    ///
    /// THE composition of the two id-renumbering stages, in the order they
    /// run: equivalence reduction renumbers into the *stripped* space, and
    /// stripping renumbers into the *original* space — one composition, not
    /// one per call site.
    pub(crate) fn pre_dve_var_to_original(&self, j: usize) -> usize {
        self.stripped_var_to_original(self.peel_equiv(VarId(j as u32)))
    }

    /// DVE-input variable → the stripped-space id it stands for: the first of
    /// the two renumbering stages undone, and only that. With no equivalence
    /// reduction the two spaces are the same one.
    fn peel_equiv(&self, j: VarId) -> VarId {
        match self.equiv_reduced.as_ref() {
            Some(eq) => eq.renumbering.old_id(j),
            None => j,
        }
    }

    /// `frozen` — ORIGINAL variable indices — translated into the DVE-INPUT
    /// space, the equiv-reduced / stripped space DVE is handed.
    ///
    /// A DVE-input variable is frozen when the original variable its
    /// representative stands for is frozen, OR when any equivalence partner
    /// folded onto that representative is. The partner half is what the raw
    /// representative misses: an equal-weight representative that absorbed an
    /// unequal-weight partner carries the partner's weight after the fold, so
    /// eliminating it would be non-scalar all the same.
    pub(crate) fn frozen_in_dve_space(
        &self,
        frozen: &rustc_hash::FxHashSet<VarId>,
        dve_input_num_vars: u32,
    ) -> rustc_hash::FxHashSet<VarId> {
        let mut out = rustc_hash::FxHashSet::default();
        if frozen.is_empty() {
            return out;
        }
        for j in 0..dve_input_num_vars {
            let rep_s = self.peel_equiv(VarId(j));
            let mut is_frozen =
                frozen.contains(&VarId(self.stripped_var_to_original(rep_s) as u32));
            if !is_frozen
                && let Some(eq) = self.equiv_reduced.as_ref()
                && let Some(partners) = eq.mapping.rep_to_equivs.get(&rep_s)
            {
                is_frozen = partners
                    .iter()
                    .any(|&p| frozen.contains(&VarId(self.stripped_var_to_original(p.var) as u32)));
            }
            if is_frozen {
                out.insert(VarId(j));
            }
        }
        out
    }

    /// [`reduced_formula`](Self::reduced_formula) variable index `i` → index of the
    /// ORIGINAL variable it stands for: peel the DVE layer (survivor → DVE-input
    /// id), then apply [`pre_dve_var_to_original`](Self::pre_dve_var_to_original).
    /// Without DVE the two agree.
    pub(crate) fn reduced_var_to_original(&self, i: usize) -> usize {
        let j = match self.dve_reduced.as_ref() {
            Some(d) => d.renumbering.old_id(VarId(i as u32)).idx(),
            None => i,
        };
        self.pre_dve_var_to_original(j)
    }

    /// What STRIPPING removed, in the ORIGINAL space and in the spelling
    /// [`crate::bundle::PreprocessRecord`] publishes: the forced literals as
    /// signed DIMACS, the dead (unconstrained) variables as 1-based ids. Empty
    /// when no stripping ran.
    pub(crate) fn stripped_forced_and_free(&self) -> (Vec<i32>, Vec<u32>) {
        match self.stripped.as_ref() {
            Some(s) => (
                s.removed
                    .backbone
                    .iter()
                    .map(|&(v, pos)| Literal::new(v, pos).to_dimacs())
                    .collect(),
                s.removed
                    .dead
                    .iter()
                    .map(|v| v.to_dimacs() as u32)
                    .collect(),
            ),
            None => (Vec::new(), Vec::new()),
        }
    }

    /// The reduced→original map for a chain that ran NO renumbering stage after
    /// this one: every [`reduced_formula`](Self::reduced_formula) index maps to the
    /// original variable it stands for, and no entry can be missing. A chain
    /// with a later stage composes that stage's map on top of this one instead.
    pub(crate) fn composed_var_map(
        &self,
    ) -> crate::preprocess::VarMap<crate::cnf::Reduced, crate::cnf::Original> {
        (0..self.reduced_formula().num_vars as usize)
            .map(|j| Some(VarId(self.reduced_var_to_original(j) as u32).to_dimacs()))
            .collect()
    }

    /// What became of every ORIGINAL variable, in original-variable order:
    /// `original_fates()[o]` describes 0-based original variable `o`, and the
    /// vector covers the whole original variable space.
    ///
    /// The TOTAL inverse of [`reduced_var_to_original`](Self::reduced_var_to_original),
    /// and the only direction in which a variable preprocessing dropped can still
    /// be named. Composed from the same two renumbering stages, read the other
    /// way: stripping says which variables left and why, and the equivalence
    /// reduction says which representative each survivor folded onto.
    ///
    /// **Totality has a precondition**: no count-only stage may have run. Gate
    /// detection and DVE remove a variable determined by a FUNCTION of the
    /// survivors, and no [`OriginalFate`] can name that, so only the
    /// function-preserving contract ([`SimplifyPurpose::Function`]) asks.
    pub(crate) fn original_fates(&self) -> Vec<OriginalFate> {
        debug_assert!(
            self.dve_reduced.is_none(),
            "a DVE-eliminated variable is determined by a definition, which no OriginalFate names",
        );
        let n = self.original.num_vars as usize;
        // Whatever the two loops below do not name was stripped as dead: it
        // occurs in no clause of the reduced formula and is fixed by nothing.
        let mut fates = vec![OriginalFate::Unconstrained; n];
        // A variable of the STRIPPED space, named in `reduced_formula()`: its
        // equivalence representative when the reduction ran, itself otherwise.
        let in_best = |sid: VarId| match self.equiv_reduced.as_ref() {
            Some(eq) => {
                let rep = eq.mapping.var_to_rep[sid.idx()];
                let index = eq
                    .renumbering
                    .new_id(rep.var)
                    .expect("an equivalence representative survives into the reduced formula")
                    .idx();
                OriginalFate::Variable {
                    index,
                    same_polarity: rep.positive,
                }
            }
            None => OriginalFate::Variable {
                index: sid.idx(),
                same_polarity: true,
            },
        };
        match self.stripped.as_ref() {
            Some(s) => {
                for &(var, positive) in &s.removed.backbone {
                    fates[var.idx()] = OriginalFate::Forced(positive);
                }
                for (sid, &original) in s.removed.renumbering.kept().iter().enumerate() {
                    fates[original.idx()] = in_best(VarId(sid as u32));
                }
            }
            // Preprocessing preserves the variable count, so with no stripping
            // the pre-equivalence space IS the original space.
            None => {
                for (o, fate) in fates.iter_mut().enumerate() {
                    *fate = in_best(VarId(o as u32));
                }
            }
        }
        fates
    }

    /// Promote one backbone variable to live when stripping eliminated every
    /// variable. A record that still has a live variable is left as it is.
    ///
    /// Vtree construction requires ≥1 var; the trivial 1-var unit-clause
    /// formula compiles instantly and the record's forced literals account
    /// for the rest.
    pub(crate) fn promote_all_backbone_to_live(&mut self) {
        let Some(s) = self.stripped.as_mut() else {
            return;
        };
        if s.removed.renumbering.num_new_vars() != 0 {
            return;
        }
        let (live_var, live_polarity) = s.removed.backbone.remove(0);
        s.removed.renumbering = Renumber::of_kept(s.removed.renumbering.num_old_vars(), [live_var]);
        s.formula = CnfFormula {
            num_vars: 1,
            clauses: vec![Clause::new(vec![Literal::new(VarId(0), live_polarity)])],
        };
        // reduced_formula() must see the new 1-var stripped formula, not the
        // 0-var equiv reduction.
        self.equiv_reduced = None;
    }
}
