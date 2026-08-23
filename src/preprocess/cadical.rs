//! CaDiCaL-based preprocessing.
//!
//! Freezes all variables to prevent BVE/BCE (unsafe for model counting),
//! then runs CaDiCaL's simplify() which performs equivalence-preserving
//! techniques: subsumption, vivification, failed literal probing,
//! self-subsuming resolution, backbone detection.

use std::time::{Duration, Instant};

use super::cadical_ffi::{Bounded, CaDiCal, ClauseIterator, Terminator, note_solver_unavailable};
use super::renumber::Renumber;
use crate::cnf::occ;

use crate::cnf::VarId;
use crate::cnf::{Clause, CnfFormula, Literal};

/// Terminator that fires after a wall-clock deadline. Handed to
/// [`Bounded`] around `simplify()` (or around an incremental `solve()` loop) so
/// a runaway pass returns control instead of hanging indefinitely.
pub(super) struct WallClockTerminator {
    deadline: Instant,
}

impl WallClockTerminator {
    pub(super) fn new(budget: Duration) -> Self {
        Self {
            deadline: Instant::now() + budget,
        }
    }
}

impl Terminator for WallClockTerminator {
    fn terminated(&mut self) -> bool {
        Instant::now() >= self.deadline
    }
}

/// Returns the simplified formula (same `num_vars`) and the number of forced
/// variables. Forced variables are included as unit clauses in the output.
///
/// `deadline` is the whole-run wall-clock deadline derived from the caller's
/// budget; when `Some`, the CaDiCaL simplify pass is bounded by a
/// `WallClockTerminator` whose ceiling is the budget remaining at this call — a
/// phase ceiling must never outlive the run's. `solve()` honors the terminator
/// strictly; `simplify()`'s inprocessing loops poll it coarsely (between
/// rounds/phases), so the bound is best-effort but prevents a runaway pass from
/// hanging past the deadline. `None` = unbounded.
pub(super) fn preprocess_cadical(
    formula: &CnfFormula,
    rounds: i32,
    deadline: Option<Instant>,
) -> (CnfFormula, usize) {
    // Clamp to the budget remaining now; a past deadline yields a zero budget that
    // terminates the pass immediately (never below the remaining budget).
    let budget = deadline.map(crate::budget::remaining);
    preprocess_cadical_budgeted(formula, rounds, budget)
}

/// Core freeze-only CaDiCaL preprocessing on `formula`, freezing every variable
/// flagged in `appears` (frozen vars are never BVE/BCE-eliminated, which is what
/// preserves model count). Returns the simplified clauses and the forced literals
/// in `formula`'s OWN variable space; forced literals are NOT yet appended as
/// unit clauses (the caller does that after any var-space expansion).
///
/// `None` when no solver could be allocated — the pass then has nothing to
/// report, which the caller reads as "no simplification".
fn cadical_freeze_run(
    formula: &CnfFormula,
    appears: &[bool],
    rounds: i32,
    budget: Option<Duration>,
) -> Option<(Vec<Clause>, Vec<Literal>)> {
    let num_vars = formula.num_vars;

    let mut solver = CaDiCal::new()?;

    // Add all clauses (1-indexed, DIMACS-style, terminated by 0).
    for clause in &formula.clauses {
        for lit in &clause.literals {
            solver.add(lit.to_dimacs());
        }
        solver.add(0);
    }

    for var_idx in 0..num_vars {
        if appears[var_idx as usize] {
            solver.freeze(VarId(var_idx).to_dimacs());
        }
    }

    // The guard is a temporary: the terminator is connected for the simplify
    // call and disconnected the moment the statement ends, before anything
    // below reads the solver back.
    let _status = match budget {
        Some(b) => Bounded::new(&mut solver, WallClockTerminator::new(b)).simplify(rounds),
        None => solver.simplify(rounds),
    };

    let mut forced_vars = Vec::new();
    for var_idx in 0..num_vars {
        if !appears[var_idx as usize] {
            continue;
        }
        let var = VarId(var_idx);
        let v = solver.fixed(var.to_dimacs());
        if v != 0 {
            forced_vars.push(Literal::new(var, v > 0));
        }
    }

    let mut collector = ClauseCollector {
        clauses: Vec::new(),
    };
    solver.traverse_clauses(&mut collector);

    let clauses: Vec<Clause> = collector
        .clauses
        .into_iter()
        .map(|dimacs_lits| Clause::new(dimacs_lits.into_iter().map(Literal::from).collect()))
        .collect();

    Some((clauses, forced_vars))
}

/// Same as `preprocess_cadical` but with an explicit optional wall-clock budget.
///
/// Variable compaction: huge formulas (e.g. feature models with millions of
/// zero-occurrence vars) make CaDiCaL allocate per-variable structures up to the
/// max declared var id and loop over all of them, even when only a tiny fraction
/// occur. When occurring vars are sparse, we renumber them to a contiguous `0..K`
/// space, run CaDiCaL there, and expand the result back to the original var ids.
/// This is TRANSPARENT — every appearing var is still frozen, so model count is
/// preserved identically; only CaDiCaL's internal allocation/iteration shrinks.
/// Output clauses and forced literals are bit-identical to the uncompacted path.
pub(super) fn preprocess_cadical_budgeted(
    formula: &CnfFormula,
    rounds: i32,
    budget: Option<Duration>,
) -> (CnfFormula, usize) {
    let num_vars = formula.num_vars;

    if formula.clauses.is_empty() {
        return (formula.clone(), 0);
    }

    let appears = occ::appearance_mask(&formula.clauses, num_vars as usize);
    let n_appear = appears.iter().filter(|&&a| a).count() as u32;

    // Forced literals (in ORIGINAL var space) and simplified clauses (already in
    // original var space) produced by either the direct or the compacted path.
    let run: Option<(Vec<Clause>, Vec<Literal>)> = if n_appear == num_vars {
        // No (worthwhile) gap between declared and occurring vars — run CaDiCaL
        // directly on the original formula.
        cadical_freeze_run(formula, &appears, rounds, budget)
    } else {
        // Sparse occurrence: renumber occurring vars to a contiguous space.
        // Every variable of a clause occurs in it, so nothing is ever
        // dropped here — the rewrite is a pure remap, which is what keeps
        // the compacted path bit-identical to the uncompacted one.
        let compaction = Renumber::keeping(num_vars as usize, |v| appears[v.idx()]);
        let compact_nv = compaction.num_new_vars();

        let compact_clauses: Vec<Clause> = formula
            .clauses
            .iter()
            .map(|c| {
                Clause::new(
                    c.literals
                        .iter()
                        .filter_map(|l| compaction.apply_lit(*l))
                        .collect(),
                )
            })
            .collect();
        let compact_formula = CnfFormula {
            num_vars: compact_nv,
            clauses: compact_clauses,
        };

        let compact_appears = vec![true; compact_nv as usize];
        cadical_freeze_run(&compact_formula, &compact_appears, rounds, budget).map(
            |(compact_clauses, compact_forced)| {
                let clauses = compact_clauses
                    .into_iter()
                    .map(|c| {
                        Clause::new(
                            c.literals
                                .iter()
                                .map(|l| compaction.apply_inverse_lit(*l))
                                .collect(),
                        )
                    })
                    .collect();
                let forced = compact_forced
                    .iter()
                    .map(|l| compaction.apply_inverse_lit(*l))
                    .collect();
                (clauses, forced)
            },
        )
    };

    let Some((mut clauses, forced_orig)) = run else {
        note_solver_unavailable("cadical", "the formula is left unsimplified");
        return (formula.clone(), 0);
    };

    let forced_count = forced_orig.len();

    for lit in forced_orig {
        clauses.push(Clause::new(vec![lit]));
    }

    (CnfFormula { num_vars, clauses }, forced_count)
}

struct ClauseCollector {
    clauses: Vec<Vec<i32>>,
}

impl ClauseIterator for ClauseCollector {
    fn clause(&mut self, clause: &[i32]) -> bool {
        self.clauses.push(clause.to_vec());
        true
    }
}
