//! The shared occurrence, mask and frequency builders.
//!
//! [`crate::cnf::occ`] holds five derived views of a clause set that six passes
//! used to build inline, each under its own name. Two angles here: the exact
//! contents of every table on a formula small enough to read off by hand, and
//! the identities that hold between them on one with real circuit structure —
//! the agreement that lets one set of tables serve all six readers.

use crate::cnf::occ::*;
use crate::cnf::{Clause, CnfFormula, Literal, VarId};
use crate::tests::common::lit;

/// A multiplier encoding: variables occur with both polarities, at very
/// different frequencies, and every clause is a Tseitin gate rather than a
/// shape picked to make a table come out a particular way.
fn fixture() -> CnfFormula {
    crate::tests::circuit_fixture::multiplier()
}

/// A small hand-built formula whose declared count exceeds the variables it
/// actually uses, exercising the "declared but absent" entries of every table.
fn sparse_fixture() -> CnfFormula {
    CnfFormula {
        num_vars: 8,
        clauses: vec![
            Clause::new(vec![lit(0, true), lit(2, false)]),
            Clause::new(vec![lit(2, true), lit(5, true), lit(0, false)]),
            Clause::new(vec![lit(5, false)]),
        ],
    }
}

/// The normalization projected BVE applies before building its lists. It works
/// on literal vectors rather than [`Clause`] values, which is the case
/// [`occurrence_lists_of`] exists for.
fn normalized(formula: &CnfFormula) -> Vec<Vec<Literal>> {
    formula
        .clauses
        .iter()
        .map(|c| {
            let mut lits = c.literals.clone();
            lits.sort_by_key(|l| (l.var.0, !l.positive));
            lits.dedup();
            lits
        })
        .collect()
}

/// Three clauses — `0 ∨ ¬2`, `2 ∨ 5 ∨ ¬0`, `¬5` — over eight declared
/// variables, so every table is short enough to state outright.
#[test]
fn every_table_reads_off_a_three_clause_formula() {
    let formula = sparse_fixture();
    let n = formula.num_vars as usize;
    let clauses = &formula.clauses;

    // One row per declared variable, holding the clauses that variable occurs in.
    let table = |rows: [&[usize]; 8]| rows.iter().map(|r| r.to_vec()).collect::<Vec<_>>();
    assert_eq!(
        occurrence_lists(clauses, n),
        (
            table([&[0], &[], &[1], &[], &[], &[1], &[], &[]]),
            table([&[1], &[], &[0], &[], &[], &[2], &[], &[]]),
        )
    );

    assert_eq!(
        appearance_mask(clauses, n),
        vec![true, false, true, false, false, true, false, false]
    );
    assert_eq!(frequency(clauses, n), vec![2, 0, 2, 0, 0, 2, 0, 0]);
    assert_eq!(
        literal_frequency(clauses, n),
        vec![1, 1, 0, 0, 1, 1, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0]
    );
}

/// The tables are views of one relation, so each pins down the others: a
/// variable's two occurrence lists name exactly the clauses its two literal
/// counts count, and the mask says only whether either list is non-empty.
#[test]
fn the_tables_agree_on_a_circuit_encoding() {
    let formula = fixture();
    let n = formula.num_vars as usize;
    let clauses = &formula.clauses;

    let (pos, neg) = occurrence_lists(clauses, n);
    let mask = appearance_mask(clauses, n);
    let per_lit = literal_frequency(clauses, n);

    for v in 0..n {
        assert_eq!(
            mask[v],
            !(pos[v].is_empty() && neg[v].is_empty()),
            "variable {v}"
        );
        assert_eq!(
            per_lit[literal_index(v, true)] as usize,
            pos[v].len(),
            "variable {v}"
        );
        assert_eq!(
            per_lit[literal_index(v, false)] as usize,
            neg[v].len(),
            "variable {v}"
        );
        for &ci in &pos[v] {
            assert!(
                clauses[ci]
                    .literals
                    .contains(&Literal::pos(VarId(v as u32))),
                "variable {v}, clause {ci}"
            );
        }
        for &ci in &neg[v] {
            assert!(
                clauses[ci]
                    .literals
                    .contains(&Literal::neg(VarId(v as u32))),
                "variable {v}, clause {ci}"
            );
        }
    }

    let norm = normalized(&formula);
    assert_eq!(
        occurrence_lists_of(norm.iter().map(|c| c.as_slice()), n),
        (pos, neg)
    );
}

/// The two frequency granularities are different tables, kept apart on purpose:
/// summing a variable's two polarity counts is exactly its variable count, and
/// a caller that needs the split cannot recover it from the sum.
#[test]
fn per_variable_frequency_is_the_sum_of_the_two_polarities() {
    let formula = fixture();
    let n = formula.num_vars as usize;
    let per_var = frequency(&formula.clauses, n);
    let per_lit = literal_frequency(&formula.clauses, n);
    for v in 0..n {
        assert_eq!(
            per_var[v],
            per_lit[literal_index(v, true)] + per_lit[literal_index(v, false)],
            "variable {v}"
        );
    }
}
