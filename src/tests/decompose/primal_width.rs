//! The primal-width bound, which a caller reads to compare two conditioning
//! choices against each other.

use crate::cnf::{CnfFormula, VarId};
use crate::decompose::conditioned_primal_width_ub;
use crate::error::VitriError;

fn formula(text: &str) -> CnfFormula {
    CnfFormula::from_dimacs(text.as_bytes())
        .expect("the fixture is well-formed DIMACS")
        .0
}

fn width(text: &str, conditioned: &[i32]) -> u32 {
    let conditioned: Vec<VarId> = conditioned.iter().map(|&v| VarId::from_dimacs(v)).collect();
    conditioned_primal_width_ub(&formula(text), &conditioned)
        .expect("every conditioned variable is inside the formula")
}

/// A chain of binary clauses is a path in the primal graph, and eliminating a
/// path leaves bags of two.
#[test]
fn the_bound_on_a_chain_of_binary_clauses_is_one() {
    assert_eq!(width("p cnf 5 4\n1 2 0\n2 3 0\n3 4 0\n4 5 0\n", &[]), 1);
}

/// One clause over every variable makes the primal graph complete, and a
/// complete graph on n vertices has no elimination order below n − 1.
#[test]
fn the_bound_on_a_single_wide_clause_is_one_below_its_width() {
    assert_eq!(width("p cnf 4 1\n1 2 3 4 0\n", &[]), 3);
}

/// What the bound is for: the same formula measured before and after a
/// conditioning choice, so the two numbers can be compared.
#[test]
fn conditioning_a_variable_out_of_a_clique_lowers_the_bound() {
    let clique = "p cnf 5 1\n1 2 3 4 5 0\n";
    assert_eq!(width(clique, &[]), 4);
    assert_eq!(width(clique, &[1]), 3);
    assert_eq!(width(clique, &[1, 2]), 2);
}

/// A variable in no clause is a vertex with no edges: removing it changes no
/// elimination.
#[test]
fn conditioning_a_variable_that_appears_in_no_clause_leaves_the_bound_alone() {
    let with_spare = "p cnf 5 1\n1 2 3 4 0\n";
    assert_eq!(width(with_spare, &[5]), width(with_spare, &[]));
}

/// Conditioning everything away leaves no graph, which is width zero rather
/// than a question the elimination is asked to answer.
#[test]
fn conditioning_every_variable_leaves_nothing_to_eliminate() {
    assert_eq!(width("p cnf 3 1\n1 2 3 0\n", &[1, 2, 3]), 0);
}

#[test]
fn a_conditioned_variable_outside_the_formula_is_refused_by_name() {
    let err =
        conditioned_primal_width_ub(&formula("p cnf 3 1\n1 2 3 0\n"), &[VarId::from_dimacs(9)])
            .expect_err("variable 9 is outside a three-variable formula");
    assert!(
        matches!(err, VitriError::Input { .. }),
        "the conditioned set is input data, got: {err:?}",
    );
    assert!(
        err.to_string().contains('9'),
        "the offending variable must appear, got: {err}",
    );
}

/// The same question asked twice gets the same answer, so two conditioning
/// choices compared against each other are comparing orders, not runs.
#[test]
fn the_bound_is_the_same_on_every_call() {
    let clique = "p cnf 6 2\n1 2 3 4 0\n3 4 5 6 0\n";
    assert_eq!(width(clique, &[2]), width(clique, &[2]));
}
