//! The derived per-variable views of a clause set, built in one place:
//! [`appearance_mask`] (does a variable occur), [`frequency`] /
//! [`literal_frequency`] (how often), [`occurrence_lists`] (in which
//! clauses).
//!
//! Every builder here silently skips a literal whose variable id is at or
//! above `num_vars` instead of indexing past the end of the table it fills —
//! relevant only on malformed DIMACS (a well-formed `p cnf` header always
//! bounds every literal), where it means a stray literal is dropped rather
//! than panicking.

use super::{Clause, Literal};

/// `mask[v]` is true when variable `v` occurs in at least one clause.
/// Length `num_vars`.
pub(crate) fn appearance_mask(clauses: &[Clause], num_vars: usize) -> Vec<bool> {
    let mut mask = vec![false; num_vars];
    for clause in clauses {
        for lit in &clause.literals {
            if let Some(slot) = mask.get_mut(lit.var.idx()) {
                *slot = true;
            }
        }
    }
    mask
}

/// `freq[v]` is how many literals of variable `v` occur across `clauses`, both
/// polarities counted together. Length `num_vars`. See [`literal_frequency`]
/// for the per-polarity split.
pub(crate) fn frequency(clauses: &[Clause], num_vars: usize) -> Vec<u32> {
    let mut freq = vec![0u32; num_vars];
    for clause in clauses {
        for lit in &clause.literals {
            if let Some(slot) = freq.get_mut(lit.var.idx()) {
                *slot += 1;
            }
        }
    }
    freq
}

/// Index of a literal in a per-literal table: `2v` for `v`, `2v + 1` for `¬v`.
pub(crate) fn literal_index(var: usize, positive: bool) -> usize {
    var * 2 + if positive { 0 } else { 1 }
}

/// Per-literal occurrence counts, indexed by [`literal_index`]: `freq[2v]`
/// counts occurrences of `v`, `freq[2v + 1]` counts occurrences of `¬v`.
/// Length `2 * num_vars`.
pub(crate) fn literal_frequency(clauses: &[Clause], num_vars: usize) -> Vec<u32> {
    let mut freq = vec![0u32; num_vars * 2];
    for clause in clauses {
        for lit in &clause.literals {
            if let Some(slot) = freq.get_mut(literal_index(lit.var.idx(), lit.positive)) {
                *slot += 1;
            }
        }
    }
    freq
}

/// For every variable, the indices of the clauses it occurs in: the first
/// returned vector holds the positive occurrences, the second the negative.
/// Both have length `num_vars`, and each variable's list is in clause order.
pub(crate) fn occurrence_lists(
    clauses: &[Clause],
    num_vars: usize,
) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
    occurrence_lists_of(clauses.iter().map(|c| c.literals.as_slice()), num_vars)
}

/// [`occurrence_lists`] over any sequence of literal slices. Clause indices
/// are positions in the sequence.
pub(crate) fn occurrence_lists_of<'a>(
    clauses: impl IntoIterator<Item = &'a [Literal]>,
    num_vars: usize,
) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
    let mut pos: Vec<Vec<usize>> = vec![Vec::new(); num_vars];
    let mut neg: Vec<Vec<usize>> = vec![Vec::new(); num_vars];
    for (ci, literals) in clauses.into_iter().enumerate() {
        for lit in literals {
            let bucket = if lit.positive { &mut pos } else { &mut neg };
            if let Some(list) = bucket.get_mut(lit.var.idx()) {
                list.push(ci);
            }
        }
    }
    (pos, neg)
}
