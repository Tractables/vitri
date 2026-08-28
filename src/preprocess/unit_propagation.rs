//! Unit propagation (Boolean Constraint Propagation) to fixpoint.
//!
//! This is the EQUIVALENCE-preserving BCP — not to be confused with
//! `dve::elim::propagate_forced`, the COUNT-preserving flavor (which retains
//! a forced show var's unit clause instead of propagating it away). The two
//! are not interchangeable: this one has no frozen/show concept.

use crate::cnf::occ;
use crate::cnf::{Clause, Literal};

enum AssignResult {
    Fresh,
    Redundant,
    Conflict,
}

fn try_assign(
    assignment: &mut [Option<bool>],
    forced: &mut Vec<Literal>,
    lit: Literal,
) -> AssignResult {
    let slot = &mut assignment[lit.var.0 as usize];
    match *slot {
        None => {
            *slot = Some(lit.positive);
            forced.push(lit);
            AssignResult::Fresh
        }
        Some(v) if v == lit.positive => AssignResult::Redundant,
        Some(_) => AssignResult::Conflict,
    }
}

/// Build the canonical UNSAT return: single empty clause, preserving the forced list.
fn unsat(forced: Vec<Literal>) -> (Vec<Clause>, Vec<Literal>) {
    (vec![Clause::new(vec![])], forced)
}

/// Propagate unit clauses to fixpoint. Returns `(simplified_clauses,
/// forced_literals)`; UNSAT is signaled by a single empty clause present in
/// `simplified_clauses`.
pub(crate) fn propagate(clauses: &[Clause], num_vars: u32) -> (Vec<Clause>, Vec<Literal>) {
    // The input is already refuted. Canonicalize before allocating occurrence
    // tables or collecting units: no assignment list adds information to false.
    if clauses.iter().any(|clause| clause.is_empty()) {
        return unsat(Vec::new());
    }

    let n = num_vars as usize;

    let mut assignment: Vec<Option<bool>> = vec![None; n];
    let mut forced: Vec<Literal> = Vec::new();

    let mut working: Vec<Option<Vec<Literal>>> =
        clauses.iter().map(|c| Some(c.literals.clone())).collect();

    let (mut pos_occ, mut neg_occ) = occ::occurrence_lists(clauses, n);

    let mut queue: Vec<Literal> = Vec::new();

    for lits in working.iter().flatten() {
        if lits.len() == 1 {
            let lit = lits[0];
            match try_assign(&mut assignment, &mut forced, lit) {
                AssignResult::Fresh => queue.push(lit),
                AssignResult::Redundant => {}
                AssignResult::Conflict => return unsat(forced),
            }
        }
    }

    while let Some(lit) = queue.pop() {
        let var = lit.var.0 as usize;

        let satisfied = if lit.positive {
            std::mem::take(&mut pos_occ[var])
        } else {
            std::mem::take(&mut neg_occ[var])
        };
        for ci in satisfied {
            if working[ci].is_some() {
                working[ci] = None;
            }
        }

        let shortened = if lit.positive {
            std::mem::take(&mut neg_occ[var])
        } else {
            std::mem::take(&mut pos_occ[var])
        };
        for ci in shortened {
            let clause_lits = match working[ci].as_mut() {
                Some(lits) => lits,
                None => continue,
            };

            clause_lits.retain(|l| l.var != lit.var);

            if clause_lits.is_empty() {
                return (vec![Clause::new(vec![])], forced);
            }

            if clause_lits.len() == 1 {
                let new_lit = clause_lits[0];
                match try_assign(&mut assignment, &mut forced, new_lit) {
                    AssignResult::Fresh => queue.push(new_lit),
                    AssignResult::Redundant => {}
                    AssignResult::Conflict => return unsat(forced),
                }
            }
        }
    }

    // Excludes a residual unit clause for an already-forced variable; forced
    // literals are reported separately for the caller to handle.
    let result: Vec<Clause> = working
        .into_iter()
        .filter_map(|slot| slot.map(Clause::new))
        .filter(|c| {
            if c.literals.len() == 1 {
                let l = c.literals[0];
                assignment[l.var.0 as usize].is_none()
            } else {
                true
            }
        })
        .collect();

    (result, forced)
}
