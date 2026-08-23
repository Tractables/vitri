//! Tests for the two shape statistics and the `coloring_like` predicate over
//! them.

use crate::cnf::CnfFormula;
use crate::cnf::stats::*;
use crate::tests::common::{grid_fixture, mixed_width_fixture};

/// The formula a DIMACS text parses to. Nothing here reads a meta line, so the
/// metadata half is dropped at the door rather than at every call.
fn formula_of(s: &str) -> CnfFormula {
    crate::tests::common::parse(s).0
}

/// `clause_width_cv` is THE width half of the coloring-like predicate. Pin it at
/// the 0.30 threshold the predicate tests — a uniform-width clause set scores 0
/// (passes), a mixed 2..8-wide one is far above it (fails).
#[test]
fn clause_width_cv_pins_the_predicate_threshold() {
    let cv_uniform = clause_width_cv(&grid_fixture());
    assert!(
        cv_uniform < COLORING_WIDTH_CV_MAX,
        "uniform-width CNF must pass the width half, got {cv_uniform}"
    );
    let cv_mixed = clause_width_cv(&mixed_width_fixture());
    assert!(
        cv_mixed > COLORING_WIDTH_CV_MAX,
        "mixed-width CNF must fail the width half, got {cv_mixed}"
    );
    // Degenerate inputs score 0.0, never NaN.
    assert_eq!(clause_width_cv(&formula_of("p cnf 2 1\n1 2 0\n")), 0.0);
}

/// Every variable's occurrence count enters the statistic, including the first
/// one. Renaming variables permutes the counts without changing their multiset,
/// so the coefficient of variation has to come out the same — a fixture whose
/// only difference is which id carries the frequent variable pins that.
#[test]
fn var_occurrence_cv_counts_every_variable() {
    // Occurrence counts 4, 2, 2 — carried by variable 1 and by variable 3.
    let first_is_frequent = formula_of("p cnf 3 4\n1 2 0\n1 3 0\n1 2 0\n1 3 0\n");
    let last_is_frequent = formula_of("p cnf 3 4\n3 2 0\n3 1 0\n3 2 0\n3 1 0\n");
    let cv_first = var_occurrence_cv(&first_is_frequent);
    assert!(
        cv_first > 0.0,
        "unequal occurrence counts must disperse, got {cv_first}"
    );
    assert_eq!(
        cv_first,
        var_occurrence_cv(&last_is_frequent),
        "the statistic must not depend on which variable id carries a count",
    );
}

/// Both halves have to pass, and neither relaxes the other.
#[test]
fn coloring_like_needs_both_halves() {
    // Uniform grid signal: uniform occurrence, uniform clause width.
    assert!(coloring_like_predicate(0.013, 0.206));
    // Dispersed clause widths ⇒ not coloring-like, however uniform the
    // occurrence is.
    assert!(!coloring_like_predicate(0.014, 0.66));
    assert!(!coloring_like_predicate(0.3, 0.46));
    // The occurrence half binds on its own, however uniform the widths are.
    assert!(!coloring_like_predicate(0.9, 0.01));
}

/// Both statistics on one formula, through the predicate: the grid is
/// coloring-like and the mixed-width clause set is not. This is the pair the
/// Arjun `auto` policy and the portfolio's adoption gate both read.
#[test]
fn the_two_statistics_classify_the_two_fixtures() {
    let grid = grid_fixture();
    assert!(
        coloring_like_predicate(var_occurrence_cv(&grid), clause_width_cv(&grid)),
        "a uniform grid must be coloring-like",
    );
    let mixed = mixed_width_fixture();
    assert!(
        !coloring_like_predicate(var_occurrence_cv(&mixed), clause_width_cv(&mixed)),
        "dispersed clause widths must not be coloring-like",
    );
}
