//! Main DVE preprocessing pipeline.

use crate::cnf::{Clause, CnfFormula, VarId};
use crate::diagnostics::diag;

use crate::preprocess::renumber::{Renumber, renumber_clauses};

use crate::cnf::occ;

use super::elim::{RoundStats, count_active_vars, dve_round, should_terminate_dve};
use super::oracle;
use super::strengthen::{FrozenEquiv, merge_equivalences, strengthen_clauses_with_meter};
use super::types::{DveFate, DveResult};

/// The longest one DVE round may run, whatever is left of the pass's budget.
/// A round that has not converged in this long is grinding, and the rounds
/// after it want their share of the clock.
const ROUND_MAX_MS: u64 = 60_000;

/// How many rounds may follow the oracle pass when it rewrote the clauses.
const ORACLE_EXTRA_ROUNDS: usize = 2;

/// The one wording of the budget-exhausted line.
fn budget_hit(time_limit_ms: u64, start: std::time::Instant) {
    diag!(
        "[dve-budget] HIT time_limit_ms={time_limit_ms}, elapsed_ms={}",
        start.elapsed().as_millis()
    );
}

/// Policy and variable-set context for one DVE pass.
pub(crate) struct DveConfig<'a> {
    pub max_rounds: usize,
    pub time_limit_ms: u64,
    pub keep_original_vars: bool,
    /// Whether elimination uses syntactic gate definitions
    /// ([`super::gates`]): to vouch for a probe candidate, to restrict a
    /// defined variable's resolution to its definition pairs, and to eliminate
    /// a gate output the probe did not return. Off, every pair is resolved and
    /// only the probe vouches.
    pub gate_definitions: bool,
    pub frozen: &'a rustc_hash::FxHashSet<VarId>,
    pub frozen_equiv: FrozenEquiv,
}

/// Entry point for the pipeline described in the module doc, for a caller with
/// no meter of its own: the pass is metered on a fresh wall clock and the
/// readings are dropped with it.
pub(crate) fn preprocess_dve(formula: &CnfFormula, config: DveConfig<'_>) -> DveResult {
    let mut meter =
        crate::preprocess::meter::PreprocessMeter::new(crate::config::PreprocessClock::WallClock);
    preprocess_dve_with_meter(formula, config, &mut meter)
}

/// [`preprocess_dve`] charging its work to the caller's meter.
pub(crate) fn preprocess_dve_with_meter(
    formula: &CnfFormula,
    config: DveConfig<'_>,
    meter: &mut crate::preprocess::meter::PreprocessMeter,
) -> DveResult {
    let DveConfig {
        max_rounds,
        time_limit_ms,
        keep_original_vars,
        gate_definitions,
        frozen,
        frozen_equiv,
    } = config;
    let num_vars = formula.num_vars() as usize;
    let time_limit_ms = meter
        .clamp(std::time::Duration::from_millis(time_limit_ms), None)
        .as_millis()
        .min(u64::MAX as u128) as u64;
    let mark = meter.begin(
        crate::bundle::PreprocessPhase::Dve,
        std::time::Duration::from_millis(time_limit_ms),
    );

    let mut run = DveRun::new(formula, time_limit_ms, frozen, mark, meter);
    if !formula.clauses().is_empty() && num_vars > 0 {
        run.rounds(max_rounds, gate_definitions, frozen_equiv);
    }
    run.finish(formula, keep_original_vars)
}

/// What the pass's rounds build up: the clause set as it stands, what each
/// variable's fate is so far, and the counts accumulated over every round.
/// [`DveRun::finish`] turns it into the [`DveResult`] the caller sees.
struct DveRun<'a> {
    num_vars: usize,
    clauses: Vec<Clause>,
    fates: Vec<DveFate>,
    total_dve_eliminated: usize,
    total_equiv_eliminated: usize,
    /// The variables no phase may eliminate, whatever the caller froze them for.
    frozen: &'a rustc_hash::FxHashSet<VarId>,
    /// The clock the rounds stop on.
    start: std::time::Instant,
    time_limit_ms: u64,
    mark: crate::preprocess::meter::PhaseMark,
    meter: &'a mut crate::preprocess::meter::PreprocessMeter,
    decisions: crate::preprocess::meter::DvePassDecisions,
    /// The clause count the pass started with, which the termination rule
    /// measures growth against.
    orig_clause_count: usize,
    /// How many variables the first round eliminated, which the termination
    /// rule measures diminishing returns against.
    round1_dve_elim: usize,
    /// Whether the oracle pass removed a literal or a clause. Without an
    /// elimination that is still a result the caller must get.
    rewritten: bool,
}

impl<'a> DveRun<'a> {
    fn new(
        formula: &CnfFormula,
        time_limit_ms: u64,
        frozen: &'a rustc_hash::FxHashSet<VarId>,
        mark: crate::preprocess::meter::PhaseMark,
        meter: &'a mut crate::preprocess::meter::PreprocessMeter,
    ) -> Self {
        let num_vars = formula.num_vars() as usize;
        DveRun {
            num_vars,
            clauses: formula.clauses().to_vec(),
            fates: vec![DveFate::Kept; num_vars],
            total_dve_eliminated: 0,
            total_equiv_eliminated: 0,
            frozen,
            start: std::time::Instant::now(),
            time_limit_ms,
            mark,
            meter,
            decisions: crate::preprocess::meter::DvePassDecisions::default(),
            orig_clause_count: formula.clauses().len(),
            round1_dve_elim: 0,
            rewritten: false,
        }
    }

    /// What is left of the budget, zero once it is spent.
    fn remaining_ms(&self) -> u64 {
        self.time_limit_ms
            .saturating_sub(self.meter.elapsed_ms(self.mark))
    }

    /// The same budget as an absolute instant, and the one derivation of it.
    ///
    /// [`Self::remaining_ms`] gates whether the next round starts. A step that
    /// polls no clock of its own — the CaDiCaL vivification round in
    /// `strengthen_clauses_with_meter` — has to be handed this instead, so it
    /// can be cut part-way through.
    fn stage_deadline(&self) -> Option<std::time::Instant> {
        self.meter.deadline_or_none(Some(
            self.start + std::time::Duration::from_millis(self.time_limit_ms),
        ))
    }

    /// The main loop, then the oracle pass: the rounds of
    /// [`Self::run_rounds`] up to `max_rounds`, then vivification and
    /// redundant-clause removal once. If that shortened or removed anything,
    /// at most [`ORACLE_EXTRA_ROUNDS`] more rounds run, since shorter clauses
    /// can make more variables eliminable.
    fn rounds(&mut self, max_rounds: usize, gate_definitions: bool, frozen_equiv: FrozenEquiv) {
        self.run_rounds(0..max_rounds, gate_definitions, frozen_equiv);
        if !oracle::enabled()
            || self.remaining_ms() == 0
            || crate::cnf::contains_empty_clause(&self.clauses)
        {
            return;
        }
        let window = oracle::Window {
            mark: self.mark,
            limit_ms: self.time_limit_ms,
            deadline: self.stage_deadline(),
        };
        match oracle::run(&mut self.clauses, self.num_vars, window, self.meter) {
            oracle::Outcome::Rewritten => {
                self.rewritten = true;
                if self.remaining_ms() > 0 {
                    self.run_rounds(
                        max_rounds..max_rounds + ORACLE_EXTRA_ROUNDS,
                        gate_definitions,
                        frozen_equiv,
                    );
                }
            }
            oracle::Outcome::Unchanged | oracle::Outcome::Unsatisfiable => {}
        }
    }

    /// Equivalence merging, one DVE round, then clause strengthening, for each
    /// round index in `rounds`, until the range, the budget or the termination
    /// rule ends it.
    fn run_rounds(
        &mut self,
        rounds: std::ops::Range<usize>,
        gate_definitions: bool,
        frozen_equiv: FrozenEquiv,
    ) {
        let orig_clause_count = self.orig_clause_count;

        for round in rounds {
            let remaining_ms = self.remaining_ms();
            if remaining_ms == 0 {
                budget_hit(self.time_limit_ms, self.start);
                self.decisions.budget_hit = true;
                break;
            }
            self.decisions.rounds += 1;

            let vars_before = count_active_vars(&self.clauses);
            let clauses_before = self.clauses.len();
            self.meter.charge_scan(|| {
                self.clauses
                    .iter()
                    .map(|clause| clause.literals.len())
                    .sum()
            });

            // --- Step 1: Equivalence merging (GPMC: MergeAdjEquivs) ---
            let equiv_elim = merge_equivalences(
                &mut self.clauses,
                self.num_vars,
                &mut self.fates,
                self.frozen,
                frozen_equiv,
            );
            self.total_equiv_eliminated += equiv_elim;

            // --- Step 2: the definability probe and elimination ---
            let round_limit = remaining_ms.min(ROUND_MAX_MS);
            let elimination = dve_round(
                &mut self.clauses,
                self.num_vars,
                &mut self.fates,
                round_limit,
                gate_definitions,
                self.frozen,
                self.meter,
            );
            let dve_elim = elimination.eliminated();
            self.total_dve_eliminated += dve_elim;
            self.decisions.budget_hit |= elimination.budget_hit;

            // --- Step 3: Clause strengthening (GPMC: Strengthen / vivification) ---
            // Skip when nothing was eliminated this round or the formula is too
            // small (under 50 clauses) for CaDiCaL's overhead to be worth it.
            let stage_deadline = self.stage_deadline();
            let strengthened = if (dve_elim >= 1 || equiv_elim >= 1) && self.clauses.len() >= 50 {
                strengthen_clauses_with_meter(
                    &mut self.clauses,
                    self.num_vars,
                    stage_deadline,
                    self.meter,
                )
            } else {
                false
            };

            let progress = equiv_elim > 0 || dve_elim > 0 || strengthened;

            if equiv_elim > 0 || dve_elim > 0 {
                diag!(
                    "[dve-round {}] {} equiv + {} dve eliminated ({} of {} proven defined, {} by gate definition), {} clauses{}",
                    round + 1,
                    equiv_elim,
                    dve_elim,
                    elimination.probed,
                    elimination.proven_defined,
                    elimination.by_gate,
                    self.clauses.len(),
                    if strengthened { " (strengthened)" } else { "" },
                );
            }

            if round == 0 {
                self.round1_dve_elim = dve_elim;
            }

            let vars_after = count_active_vars(&self.clauses);
            if should_terminate_dve(&RoundStats {
                round,
                dve_elim,
                equiv_elim,
                round1_dve_elim: self.round1_dve_elim,
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
            start,
            mark,
            meter,
            mut decisions,
            rewritten,
            ..
        } = self;

        let total_eliminated = total_dve_eliminated + total_equiv_eliminated;
        decisions.defined_eliminated = total_dve_eliminated;
        decisions.equivalence_eliminated = total_equiv_eliminated;
        if total_eliminated == 0 && !rewritten {
            let result = DveResult::unchanged(formula, fates, start.elapsed().as_millis() as u64);
            meter.record_dve(decisions);
            meter.finish_phase(mark);
            return result;
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
            let formula = CnfFormula::from_parts(num_vars as u32, clauses);
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
                format!("{} → {} vars", num_vars, result_formula.num_vars())
            },
            result_formula.clauses().len(),
        );

        let result = DveResult {
            formula: result_formula,
            renumbering: Some(renumbering),
            fates,
            elapsed_ms: start.elapsed().as_millis() as u64,
        };
        result.debug_validate();
        meter.record_dve(decisions);
        meter.finish_phase(mark);
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
