//! The clause store definability elimination edits in place.
//!
//! Clauses are kept normalized (sorted by variable, no repeated variable, no
//! tautology) and never move: a clause is named by its [`ClauseId`] for as long
//! as the store lives, and removing one only marks it dead. Each literal has
//! an occurrence list and a live count; the lists may still name dead clauses,
//! which every reader skips, and a list is compacted once its dead entries
//! outnumber its live ones.

use crate::cnf::occ::literal_index;
use crate::cnf::{Clause, Literal, VarId};

/// Index of a clause in a [`ClauseDb`], stable across edits.
pub(super) type ClauseId = u32;

pub(super) struct ClauseDb {
    num_vars: usize,
    clauses: Vec<Vec<Literal>>,
    alive: Vec<bool>,
    /// Per literal ([`literal_index`]), every clause containing it.
    occ: Vec<Vec<ClauseId>>,
    /// Per literal, the clauses whose FIRST literal it is. Each clause is in
    /// exactly one of these lists, which is what lets a subset search visit
    /// each candidate once.
    first: Vec<Vec<ClauseId>>,
    /// Per literal, how many live clauses contain it.
    count: Vec<u32>,
    live: usize,
}

#[inline]
fn index(lit: Literal) -> usize {
    literal_index(lit.var.idx(), lit.positive)
}

/// Whether every literal of `a` occurs in `b`, both sorted by variable.
pub(super) fn subset_sorted(a: &[Literal], b: &[Literal]) -> bool {
    if a.len() > b.len() {
        return false;
    }
    let mut j = 0;
    for &x in a {
        while j < b.len() && b[j].var < x.var {
            j += 1;
        }
        if j == b.len() || b[j] != x {
            return false;
        }
        j += 1;
    }
    true
}

impl ClauseDb {
    /// The store over `clauses`, normalized: tautologies are dropped, repeated
    /// literals merged, and a clause equal to an earlier one is not stored
    /// twice. Every literal must name a variable below `num_vars`.
    pub(super) fn new(num_vars: usize, clauses: impl IntoIterator<Item = Clause>) -> Self {
        let mut db = ClauseDb {
            num_vars,
            clauses: Vec::new(),
            alive: Vec::new(),
            occ: vec![Vec::new(); 2 * num_vars],
            first: vec![Vec::new(); 2 * num_vars],
            count: vec![0; 2 * num_vars],
            live: 0,
        };
        let mut seen: rustc_hash::FxHashSet<Vec<Literal>> = rustc_hash::FxHashSet::default();
        for clause in clauses {
            let Some(literals) = crate::cnf::normalize_literals(clause.literals) else {
                continue;
            };
            if seen.insert(literals.clone()) {
                db.add(literals);
            }
        }
        db
    }

    pub(super) fn num_vars(&self) -> usize {
        self.num_vars
    }

    /// How many clauses are live.
    pub(super) fn live_count(&self) -> usize {
        self.live
    }

    /// The literals of clause `id`, sorted by variable.
    #[inline]
    pub(super) fn clause(&self, id: ClauseId) -> &[Literal] {
        &self.clauses[id as usize]
    }

    /// The live clauses containing `lit`.
    pub(super) fn occurrences(&self, lit: Literal) -> impl Iterator<Item = ClauseId> + '_ {
        self.occ[index(lit)]
            .iter()
            .copied()
            .filter(|&id| self.alive[id as usize])
    }

    /// The live clauses whose first literal is `lit`.
    pub(super) fn first_occurrences(&self, lit: Literal) -> impl Iterator<Item = ClauseId> + '_ {
        self.first[index(lit)]
            .iter()
            .copied()
            .filter(|&id| self.alive[id as usize])
    }

    /// How many live clauses contain `lit`.
    #[inline]
    pub(super) fn occurrence_count(&self, lit: Literal) -> u32 {
        self.count[index(lit)]
    }

    /// The occurrence product of `var`: an upper bound on the resolvents
    /// eliminating it can produce.
    pub(super) fn occurrence_product(&self, var: VarId) -> u64 {
        u64::from(self.occurrence_count(Literal::pos(var)))
            * u64::from(self.occurrence_count(Literal::neg(var)))
    }

    /// How many live clauses mention `var` in either polarity.
    pub(super) fn occurrences_of_var(&self, var: VarId) -> u32 {
        self.occurrence_count(Literal::pos(var)) + self.occurrence_count(Literal::neg(var))
    }

    /// Store a normalized clause and return its id.
    pub(super) fn add(&mut self, literals: Vec<Literal>) -> ClauseId {
        let id = self.clauses.len() as ClauseId;
        for &lit in &literals {
            self.occ[index(lit)].push(id);
            self.count[index(lit)] += 1;
        }
        if let Some(&head) = literals.first() {
            self.first[index(head)].push(id);
        }
        self.clauses.push(literals);
        self.alive.push(true);
        self.live += 1;
        id
    }

    /// Mark clause `id` dead. Removing a dead clause is a caller bug.
    pub(super) fn remove(&mut self, id: ClauseId) {
        debug_assert!(self.alive[id as usize], "clause {id} removed twice");
        let ClauseDb {
            clauses,
            alive,
            occ,
            first,
            count,
            live,
            ..
        } = self;
        alive[id as usize] = false;
        *live -= 1;
        let literals = &clauses[id as usize];
        for &lit in literals {
            let i = index(lit);
            count[i] -= 1;
            if occ[i].len() > 2 * count[i] as usize + 16 {
                occ[i].retain(|&c| alive[c as usize]);
            }
        }
        if let Some(&head) = literals.first() {
            let i = index(head);
            if first[i].len() > 2 * count[i] as usize + 16 {
                first[i].retain(|&c| alive[c as usize]);
            }
        }
    }

    /// The live clauses, in the order they were stored.
    pub(super) fn to_clauses(&self) -> Vec<Clause> {
        self.clauses
            .iter()
            .zip(&self.alive)
            .filter(|(_, alive)| **alive)
            .map(|(literals, _)| Clause::new(literals.clone()))
            .collect()
    }

    /// [`Self::to_clauses`], consuming the store.
    pub(super) fn into_clauses(self) -> Vec<Clause> {
        self.clauses
            .into_iter()
            .zip(self.alive)
            .filter(|(_, alive)| *alive)
            .map(|(literals, _)| Clause::new(literals))
            .collect()
    }
}

/// A per-literal scratch table: [`Self::clear`] forgets every entry in
/// constant time, so one table serves every query of a pass.
pub(super) struct LitMarks {
    stamp: Vec<u32>,
    value: Vec<ClauseId>,
    current: u32,
}

impl LitMarks {
    pub(super) fn new(num_vars: usize) -> Self {
        LitMarks {
            stamp: vec![0; 2 * num_vars],
            value: vec![0; 2 * num_vars],
            current: 1,
        }
    }

    pub(super) fn clear(&mut self) {
        self.current = self.current.wrapping_add(1);
        if self.current == 0 {
            self.stamp.fill(0);
            self.current = 1;
        }
    }

    pub(super) fn mark(&mut self, lit: Literal) {
        self.set(lit, 0);
    }

    /// Mark `lit` and remember `value` with it.
    pub(super) fn set(&mut self, lit: Literal, value: ClauseId) {
        let i = index(lit);
        self.stamp[i] = self.current;
        self.value[i] = value;
    }

    #[inline]
    pub(super) fn is_marked(&self, lit: Literal) -> bool {
        self.stamp[index(lit)] == self.current
    }

    /// The value stored with `lit`, if it is marked.
    pub(super) fn get(&self, lit: Literal) -> Option<ClauseId> {
        self.is_marked(lit).then(|| self.value[index(lit)])
    }
}
