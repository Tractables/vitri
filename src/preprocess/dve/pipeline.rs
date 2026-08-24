//! Main DVE preprocessing pipeline.

use crate::cnf::{Clause, CnfFormula, VarId};
use crate::diagnostics::diag;

use crate::preprocess::renumber::{Renumber, renumber_clauses};

use super::definability::pick_def_vars;
use crate::cnf::occ;

use super::elim::{
    RoundStats, apply_elimination, count_active_vars, dve_round, should_terminate_dve,
};
use super::strengthen::{EquivState, FrozenEquiv, merge_equivalences, strengthen_clauses};
use super::types::{DveFate, DveResult};

/// The one wording of the budget-exhausted line. Both elimination loops stop on
/// the same clock, so they say so the same way.
fn budget_hit(time_limit_ms: u64, start: std::time::Instant) {
    diag!(
        "[dve-budget] HIT time_limit_ms={time_limit_ms}, elapsed_ms={}",
        start.elapsed().as_millis()
    );
}

/// Entry point for the pipeline described in the module doc.
pub(crate) fn preprocess_dve(
    formula: &CnfFormula,
    max_rounds: usize,
    time_limit_ms: u64,
    keep_original_vars: bool,
    known_defined: &rustc_hash::FxHashSet<VarId>,
    frozen: &rustc_hash::FxHashSet<VarId>,
    frozen_equiv: FrozenEquiv,
) -> DveResult {
    let num_vars = formula.num_vars as usize;

    if formula.clauses.is_empty() || num_vars == 0 {
        return DveResult::unchanged(formula, vec![DveFate::Kept; num_vars]);
    }

    let mut run = DveRun::new(formula, time_limit_ms, frozen);
    run.rounds(max_rounds, known_defined, frozen_equiv);
    run.aggressive_cascade();
    run.finish(formula, keep_original_vars)
}

/// What the pass's two elimination loops build up between them: the clause set
/// as it stands, what each variable's fate is so far, and the definitions and
/// counts accumulated over every round of both. [`DveRun::finish`] turns it
/// into the [`DveResult`] the caller sees.
struct DveRun<'a> {
    num_vars: usize,
    clauses: Vec<Clause>,
    fates: Vec<DveFate>,
    total_dve_eliminated: usize,
    total_equiv_eliminated: usize,
    /// One entry per resolved-away variable, in elimination order. The
    /// equivalence merges accumulate separately and are appended in `finish`,
    /// so the two orders never interleave: re-introducing a definition
    /// undoes one elimination, and an elimination is only undoable against the
    /// clause set its own phase saw. Both steps hand back the same shape, and
    /// the two land in different bags on purpose.
    all_definition_clauses: Vec<Vec<Clause>>,
    all_equiv_definition_clauses: Vec<Vec<Clause>>,
    /// The variables no phase may eliminate, whatever the caller froze them for.
    frozen: &'a rustc_hash::FxHashSet<VarId>,
    /// The one clock both loops stop on.
    start: std::time::Instant,
    time_limit_ms: u64,
}

impl<'a> DveRun<'a> {
    fn new(
        formula: &CnfFormula,
        time_limit_ms: u64,
        frozen: &'a rustc_hash::FxHashSet<VarId>,
    ) -> Self {
        let num_vars = formula.num_vars as usize;
        DveRun {
            num_vars,
            clauses: formula.clauses.clone(),
            fates: vec![DveFate::Kept; num_vars],
            total_dve_eliminated: 0,
            total_equiv_eliminated: 0,
            all_definition_clauses: Vec::new(),
            all_equiv_definition_clauses: Vec::new(),
            frozen,
            start: std::time::Instant::now(),
            time_limit_ms,
        }
    }

    /// What is left of the budget, zero once it is spent.
    fn remaining_ms(&self) -> u64 {
        self.time_limit_ms
            .saturating_sub(self.start.elapsed().as_millis() as u64)
    }

    /// The same budget as an absolute instant, and the one derivation of it.
    ///
    /// [`Self::remaining_ms`] gates whether the next round starts. A step that
    /// polls no clock of its own — the CaDiCaL vivification round in
    /// `strengthen_clauses` — has to be handed this instead, so it can be cut
    /// part-way through.
    fn stage_deadline(&self) -> std::time::Instant {
        self.start + std::time::Duration::from_millis(self.time_limit_ms)
    }

    /// The main loop: equivalence merging, one DVE round, then clause
    /// strengthening, until `max_rounds`, the budget or the termination rule
    /// ends it.
    fn rounds(
        &mut self,
        max_rounds: usize,
        known_defined: &rustc_hash::FxHashSet<VarId>,
        frozen_equiv: FrozenEquiv,
    ) {
        let orig_clause_count = self.clauses.len();
        // representative[v]: sign encodes polarity — positive means same polarity
        // as the representative, negative means flipped.
        let mut representative: Vec<i32> = (0..self.num_vars as i32).collect();
        let mut round1_dve_elim = 0usize;

        // SOUNDNESS: caller-supplied `known_defined` reflects gates detected on the
        // *original* formula. Equivalence merging in any round can substitute input
        // vars of those gates and turn gate-defining clauses into tautologies (then
        // dropped), so the gate output is no longer functionally defined in the
        // current `clauses` even though it's still listed as "preknown defined".
        // Eliminating it via Boolean resolution would corrupt the model count.
        // See regression test `dve_equiv_followed_by_gate_elim_preserves_mc`.
        let mut current_known_defined: rustc_hash::FxHashSet<VarId> = known_defined.clone();
        current_known_defined.retain(|v| !self.frozen.contains(v));

        for round in 0..max_rounds {
            let remaining_ms = self.remaining_ms();
            if remaining_ms == 0 {
                budget_hit(self.time_limit_ms, self.start);
                break;
            }

            let vars_before = count_active_vars(&self.clauses);
            let clauses_before = self.clauses.len();

            // --- Step 1: Equivalence merging (GPMC: MergeAdjEquivs) ---
            let equivs = merge_equivalences(
                &mut self.clauses,
                self.num_vars,
                &mut EquivState {
                    fates: &mut self.fates,
                    representative: &mut representative,
                },
                self.frozen,
                frozen_equiv,
            );
            let equiv_elim = equivs.eliminated;
            self.total_equiv_eliminated += equiv_elim;
            self.all_equiv_definition_clauses.extend(equivs.definitions);

            if equiv_elim > 0 {
                let temp = CnfFormula {
                    num_vars: self.num_vars as u32,
                    clauses: self.clauses.clone(),
                };
                current_known_defined = super::super::gates::detect_gates(&temp).eliminated;
                current_known_defined.retain(|v| !self.frozen.contains(v));
            }

            // --- Step 2: DVE round (GPMC: VariableEliminate with dve=true) ---
            let round_limit = remaining_ms.min(60_000);
            let dve = dve_round(
                &mut self.clauses,
                self.num_vars,
                &mut self.fates,
                round_limit,
                &current_known_defined,
                self.frozen,
            );
            let dve_elim = dve.eliminated;
            self.total_dve_eliminated += dve_elim;
            self.all_definition_clauses.extend(dve.definitions);

            // --- Step 3: Clause strengthening (GPMC: Strengthen / vivification) ---
            // Skip when nothing was eliminated this round or the formula is too
            // small (under 50 clauses) for CaDiCaL's overhead to be worth it.
            let stage_deadline = self.stage_deadline();
            let strengthened = if (dve_elim >= 1 || equiv_elim >= 1) && self.clauses.len() >= 50 {
                strengthen_clauses(&mut self.clauses, self.num_vars, Some(stage_deadline))
            } else {
                false
            };

            let progress = equiv_elim > 0 || dve_elim > 0 || strengthened;

            if equiv_elim > 0 || dve_elim > 0 {
                diag!(
                    "[dve-round {}] {} equiv + {} dve eliminated, {} clauses{}",
                    round + 1,
                    equiv_elim,
                    dve_elim,
                    self.clauses.len(),
                    if strengthened { " (strengthened)" } else { "" },
                );
            }

            if round == 0 {
                round1_dve_elim = dve_elim;
            }

            let vars_after = count_active_vars(&self.clauses);
            if should_terminate_dve(&RoundStats {
                round,
                dve_elim,
                equiv_elim,
                round1_dve_elim,
                vars_before,
                vars_after,
                clauses_before,
                clauses_after: self.clauses.len(),
                orig_clause_count,
                progress,
            }) {
                break;
            }
        }
    }

    /// Aggressive cascade: the main DVE loop's `is_ve_candidate` filter misses
    /// defined vars that pass the definability test but aren't resolvent-bounded
    /// (pos*neg greater than pos+neg). This phase drops that filter, running
    /// `pick_def_vars` on every remaining var and eliminating the
    /// resolvent-bounded subset.
    ///
    /// Only fires on small residual formulas — for larger ones the extra SAT
    /// calls and strengthen overhead outweigh the marginal elimination gained.
    fn aggressive_cascade(&mut self) {
        const AGGRESSIVE_MAX_VARS: usize = 700;
        const AGGRESSIVE_MAX_CLAUSES: usize = 5000;
        loop {
            let remaining_ms = self.remaining_ms();
            if remaining_ms <= 500 {
                budget_hit(self.time_limit_ms, self.start);
                break;
            }

            let appears = occ::appearance_mask(&self.clauses, self.num_vars);
            let all_candidates: Vec<u32> = (0..self.num_vars)
                .filter(|&v| {
                    !self.fates[v].eliminated()
                        && appears[v]
                        && !self.frozen.contains(&VarId(v as u32))
                })
                .map(|v| v as u32)
                .collect();
            if all_candidates.is_empty() {
                break;
            }
            if all_candidates.len() > AGGRESSIVE_MAX_VARS
                || self.clauses.len() > AGGRESSIVE_MAX_CLAUSES
            {
                break;
            }

            let defined =
                pick_def_vars(&self.clauses, self.num_vars, &all_candidates, remaining_ms);
            if defined.is_empty() {
                break;
            }

            let n_defined = defined.len();
            let max_clauses = self.clauses.len();
            let cascade = apply_elimination(
                &mut self.clauses,
                &defined,
                &mut self.fates,
                max_clauses,
                self.frozen,
            );
            let elim_count = cascade.eliminated;
            self.all_definition_clauses.extend(cascade.definitions);
            self.total_dve_eliminated += elim_count;

            diag!(
                "[dve-aggressive] {} vars eliminated (of {} defined found), {} clauses",
                elim_count,
                n_defined,
                self.clauses.len(),
            );

            if elim_count == 0 {
                // No resolvent-bounded defined vars remain — true fixpoint.
                break;
            }

            // Only strengthen when some defined vars weren't resolvent-bounded — it
            // may lower frequencies enough to unlock them next iteration. Skipping
            // it when everything was already eliminated avoids reshaping the
            // residual formula in ways that hurt vtree quality for no gain.
            if elim_count < n_defined {
                let stage_deadline = self.stage_deadline();
                strengthen_clauses(&mut self.clauses, self.num_vars, Some(stage_deadline));
            }
        }
    }

    /// Name the variables no surviving clause mentions as free, renumber the
    /// residual as the caller asked, and report. `formula` is the pass's own
    /// input, handed back untouched when nothing was eliminated.
    fn finish(self, formula: &CnfFormula, keep_original_vars: bool) -> DveResult {
        let DveRun {
            num_vars,
            clauses,
            mut fates,
            total_dve_eliminated,
            total_equiv_eliminated,
            mut all_definition_clauses,
            all_equiv_definition_clauses,
            ..
        } = self;

        let total_eliminated = total_dve_eliminated + total_equiv_eliminated;
        if total_eliminated == 0 {
            return DveResult::unchanged(formula, fates);
        }

        let appears = occ::appearance_mask(&clauses, num_vars);
        let mut num_free = 0;
        for v in 0..num_vars {
            if !fates[v].eliminated() && !appears[v] {
                fates[v] = DveFate::Free;
                num_free += 1;
            }
        }

        let (result_formula, renumbering) = if keep_original_vars {
            // Even though IDs aren't renumbered here, the adaptive vtree mode may
            // still switch to vtree-post and needs this renumbering then.
            let renumbering = Renumber::keeping(num_vars, |v| !fates[v.idx()].eliminated());
            let formula = CnfFormula {
                num_vars: num_vars as u32,
                clauses,
            };
            (formula, renumbering)
        } else {
            renumber_formula(&fates, num_vars, clauses)
        };

        // Keeping the original ids leaves one variable count to report rather
        // than two, and calls what resolution removed by the name the caller's
        // own space still knows it under.
        diag!(
            "[dve-total] {} {} + {} equiv + {} free eliminated, {}, {} clauses",
            total_dve_eliminated,
            if keep_original_vars { "dve" } else { "defined" },
            total_equiv_eliminated,
            num_free,
            if keep_original_vars {
                format!("{num_vars} vars (original IDs)")
            } else {
                format!("{} → {} vars", num_vars, result_formula.num_vars)
            },
            result_formula.clauses.len(),
        );

        all_definition_clauses.extend(all_equiv_definition_clauses);
        let result = DveResult {
            formula: result_formula,
            definition_clauses: all_definition_clauses,
            renumbering: Some(renumbering),
            fates,
        };
        result.debug_validate();
        result
    }
}

/// The keep predicate the DVE pipeline reads off a variable's fate; callers
/// don't each write it out.
pub(super) fn renumber_formula(
    fates: &[DveFate],
    num_vars: usize,
    clauses: Vec<Clause>,
) -> (CnfFormula, Renumber) {
    renumber_clauses(num_vars, clauses, |v| !fates[v.idx()].eliminated())
}
