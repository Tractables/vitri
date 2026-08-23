use crate::cnf::Clause;
use crate::cnf::Literal;
use crate::cnf::VarId;
use crate::preprocess::renumber::*;

fn v(i: u32) -> VarId {
    VarId(i)
}

#[test]
fn survivors_are_renumbered_in_ascending_order_and_invert_back() {
    // Keep 1, 3, 4 out of 0..5.
    let r = Renumber::keeping(6, |x| matches!(x.0, 1 | 3 | 4));

    assert_eq!(r.num_old_vars(), 6);
    assert_eq!(r.num_new_vars(), 3);
    assert_eq!(r.kept(), &[v(1), v(3), v(4)]);

    assert_eq!(r.new_id(v(1)), Some(v(0)));
    assert_eq!(r.new_id(v(3)), Some(v(1)));
    assert_eq!(r.new_id(v(4)), Some(v(2)));
    for new in 0..3 {
        assert_eq!(r.new_id(r.old_id(v(new))), Some(v(new)));
    }

    // Dropped variables, and ids the old formula never had, have no new name.
    assert_eq!(r.new_id(v(0)), None);
    assert_eq!(r.new_id(v(2)), None);
    assert_eq!(r.new_id(v(9)), None);
}

#[test]
fn renumbering_a_literal_never_flips_its_polarity() {
    let r = Renumber::keeping(4, |x| x.0 != 0);

    assert_eq!(r.apply_lit(Literal::neg(v(2))), Some(Literal::neg(v(1))));
    assert_eq!(r.apply_lit(Literal::pos(v(2))), Some(Literal::pos(v(1))));
    assert_eq!(r.apply_lit(Literal::pos(v(0))), None);

    assert_eq!(r.apply_inverse_lit(Literal::neg(v(1))), Literal::neg(v(2)));
    assert_eq!(r.apply_inverse_lit(Literal::pos(v(1))), Literal::pos(v(2)));
}

#[test]
fn a_clause_that_loses_every_literal_survives_as_the_empty_clause() {
    // Eliminating variable 0 empties the first clause, which is the UNSAT
    // certificate the caller's has-empty-clause check reads.
    let clauses = vec![
        Clause::new(vec![Literal::pos(v(0))]),
        Clause::new(vec![Literal::pos(v(0)), Literal::neg(v(1))]),
    ];
    let (formula, r) = renumber_clauses(2, clauses, |x| x.0 != 0);

    assert_eq!(formula.num_vars, 1);
    assert_eq!(r.kept(), &[v(1)]);
    assert_eq!(
        formula.clauses,
        vec![
            Clause::new(vec![Literal::neg(v(0))]),
            Clause::new(Vec::new()),
        ],
    );
}

/// Stages run back to back, each renumbering the space the last one produced, so
/// the composition has to name a survivor by its ORIGINAL variable rather than
/// by the intermediate id it held in between — and a variable either stage
/// dropped must have no name at all.
#[test]
fn composing_two_renumberings_names_each_survivor_by_its_original_variable() {
    let outer = Renumber::keeping(6, |x| matches!(x.0, 1 | 3 | 4 | 5));
    // Over the four variables `outer` produced: keep its new 1 and 3, which are
    // the original 3 and 5.
    let inner = Renumber::keeping(4, |x| matches!(x.0, 1 | 3));

    let composed = outer.compose(&inner);

    assert_eq!(
        composed.num_old_vars(),
        6,
        "the composition is over the original space"
    );
    assert_eq!(composed.kept(), &[v(3), v(5)]);
    assert_eq!(composed.new_id(v(3)), Some(v(0)));
    assert_eq!(composed.new_id(v(5)), Some(v(1)));
    assert_eq!(
        composed.new_id(v(1)),
        None,
        "a variable the second stage dropped keeps no name from the first",
    );
    assert_eq!(
        composed.new_id(v(0)),
        None,
        "a variable the first stage dropped stays dropped",
    );
}

/// The empty clause is a certificate, not a count: however many clauses an
/// elimination empties, the result carries ONE of them, so a caller reading
/// "does this formula contain the empty clause" gets the same answer either way
/// and no consumer has to deduplicate.
#[test]
fn several_clauses_losing_every_literal_still_leave_exactly_one_empty_clause() {
    let clauses = vec![
        Clause::new(vec![Literal::pos(v(0))]),
        Clause::new(vec![Literal::neg(v(0)), Literal::pos(v(1))]),
        Clause::new(vec![Literal::pos(v(2))]),
    ];
    let (formula, _) = renumber_clauses(3, clauses, |x| x.0 == 2);

    assert_eq!(
        formula.clauses,
        vec![
            Clause::new(vec![Literal::pos(v(0))]),
            Clause::new(Vec::new()),
        ],
        "two emptied clauses must leave one empty clause, appended last",
    );
}
