//! Renumbering an UNSAT residual.
//!
//! An empty clause is a certificate, and compacting variable ids must not
//! lose it — including when every literal was eliminated and nothing is
//! left to renumber.

use super::*;

#[test]
fn renumber_preserves_empty_clause_unsat_certificate() {
    // var 0 is eliminated; var 1 survives and renumbers to local id 0.
    let fates = vec![DveFate::Defined, DveFate::Kept];
    let clauses = vec![
        Clause::new(Vec::new()),                         // UNSAT certificate
        Clause::new(vec![Literal::new(VarId(1), true)]), // survives
    ];
    let (formula, _map) = renumber_formula(&fates, 2, clauses);
    assert!(
        formula.clauses.iter().any(|c| c.literals.is_empty()),
        "empty clause (UNSAT certificate) must be preserved through renumber_formula",
    );
}

#[test]
fn renumber_empty_when_all_literals_eliminated_is_unsat() {
    let fates = vec![DveFate::Defined, DveFate::Kept]; // var 0 eliminated
    let clauses = vec![
        Clause::new(vec![Literal::new(VarId(0), true)]), // -> empty after renumber
        Clause::new(vec![Literal::new(VarId(1), false)]), // survives
    ];
    let (formula, _map) = renumber_formula(&fates, 2, clauses);
    assert!(
        formula.clauses.iter().any(|c| c.literals.is_empty()),
        "clause reduced to empty by elimination must be kept as UNSAT certificate",
    );
}
