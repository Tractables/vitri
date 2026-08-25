//! The search over readings under a wall-clock bound.

use std::time::{Duration, Instant};

use crate::cnf::CnfFormula;
use crate::decompose::TreeDecomposition;
use crate::decompose::td_to_vtree::{Fold, Place, Reading, Root, td_to_vtree_reading};
use crate::tests::common::{make_formula, make_td};

/// A star decomposition: a hub bag holding variable 0, and one leaf bag per
/// other variable. Eight leaf bags is enough for the screen to have several
/// candidate roots to rank, so both halves of the search run.
fn star_td() -> (TreeDecomposition, CnfFormula) {
    let num_vars = 9;
    let mut bags = vec![vec![0u32]];
    let mut edges = Vec::new();
    for v in 1..num_vars {
        bags.push(vec![0, v]);
        edges.push((0usize, v as usize));
    }
    let clauses: Vec<Vec<i32>> = (1..num_vars).map(|v| vec![1, v as i32 + 1]).collect();
    (
        make_td(bags, edges, num_vars),
        make_formula(num_vars, clauses),
    )
}

/// A search handed a deadline that has already passed still returns a vtree
/// over every variable.
///
/// The bound governs how many readings get scored, never whether any does. The
/// caller is a construction that has just spent its whole budget building a
/// decomposition, so a refusal here would throw that decomposition away exactly
/// when the wall around it starts working.
#[test]
fn an_expired_deadline_still_returns_a_vtree_over_every_variable() {
    let (td, formula) = star_td();
    let vtree = td_to_vtree_reading(
        &td,
        formula.num_vars,
        Reading::default(),
        Some(&formula),
        Some(Instant::now() - Duration::from_secs(1)),
    );
    assert_eq!(
        vtree.num_leaves(),
        formula.num_vars,
        "an expired deadline returned a partial vtree",
    );
}

/// A deadline the search never reaches leaves the winner unchanged.
#[test]
fn a_deadline_the_search_never_reaches_selects_the_unbounded_winner() {
    let (td, formula) = star_td();
    let unbounded = td_to_vtree_reading(
        &td,
        formula.num_vars,
        Reading::default(),
        Some(&formula),
        None,
    );
    let bounded = td_to_vtree_reading(
        &td,
        formula.num_vars,
        Reading::default(),
        Some(&formula),
        Some(Instant::now() + Duration::from_secs(3600)),
    );
    assert_eq!(
        bounded.to_vtree_text(),
        unbounded.to_vtree_text(),
        "a bound the search never reaches changed the vtree it selected",
    );
}

/// A reading named in full is built as written: the search has nothing left to
/// walk, so the tree that comes back is that reading's, not the cheapest one
/// the same decomposition could have named.
#[test]
fn a_reading_named_in_full_is_the_one_that_is_built() {
    let (td, formula) = star_td();
    let named = |fold| {
        td_to_vtree_reading(
            &td,
            formula.num_vars,
            Reading {
                root: Some(Root::First),
                place: Some(Place::Deep),
                fold: Some(fold),
            },
            Some(&formula),
            None,
        )
        .to_vtree_text()
    };
    assert_ne!(
        named(Fold::LeftDeep),
        named(Fold::Balanced),
        "two readings named in full built the same tree, so neither was honoured",
    );
}

/// Without a formula there is nothing to score a reading against, so the
/// conversion builds exactly one whatever the caller left open — and it is the
/// same tree naming that one reading gives.
#[test]
fn a_conversion_with_nothing_to_score_builds_the_first_reading() {
    let (td, formula) = star_td();
    let unscored = td_to_vtree_reading(&td, formula.num_vars, Reading::default(), None, None);
    let first = td_to_vtree_reading(
        &td,
        formula.num_vars,
        Reading {
            root: Some(Root::First),
            place: Some(Place::Deep),
            fold: Some(Fold::Balanced),
        },
        None,
        None,
    );
    assert_eq!(
        unscored.to_vtree_text(),
        first.to_vtree_text(),
        "a conversion with no formula searched something",
    );
}
