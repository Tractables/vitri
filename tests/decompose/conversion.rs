//! A decomposition turned into a vtree: the edge shapes, and every reading

use super::*;

/// The only bag covers two of the three declared variables, so the third
/// reaches the vtree through the append of variables no bag holds at all.
#[test]
fn test_td_to_vtree_isolated_vars() {
    let td_str = "s td 1 2 2\nb 1 1 2\n";
    let td = parse_pace_td(td_str, GraphKind::Primal, 3).expect("parse");
    let vtree = td_to_vtree(&td, 3);

    assert_eq!(vtree.num_leaves(), 3);
}

/// Single-variable TD with a single bag. Smallest valid input — checks the
/// 1-leaf vtree path doesn't trip the balanced-combine fallback.
#[test]
fn test_td_to_vtree_single_var_single_bag() {
    let td = make_td(vec![vec![0]], vec![], 1);
    let vtree = td_to_vtree(&td, 1);
    assert_covers_all_vars(&vtree, 1, "one bag holding one variable");
}

/// Multi-component TD where one component has an isolated variable not in any
/// bag. Mixes both recovery paths: component join, and the append of variables
/// that appear in no bag at all.
#[test]
fn test_td_to_vtree_multi_component_plus_isolated_var() {
    // bag0 = {0, 1}, bag1 = {2, 3}, no edges. Variable 4 in no bag.
    let td = make_td(vec![vec![0, 1], vec![2, 3]], vec![], 5);
    let vtree = td_to_vtree(&td, 5);
    assert_covers_all_vars(&vtree, 5, "two components plus a variable no bag holds");
}

/// Single bag stuffed with all variables. The wide-bag stress case — the in-bag
/// ordering heuristics all reduce to identity when there is no clause-affinity
/// signal to break ties.
#[test]
fn test_td_to_vtree_single_wide_bag() {
    let all_vars: Vec<u32> = (0..16).collect();
    let td = make_td(vec![all_vars], vec![], 16);
    let vtree = td_to_vtree(&td, 16);
    assert_covers_all_vars(&vtree, 16, "one bag holding every variable");
}

#[test]
fn test_td_to_vtree_hypergraph_binarization() {
    let td_str = "s td 2 3 4\nb 1 1 2 3\nb 2 3 4\n1 2\n";
    let td = parse_pace_td(td_str, GraphKind::Primal, 4).expect("parse");

    let reading = Reading {
        root: Some(Root::First),
        place: Some(Place::Deep),
        binarize: Some(Binarization::Hypergraph),
    };
    let vtree = td_to_vtree_reading(&td, 4, reading, None, None);
    assert_eq!(vtree.num_leaves(), 4);
}

/// Every reading the three dimensions name, on a decomposition that is one path
/// and on one that falls into two components, with and without the formula the
/// clause-driven binarizations read.
///
/// A reading chooses a shape; none of them may choose a different variable set,
/// and a conversion that dropped or duplicated a leaf would produce a vtree no
/// consumer can score against the formula it was built for.
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
                        let vtree = td_to_vtree_reading(td, num_vars, reading, read_formula, None);
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
