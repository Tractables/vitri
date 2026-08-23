//! The reader is deliberately tolerant of real-world files — clauses
//! spread over lines, more clauses than the header promised, a trailing
//! `%` block, a last clause missing its `0` — while still normalizing
//! duplicate and tautological clauses, reading a bare `0` as the empty
//! clause, and refusing tokens that are not literals.

use super::*;

#[test]
fn test_parse_simple_dimacs() {
    let input = b"c comment\np cnf 3 2\n1 -2 0\n2 3 0\n";
    let formula = CnfFormula::from_dimacs(&input[..]).unwrap().0;
    assert_eq!(formula.num_vars, 3);
    assert_eq!(formula.clauses.len(), 2);

    assert_eq!(formula.clauses[0].literals.len(), 2);
    assert_eq!(formula.clauses[0].literals[0], Literal::pos(VarId(0)));
    assert_eq!(formula.clauses[0].literals[1], Literal::neg(VarId(1)));

    assert_eq!(formula.clauses[1].literals.len(), 2);
    assert_eq!(formula.clauses[1].literals[0], Literal::pos(VarId(1)));
    assert_eq!(formula.clauses[1].literals[1], Literal::pos(VarId(2)));
}

#[test]
fn test_parse_empty_formula() {
    let input = b"p cnf 3 0\n";
    let formula = CnfFormula::from_dimacs(&input[..]).unwrap().0;
    assert_eq!(formula.num_vars, 3);
    assert_eq!(formula.clauses.len(), 0);
}

#[test]
fn test_parse_multiline_clause() {
    let input = b"p cnf 4 1\n1 2\n3 4 0\n";
    let formula = CnfFormula::from_dimacs(&input[..]).unwrap().0;
    assert_eq!(formula.clauses.len(), 1);
    assert_eq!(formula.clauses[0].literals.len(), 4);
}

#[test]
fn test_parse_rejects_invalid_tokens() {
    let input = b"p cnf 3 1\n1 NONE69 0\n";
    let result = CnfFormula::from_dimacs(&input[..]);
    assert!(result.is_err(), "should reject non-integer token NONE69");
    assert!(result.unwrap_err().to_string().contains("NONE69"));
}

#[test]
fn test_parse_rejects_bare_minus() {
    let input = b"p cnf 3 1\n1 2 - 0\n";
    let result = CnfFormula::from_dimacs(&input[..]);
    assert!(result.is_err(), "should reject bare minus sign");
}

#[test]
fn test_parse_reads_beyond_declared_clause_count() {
    let input = b"p cnf 3 2\n1 -2 0\n2 3 0\n1 3 0\n";
    let formula = CnfFormula::from_dimacs(&input[..]).unwrap().0;
    assert_eq!(
        formula.clauses.len(),
        3,
        "should read all clauses, not just declared 2"
    );
}

#[test]
fn test_parse_stops_at_satlib_eof() {
    let input = b"p cnf 3 10\n1 -2 0\n%\n0\nGARBAGE\n";
    let formula = CnfFormula::from_dimacs(&input[..]).unwrap().0;
    assert_eq!(formula.clauses.len(), 1, "should stop at % marker");
}

#[test]
fn test_parse_deduplicates_literals() {
    let input = b"p cnf 6 1\n1 6 6 0\n";
    let formula = CnfFormula::from_dimacs(&input[..]).unwrap().0;
    assert_eq!(formula.clauses.len(), 1);
    assert_eq!(
        formula.clauses[0].literals.len(),
        2,
        "duplicate literal should be removed: {:?}",
        formula.clauses[0].literals
    );
}

#[test]
fn test_parse_removes_tautological_clause() {
    let input = b"p cnf 5 2\n1 2 0\n5 -5 3 0\n";
    let formula = CnfFormula::from_dimacs(&input[..]).unwrap().0;
    assert_eq!(
        formula.clauses.len(),
        1,
        "tautological clause (5 ∨ ¬5) should be dropped"
    );
    assert_eq!(formula.clauses[0].literals.len(), 2);
}

/// A `0` with nothing before it is the empty clause, not a formatting quirk to
/// step over. Stepping over it drops the one clause no assignment satisfies, so
/// an instance that is genuinely UNSAT reads back as satisfiable — and no later
/// stage can re-derive a contradiction the parser never handed it.
#[test]
fn a_bare_zero_is_the_empty_clause() {
    let input = b"p cnf 2 2\n1 2 0\n0\n";
    let formula = CnfFormula::from_dimacs(&input[..]).unwrap().0;
    assert_eq!(formula.clauses.len(), 2);
    assert!(
        formula.clauses[1].literals.is_empty(),
        "the bare `0` must survive as the empty clause: {:?}",
        formula.clauses,
    );
}

/// The same `0`, written inline rather than on its own line: the first closes
/// the clause, the second closes the empty one after it.
#[test]
fn two_zeros_in_a_row_close_a_clause_and_then_the_empty_one() {
    let input = b"p cnf 2 1\n1 2 0 0\n";
    let formula = CnfFormula::from_dimacs(&input[..]).unwrap().0;
    assert_eq!(formula.clauses.len(), 2);
    assert_eq!(formula.clauses[0].literals.len(), 2);
    assert!(
        formula.clauses[1].literals.is_empty(),
        "the second `0` must close the empty clause: {:?}",
        formula.clauses,
    );
}

/// A last clause missing its `0` is normalized like a terminated one. It used
/// to bypass normalization entirely and hand `Clause::new` a variable twice,
/// breaking the at-most-once invariant the type states and asserts.
#[test]
fn a_final_clause_without_its_zero_is_sorted_and_deduplicated() {
    let input = b"p cnf 3 1\n3 1 3\n";
    let formula = CnfFormula::from_dimacs(&input[..]).unwrap().0;
    assert_eq!(formula.clauses.len(), 1);
    assert_eq!(
        formula.clauses[0].literals,
        vec![Literal::pos(VarId(0)), Literal::pos(VarId(2))],
    );
}

#[test]
fn a_final_tautological_clause_without_its_zero_is_dropped() {
    let input = b"p cnf 3 2\n1 2 0\n3 -3 1\n";
    let formula = CnfFormula::from_dimacs(&input[..]).unwrap().0;
    assert_eq!(
        formula.clauses.len(),
        1,
        "an unterminated tautology is dropped like a terminated one: {:?}",
        formula.clauses,
    );
}

#[test]
fn test_parse_no_duplicate_vars_in_clause() {
    // Regression: duplicate variables in a clause caused self-loops in the primal graph,
    // crashing arboretum's add_edge assertion. Verify each variable appears at most once.
    let input = b"p cnf 6 3\n1 2 3 4 0\n-2 -3 4 5 0\n-4 -5 6 6 0\n";
    let formula = CnfFormula::from_dimacs(&input[..]).unwrap().0;
    for (i, clause) in formula.clauses.iter().enumerate() {
        let mut vars: Vec<u32> = clause.literals.iter().map(|l| l.var.0).collect();
        vars.sort();
        let before = vars.len();
        vars.dedup();
        assert_eq!(
            vars.len(),
            before,
            "clause {} has duplicate variables: {:?}",
            i,
            clause.literals
        );
    }
}
