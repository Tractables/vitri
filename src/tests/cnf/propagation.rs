use crate::cnf::{Clause, CnfFormula, Literal, VarId, propagate_units};

fn formula(num_vars: u32, clauses: &[&[i32]]) -> CnfFormula {
    CnfFormula {
        num_vars,
        clauses: clauses
            .iter()
            .map(|clause| Clause::new(clause.iter().copied().map(Literal::from).collect()))
            .collect(),
    }
}

#[test]
fn propagation_returns_every_derived_assignment_and_the_residual() {
    let input = formula(3, &[&[1], &[-1, 2], &[1, 3]]);
    let propagated = propagate_units(&input);

    assert_eq!(
        propagated.forced,
        vec![Literal::pos(VarId(0)), Literal::pos(VarId(1))]
    );
    assert!(propagated.residual.clauses.is_empty());
    assert_eq!(propagated.residual.num_vars, input.num_vars);
}

#[test]
fn propagation_reports_a_contradiction_as_one_empty_clause() {
    let input = formula(1, &[&[1], &[-1]]);
    let propagated = propagate_units(&input);

    assert_eq!(propagated.residual.clauses.len(), 1);
    assert!(propagated.residual.clauses[0].is_empty());
}

#[test]
fn a_preexisting_empty_clause_is_immediately_canonicalized() {
    let input = formula(2, &[&[], &[1, 2]]);
    let propagated = propagate_units(&input);

    assert_eq!(propagated.residual.clauses, vec![Clause::new(vec![])]);
    assert!(propagated.forced.is_empty());
}
