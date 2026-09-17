use crate::cnf::{Clause, Literal, VarId, propagate_units};
use crate::tests::common::make_formula;

#[test]
fn propagation_returns_every_derived_assignment_and_the_residual() {
    let input = make_formula(3, vec![vec![1], vec![-1, 2], vec![1, 3]]);
    let propagated = propagate_units(&input);

    assert_eq!(
        propagated.forced,
        vec![Literal::pos(VarId(1)), Literal::pos(VarId(2))]
    );
    assert!(propagated.residual.clauses.is_empty());
    assert_eq!(propagated.residual.num_vars, input.num_vars);
}

#[test]
fn propagation_reports_a_contradiction_as_one_empty_clause() {
    let input = make_formula(1, vec![vec![1], vec![-1]]);
    let propagated = propagate_units(&input);

    assert_eq!(propagated.residual.clauses.len(), 1);
    assert!(propagated.residual.clauses[0].is_empty());
}

#[test]
fn a_preexisting_empty_clause_is_immediately_canonicalized() {
    let input = make_formula(2, vec![vec![], vec![1, 2]]);
    let propagated = propagate_units(&input);

    assert_eq!(propagated.residual.clauses, vec![Clause::new(vec![])]);
    assert!(propagated.forced.is_empty());
}
