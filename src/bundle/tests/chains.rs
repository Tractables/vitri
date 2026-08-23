//! The count-preserving chain's clause-blowup gate.

use super::super::count_chain::grew_clause_count;
use crate::cnf::CnfFormula;
use crate::tests::common::make_formula;

/// Variable 1 with three positive and three negative occurrences — the shape
/// eliminating it by resolution turns into every pairing of the two sides.
fn raw() -> CnfFormula {
    make_formula(
        7,
        vec![
            vec![1, 2],
            vec![1, 3],
            vec![1, 4],
            vec![-1, 5],
            vec![-1, 6],
            vec![-1, 7],
        ],
    )
}

/// [`raw`] with variable 1 resolved away: six clauses become the nine pairings,
/// and what was 2..=7 is renumbered 1..=6. Fewer variables, more clauses — the
/// trade the gate exists to refuse.
fn resolved_away() -> CnfFormula {
    let clauses = [2, 3, 4]
        .into_iter()
        .flat_map(|pos| [5, 6, 7].into_iter().map(move |neg| vec![pos - 1, neg - 1]))
        .collect();
    make_formula(6, clauses)
}

/// A reduction that came back the same size: six clauses again, over one
/// variable fewer.
fn traded_evenly() -> CnfFormula {
    make_formula(
        6,
        vec![
            vec![1, 4],
            vec![1, 5],
            vec![1, 6],
            vec![2, 4],
            vec![3, 5],
            vec![2, 6],
        ],
    )
}

/// The gate reads clause counts and nothing else — not the variable count, not
/// the numbering, which is what lets it compare two formulas over different
/// variable spaces. It refuses on `>`, so an even trade is kept.
#[test]
fn a_reduction_that_grew_the_clause_count_is_discarded() {
    let raw = raw();

    assert_eq!(
        grew_clause_count(&raw, &resolved_away()),
        Some("it grew the clause count"),
        "one variable saved does not pay for three more clauses",
    );
    assert_eq!(
        grew_clause_count(&raw, &traded_evenly()),
        None,
        "the same clause count over fewer variables is a reduction worth keeping",
    );
    assert_eq!(
        grew_clause_count(&raw, &make_formula(6, vec![vec![1, 2], vec![3, 4]])),
        None,
        "fewer clauses is the ordinary case, and carries no reason",
    );
}
