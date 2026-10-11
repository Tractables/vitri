//! Syntactic gate definitions: a subset of a variable's own clauses that fixes
//! its value on its own.
//!
//! Gate patterns, output `x` (each also matched with the output negated):
//!
//! - AND `x = l₁ ∧ … ∧ lₖ`: `(¬x ∨ lᵢ)` for each `i`, and `(x ∨ ¬l₁ ∨ … ∨ ¬lₖ)`.
//!   `k = 1` is the equivalence `x ≡ l₁`; OR is AND with the output negated.
//! - ITE `x = s ? t : e`: `(¬x ∨ ¬s ∨ t)`, `(x ∨ ¬s ∨ ¬t)`, `(¬x ∨ s ∨ e)`,
//!   `(x ∨ s ∨ ¬e)`.
//! - XOR `x = l₁ ⊕ … ⊕ lₖ` (or its negation), `2 ≤ k <` [`MAX_XOR_CLAUSE`]: every
//!   clause over `{x, l₁, …, lₖ}` with one parity of negated literals.
//!
//! Every match `G` has the property elimination relies on: the clauses of `G`
//! with `x` removed are unsatisfiable together, so for every assignment of the
//! other variables `G` forces `x` one way. Two consequences, both used by
//! [`super::elim`]: `x` is defined in any formula containing `G` (by the
//! variables `G` mentions), and resolving only the pairs with at least one side
//! in `G` gives exactly the formula full resolution gives, because every
//! resolvent of two clauses outside `G` is implied by those pairs. Resolvents of
//! two clauses inside `G` are tautologies for every pattern here.
//!
//! The match is searched among the variable's live clauses only, and it may
//! leave others: the variable can occur in any number of clauses outside its
//! definition.

use super::clause_db::{ClauseDb, ClauseId, LitMarks};
use crate::cnf::{Literal, VarId};

/// Longest clause an XOR definition is matched in: an XOR over `k` inputs needs
/// `2^k` clauses of `k + 1` literals, so the pattern is worth looking for only
/// while that stays small.
const MAX_XOR_CLAUSE: usize = 5;

/// The clauses of a variable that define it.
pub(super) struct Definition {
    pub(super) clauses: Vec<ClauseId>,
}

impl Definition {
    pub(super) fn contains(&self, id: ClauseId) -> bool {
        self.clauses.contains(&id)
    }

    /// The variables the definition fixes `x` from.
    pub(super) fn inputs<'a>(
        &'a self,
        db: &'a ClauseDb,
        x: VarId,
    ) -> impl Iterator<Item = VarId> + 'a {
        self.clauses
            .iter()
            .flat_map(move |&id| db.clause(id).iter().map(|l| l.var))
            .filter(move |&v| v != x)
    }
}

/// A gate definition of `x` among its live clauses, if one of the patterns in
/// the module doc matches. `steps` is charged one per literal visited.
pub(super) fn find_definition(
    db: &ClauseDb,
    x: VarId,
    marks: &mut LitMarks,
    steps: &mut u64,
) -> Option<Definition> {
    and_gate(db, Literal::pos(x), marks, steps)
        .or_else(|| and_gate(db, Literal::neg(x), marks, steps))
        .or_else(|| ite_gate(db, x, steps))
        .or_else(|| xor_gate(db, x, steps))
        .map(|clauses| Definition { clauses })
}

/// `out ≡ l₁ ∧ … ∧ lₖ`: the binary clauses `(¬out ∨ lᵢ)` and one clause
/// `(out ∨ ¬l₁ ∨ … ∨ ¬lₖ)` whose every other literal negates a binary partner.
fn and_gate(
    db: &ClauseDb,
    out: Literal,
    marks: &mut LitMarks,
    steps: &mut u64,
) -> Option<Vec<ClauseId>> {
    marks.clear();
    let mut any = false;
    for id in db.occurrences(out.negated()) {
        let clause = db.clause(id);
        *steps += clause.len() as u64;
        if let [a, b] = clause {
            let partner = if a.var == out.var { *b } else { *a };
            marks.set(partner, id);
            any = true;
        }
    }
    if !any {
        return None;
    }
    'long: for id in db.occurrences(out) {
        let clause = db.clause(id);
        *steps += clause.len() as u64;
        if clause.len() < 2 {
            continue;
        }
        let mut gate = Vec::with_capacity(clause.len());
        for &lit in clause {
            if lit.var == out.var {
                continue;
            }
            match marks.get(lit.negated()) {
                Some(binary) => gate.push(binary),
                None => continue 'long,
            }
        }
        gate.push(id);
        return Some(gate);
    }
    None
}

/// The literals of a ternary clause other than `x`'s, if `id` is ternary.
fn ternary_others(db: &ClauseDb, id: ClauseId, x: VarId) -> Option<(Literal, Literal)> {
    match db.clause(id) {
        [a, b, c] => {
            let mut others = [*a, *b, *c].into_iter().filter(|l| l.var != x);
            Some((others.next()?, others.next()?))
        }
        _ => None,
    }
}

/// The canonical key of a ternary clause: its literals sorted by variable, as a
/// stored clause has them.
fn ternary_key(mut lits: [Literal; 3]) -> [Literal; 3] {
    lits.sort_by_key(|l| l.var);
    lits
}

/// `x = s ? t : e`, found from `(¬x ∨ ¬s ∨ t)` and `(¬x ∨ s ∨ e)` with the two
/// clauses that complete each half.
fn ite_gate(db: &ClauseDb, x: VarId, steps: &mut u64) -> Option<Vec<ClauseId>> {
    let (pos, neg) = (Literal::pos(x), Literal::neg(x));
    let negative: Vec<(ClauseId, Literal, Literal)> = db
        .occurrences(neg)
        .filter_map(|id| ternary_others(db, id, x).map(|(a, b)| (id, a, b)))
        .collect();
    if negative.len() < 2 {
        return None;
    }
    let positive: rustc_hash::FxHashMap<[Literal; 3], ClauseId> = db
        .occurrences(pos)
        .filter(|&id| db.clause(id).len() == 3)
        .map(|id| {
            let c = db.clause(id);
            ([c[0], c[1], c[2]], id)
        })
        .collect();
    if positive.len() < 2 {
        return None;
    }
    *steps += 3 * (negative.len() + positive.len()) as u64;
    for &(c1, a, b) in &negative {
        // c1 = (¬x ∨ ¬s ∨ t) with ¬s one of its literals and t the other.
        for (not_s, t) in [(a, b), (b, a)] {
            let Some(&c2) = positive.get(&ternary_key([pos, not_s, t.negated()])) else {
                continue;
            };
            for &(c3, p, q) in &negative {
                *steps += 1;
                // c3 = (¬x ∨ s ∨ e).
                let e = if p == not_s.negated() {
                    q
                } else if q == not_s.negated() {
                    p
                } else {
                    continue;
                };
                if let Some(&c4) = positive.get(&ternary_key([pos, not_s.negated(), e.negated()])) {
                    return Some(vec![c1, c2, c3, c4]);
                }
            }
        }
    }
    None
}

/// `x` the parity of the other variables of a clause, found as every clause
/// over those variables with that clause's parity of negated literals.
fn xor_gate(db: &ClauseDb, x: VarId, steps: &mut u64) -> Option<Vec<ClauseId>> {
    let (pos, neg) = (Literal::pos(x), Literal::neg(x));
    let short = |id: &ClauseId| (3..=MAX_XOR_CLAUSE).contains(&db.clause(*id).len());
    let by_literals: rustc_hash::FxHashMap<&[Literal], ClauseId> = db
        .occurrences(pos)
        .chain(db.occurrences(neg))
        .filter(short)
        .map(|id| (db.clause(id), id))
        .collect();
    let mut tried: rustc_hash::FxHashSet<Vec<VarId>> = rustc_hash::FxHashSet::default();
    for base in db.occurrences(pos).filter(short) {
        let clause = db.clause(base);
        let width = clause.len();
        let needed = 1usize << (width - 1);
        let same_parity = by_literals
            .keys()
            .filter(|c| c.len() == width && c.iter().zip(clause).all(|(l, m)| l.var == m.var))
            .count();
        *steps += (width * by_literals.len()) as u64;
        if same_parity < needed || !tried.insert(clause.iter().map(|l| l.var).collect()) {
            continue;
        }
        let parity = clause.iter().filter(|l| !l.positive).count() % 2;
        let mut gate = Vec::with_capacity(needed);
        let mut literals = clause.to_vec();
        for negations in 0u32..(1 << width) {
            if negations.count_ones() as usize % 2 != parity {
                continue;
            }
            for (i, lit) in literals.iter_mut().enumerate() {
                lit.positive = negations & (1 << i) == 0;
            }
            *steps += width as u64;
            match by_literals.get(literals.as_slice()) {
                Some(&id) => gate.push(id),
                None => break,
            }
        }
        if gate.len() == needed {
            return Some(gate);
        }
    }
    None
}
