//! One leaf per variable, whatever the decomposition and whatever the reading.
//!
//! A conversion that dropped or duplicated a leaf would produce a vtree no
//! consumer can score against the formula it was built for.

use std::collections::HashSet;

use super::{edge_reading, hub_of_clusters};
use crate::decompose::{Binarization, Place, Reading, Root, td_to_vtree, td_to_vtree_reading};
use crate::error::VitriError;
use crate::tests::common::{assert_covers_all_vars, make_formula, make_td};

#[test]
fn the_edge_binarization_gives_one_leaf_per_variable() {
    let (formula, td) = hub_of_clusters(8, 6, 4);
    let nv = formula.num_vars;
    let vtree = td_to_vtree_reading(&td, nv, edge_reading(), Some(&formula), None)
        .expect("the fixture decomposition covers the fixture formula");
    assert_covers_all_vars(&vtree, nv, "the TD-edge-aligned conversion");
}

/// The only bag covers two of the three declared variables, so the third
/// reaches the vtree through the append of variables no bag holds at all.
#[test]
fn a_variable_no_bag_holds_becomes_a_leaf_of_its_own() {
    let td = make_td(vec![vec![0, 1]], vec![], 2);
    let vtree = td_to_vtree(&td, 3).expect("a decomposition of part of the formula converts");

    assert_eq!(vtree.num_leaves(), 3);
}

/// Single-variable TD with a single bag. Smallest valid input, and a check that
/// the 1-leaf vtree path doesn't trip the balanced-combine fallback.
#[test]
fn one_bag_holding_one_variable_converts() {
    let td = make_td(vec![vec![0]], vec![], 1);
    let vtree = td_to_vtree(&td, 1).expect("one bag holding the one variable converts");
    assert_covers_all_vars(&vtree, 1, "one bag holding one variable");
}

/// Multi-component TD where one component has an isolated variable not in any
/// bag. Mixes both recovery paths: component join, and the append of variables
/// that appear in no bag at all.
#[test]
fn components_and_a_variable_no_bag_holds_both_reach_the_vtree() {
    // bag0 = {0, 1}, bag1 = {2, 3}, no edges. Variable 4 in no bag.
    let td = make_td(vec![vec![0, 1], vec![2, 3]], vec![], 4);
    let vtree = td_to_vtree(&td, 5).expect("two components and a loose variable convert");
    assert_covers_all_vars(&vtree, 5, "two components plus a variable no bag holds");
}

/// Single bag stuffed with all variables. The wide-bag stress case: the in-bag
/// ordering heuristics all reduce to identity when there is no clause-affinity
/// signal to break ties.
#[test]
fn one_bag_holding_every_variable_converts() {
    let all_vars: Vec<u32> = (0..16).collect();
    let td = make_td(vec![all_vars], vec![], 16);
    let vtree = td_to_vtree(&td, 16).expect("one bag holding everything converts");
    assert_covers_all_vars(&vtree, 16, "one bag holding every variable");
}

/// A decomposition of an incidence graph holds a vertex per clause as well as
/// per variable, and those vertices are not variables: the vtree gets one leaf
/// per variable, not one per bag vertex.
#[test]
fn a_bag_vertex_past_the_variables_is_not_a_leaf() {
    // 3 variables, 2 clauses: the incidence graph has 5 vertices, 0..3 the
    // variables and 3, 4 the clauses. One bag holds all five.
    let td = make_td(vec![vec![0, 1, 2, 3, 4]], vec![], 5);
    let reading = Reading {
        place: Some(Place::Deep),
        ..Reading::default()
    };
    let vtree = td_to_vtree_reading(&td, 3, reading, None, None)
        .expect("a decomposition of the incidence graph converts over the variables");

    assert_eq!(vtree.num_leaves(), 3);
    let leaf_vars: HashSet<u32> = vtree.leaf_bottomup().map(|(_t, var)| var.0).collect();
    assert_eq!(leaf_vars, HashSet::from([0, 1, 2]));
}

/// Every reading the three dimensions name, on a decomposition that is one path
/// and on one that falls into two components, with and without the formula the
/// clause-driven binarizations read.
///
/// A reading chooses a shape; none of them may choose a different variable set.
#[test]
fn every_reading_gives_one_leaf_per_variable() {
    let path = make_td(
        vec![vec![0, 1], vec![1, 2], vec![2, 3], vec![3, 4]],
        vec![(0, 1), (1, 2), (2, 3)],
        5,
    );
    let split = make_td(vec![vec![0, 1, 2], vec![3, 4], vec![5, 6]], vec![(1, 2)], 7);
    let path_formula = make_formula(5, vec![vec![1, 2], vec![2, 3], vec![3, 4], vec![4, 5]]);
    let split_formula = make_formula(7, vec![vec![1, 2, 3], vec![4, 5], vec![6, 7], vec![-4, 6]]);

    for (shape, td, num_vars, formula) in [
        ("a path", &path, 5u32, &path_formula),
        ("two components", &split, 7, &split_formula),
    ] {
        for place in [Place::Shallow, Place::Deep] {
            for root in [Root::First, Root::Centroid, Root::Leaf] {
                for binarize in [
                    Binarization::Edge,
                    Binarization::Hypergraph,
                    Binarization::Balanced,
                ] {
                    let reading = Reading {
                        root: Some(root),
                        place: Some(place),
                        binarize: Some(binarize),
                    };
                    for read_formula in [None, Some(formula)] {
                        let vtree = td_to_vtree_reading(td, num_vars, reading, read_formula, None)
                            .expect("the fixture decomposition covers the fixture formula");
                        let what = format!(
                            "{shape} under {reading:?} (formula: {})",
                            read_formula.is_some(),
                        );
                        assert_eq!(
                            vtree.num_leaves(),
                            num_vars,
                            "{what} changed the leaf count"
                        );
                        assert_covers_all_vars(&vtree, num_vars, &what);
                    }
                }
            }
        }
    }
}

/// A caller holding its own decomposition can hand over a pair that names no
/// vtree at all: a decomposition with no bags, or a formula with no variables.
/// Both come back as errors rather than as a panic inside the conversion.
///
/// A decomposition over a different variable set is not one of them; the tests
/// above are what that converts to.
#[test]
fn a_pair_that_names_no_vtree_is_an_error() {
    let empty = make_td(vec![], vec![], 0);
    let Err(error) = td_to_vtree(&empty, 3) else {
        panic!("an empty decomposition converted to a vtree");
    };
    assert!(matches!(error, VitriError::Input { .. }), "{error}");

    let one_bag = make_td(vec![vec![0]], vec![], 1);
    let Err(error) = td_to_vtree(&one_bag, 0) else {
        panic!("a formula with no variables converted to a vtree");
    };
    assert!(matches!(error, VitriError::Input { .. }), "{error}");
}
