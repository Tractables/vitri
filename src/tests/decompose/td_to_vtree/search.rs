//! Which reading the search picks, and what bounds it.

use std::time::{Duration, Instant};

use super::{edge_reading, hub_of_clusters};
use crate::cnf::CnfFormula;
use crate::decompose::{
    Binarization, Place, Reading, Root, TreeDecomposition, td_to_vtree_reading,
};
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

#[test]
fn the_edge_binarization_is_deterministic() {
    let (formula, td) = hub_of_clusters(8, 5, 4);
    let nv = formula.num_vars;
    let a = td_to_vtree_reading(&td, nv, edge_reading(), Some(&formula), None)
        .expect("the fixture decomposition covers the fixture formula");
    let b = td_to_vtree_reading(&td, nv, edge_reading(), Some(&formula), None)
        .expect("the fixture decomposition covers the fixture formula");
    let al: Vec<u32> = a.leaf_bottomup().map(|(_, v)| v.0).collect();
    let bl: Vec<u32> = b.leaf_bottomup().map(|(_, v)| v.0).collect();
    assert_eq!(
        al, bl,
        "the edge-aligned binarization must be deterministic"
    );
}

/// Bag-tree edges are undirected structure, not an ordering signal. Two
/// decompositions that differ only in the order those edges were supplied must
/// therefore produce the same vtree under the same fixed reading.
#[test]
fn equivalent_bag_edge_orders_convert_to_the_same_vtree() {
    let bags = vec![vec![0], vec![0, 1], vec![0, 2], vec![0, 3], vec![0, 4]];
    let ascending = make_td(bags.clone(), vec![(0, 1), (0, 2), (0, 3), (0, 4)], 5);
    let descending = make_td(bags, vec![(0, 4), (0, 3), (0, 2), (0, 1)], 5);
    let reading = Reading {
        root: Some(Root::First),
        place: Some(Place::Deep),
        binarize: Some(Binarization::Balanced),
    };

    let first = td_to_vtree_reading(&ascending, 5, reading, None, None)
        .expect("the fixture decomposition covers the fixture formula");
    let second = td_to_vtree_reading(&descending, 5, reading, None, None)
        .expect("the fixture decomposition covers the fixture formula");

    assert!(
        first.same_tree(&second),
        "equivalent undirected bag trees must not encode edge insertion order",
    );
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
    )
    .expect("the fixture decomposition covers the fixture formula");
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
    )
    .expect("the fixture decomposition covers the fixture formula");
    let bounded = td_to_vtree_reading(
        &td,
        formula.num_vars,
        Reading::default(),
        Some(&formula),
        Some(Instant::now() + Duration::from_secs(3600)),
    )
    .expect("the fixture decomposition covers the fixture formula");
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
    let named = |binarize| {
        td_to_vtree_reading(
            &td,
            formula.num_vars,
            Reading {
                root: Some(Root::First),
                place: Some(Place::Deep),
                binarize: Some(binarize),
            },
            Some(&formula),
            None,
        )
        .expect("the fixture decomposition covers the fixture formula")
        .to_vtree_text()
    };
    assert_ne!(
        named(Binarization::Hypergraph),
        named(Binarization::Balanced),
        "two readings named in full built the same tree, so neither was honoured",
    );
}

/// Without a formula there is nothing to score a reading against, so the
/// conversion builds exactly one whatever the caller left open: the reading the
/// screen runs at, with the one binarization that reads no clause.
#[test]
fn a_conversion_with_nothing_to_score_builds_the_screen_reading() {
    let (td, formula) = star_td();
    let unscored = td_to_vtree_reading(&td, formula.num_vars, Reading::default(), None, None)
        .expect("the fixture decomposition covers the fixture formula");
    let screen = td_to_vtree_reading(
        &td,
        formula.num_vars,
        Reading {
            root: Some(Root::First),
            place: Some(Place::Shallow),
            binarize: Some(Binarization::Balanced),
        },
        None,
        None,
    )
    .expect("the fixture decomposition covers the fixture formula");
    assert_eq!(
        unscored.to_vtree_text(),
        screen.to_vtree_text(),
        "a conversion with no formula searched something",
    );
}

/// `root=leaf` names a set of bags rather than one, so it still leaves the
/// search a choice, and the choice is over the leaf bags rather than over every
/// bag.
///
/// The star fixture's hub is not a leaf bag, so a rooting that reached it would
/// be reading the key as "any root".
#[test]
fn naming_the_leaf_rooting_still_searches_the_leaf_bags() {
    let (td, formula) = star_td();
    let leaves = |r: Root| {
        td_to_vtree_reading(
            &td,
            formula.num_vars,
            Reading {
                root: Some(r),
                place: Some(Place::Shallow),
                binarize: Some(Binarization::Balanced),
            },
            Some(&formula),
            None,
        )
        .expect("the fixture decomposition covers the fixture formula")
        .to_vtree_text()
    };
    assert_ne!(
        leaves(Root::Leaf),
        leaves(Root::First),
        "rooting at a leaf bag built what rooting at the first bag builds",
    );
}
