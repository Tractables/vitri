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
    let slot = &mut assignment[lit.var.idx()];
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

    // Each variable is assigned at most once, so each literal's clauses are
    // visited at most once and the lists are only ever read.
    let occurrences = occ::LiteralOccurrences::of(clauses, n);

    let mut queue: Vec<Literal> = Vec::new();

    for lits in working.iter().flatten() {
        if let &[lit] = lits.as_slice()
            && !enqueue_unit(&mut assignment, &mut forced, &mut queue, lit)
        {
            return unsat(forced);
        }
    }

    while let Some(lit) = queue.pop() {
        for &ci in occurrences.of_literal(lit) {
            working[ci] = None;
        }
        for &ci in occurrences.of_literal(lit.negated()) {
            let Some(clause_lits) = working[ci].as_mut() else {
                continue;
            };
            clause_lits.retain(|l| l.var != lit.var);
            if clause_lits.is_empty() {
                return unsat(forced);
            }
            if let &[new_lit] = clause_lits.as_slice()
                && !enqueue_unit(&mut assignment, &mut forced, &mut queue, new_lit)
            {
                return unsat(forced);
            }
        }
    }

    // Excludes a residual unit clause for an already-forced variable; forced
    // literals are reported separately for the caller to handle.
    let result = working
        .into_iter()
        .filter_map(|slot| slot.map(Clause::new))
        .filter(|c| {
            if c.literals.len() == 1 {
                let l = c.literals[0];
                assignment[l.var.idx()].is_none()
            } else {
                true
            }
        })
        .collect();

    (result, forced)
}

fn enqueue_unit(
    assignment: &mut [Option<bool>],
    forced: &mut Vec<Literal>,
    queue: &mut Vec<Literal>,
    lit: Literal,
) -> bool {
    match try_assign(assignment, forced, lit) {
        AssignResult::Fresh => {
            queue.push(lit);
            true
        }
        AssignResult::Redundant => true,
        AssignResult::Conflict => false,
    }
}

/// Unit propagation from a few assumed literals over the clauses that lie
/// inside a window of leaf positions, each clause switchable on and off.
///
/// Literals are DIMACS; a clause's span is the smallest and largest leaf
/// position of its variables, and it takes part in a call when that span lies
/// inside the call's window and it is active.
pub(crate) struct ScopedUnits {
    clauses: Vec<Vec<i32>>,
    span: Vec<(u32, u32)>,
    active: Vec<bool>,
    /// Clause ids per literal, at `2 * var + negative`.
    occ: Vec<Vec<usize>>,
    /// Per variable: 0 unassigned, 1 true, -1 false.
    value: Vec<i8>,
    trail: Vec<i32>,
}

impl ScopedUnits {
    pub(crate) fn new(num_vars: usize) -> Self {
        ScopedUnits {
            clauses: Vec::new(),
            span: Vec::new(),
            active: Vec::new(),
            occ: vec![Vec::new(); 2 * (num_vars + 1)],
            value: vec![0; num_vars + 1],
            trail: Vec::new(),
        }
    }

    fn slot(lit: i32) -> usize {
        2 * lit.unsigned_abs() as usize + usize::from(lit < 0)
    }

    /// Add an active clause with leaf span `lo..=hi`; returns its id.
    pub(crate) fn push(&mut self, lits: &[i32], lo: u32, hi: u32) -> usize {
        let id = self.clauses.len();
        for &l in lits {
            self.occ[Self::slot(l)].push(id);
        }
        self.clauses.push(lits.to_vec());
        self.span.push((lo, hi));
        self.active.push(true);
        id
    }

    pub(crate) fn set_active(&mut self, id: usize, on: bool) {
        self.active[id] = on;
    }

    fn value_of(&self, lit: i32) -> i8 {
        let v = self.value[lit.unsigned_abs() as usize];
        if lit < 0 { -v } else { v }
    }

    /// Whether propagating `assume` over the active clauses inside positions
    /// `lo..hi` reaches a conflict. Also returns the literals read.
    pub(crate) fn refutes(&mut self, assume: &[i32], lo: u32, hi: u32) -> (bool, u64) {
        let mut read = 0u64;
        let mut conflict = false;
        for &l in assume {
            match self.value_of(l) {
                1 => {}
                -1 => conflict = true,
                _ => {
                    self.value[l.unsigned_abs() as usize] = if l > 0 { 1 } else { -1 };
                    self.trail.push(l);
                }
            }
        }
        let mut head = 0;
        while !conflict && head < self.trail.len() {
            let lit = self.trail[head];
            head += 1;
            for k in 0..self.occ[Self::slot(-lit)].len() {
                let ci = self.occ[Self::slot(-lit)][k];
                let (clo, chi) = self.span[ci];
                if !self.active[ci] || clo < lo || chi >= hi {
                    continue;
                }
                let mut free = 0;
                let mut unit = 0;
                let mut sat = false;
                for &m in &self.clauses[ci] {
                    read += 1;
                    match self.value_of(m) {
                        1 => {
                            sat = true;
                            break;
                        }
                        0 => {
                            free += 1;
                            unit = m;
                            if free > 1 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if sat || free > 1 {
                    continue;
                }
                if free == 0 {
                    conflict = true;
                    break;
                }
                self.value[unit.unsigned_abs() as usize] = if unit > 0 { 1 } else { -1 };
                self.trail.push(unit);
            }
        }
        for &l in &self.trail {
            self.value[l.unsigned_abs() as usize] = 0;
        }
        self.trail.clear();
        (conflict, read)
    }
}
