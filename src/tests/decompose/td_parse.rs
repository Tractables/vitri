//! The tree-decomposition interchange type and the reader that fills it.

use crate::decompose::{GraphKind, TreeDecomposition, parse_pace_td};
use crate::tests::common::make_td;

/// A decomposition assembled directly, for the readings that are about the
/// type rather than about the file it came from.
fn td(kind: GraphKind, bags: Vec<Vec<u32>>, num_vars: u32) -> TreeDecomposition {
    TreeDecomposition {
        kind,
        ..make_td(bags, Vec::new(), num_vars)
    }
}

/// The bag list and the adjacency are indexed alike, and a consumer holding a
/// bag's position reads its neighbours at that same position. The solution line
/// sizes one and the bag lines fill the other, so a file whose two disagree
/// would slide a bag's neighbours onto a different bag.
#[test]
fn a_declared_bag_count_the_file_does_not_match_leaves_bags_and_adj_co_indexed() {
    // Three bags declared, two written.
    let short = "s td 3 2 3\nb 1 1 2\nb 2 2 3\n1 2\n";
    let err = parse_pace_td(short, GraphKind::Primal, 3)
        .map(|_| ())
        .expect_err("a file defining fewer bags than it declares is not a decomposition")
        .to_string();
    assert!(
        err.contains('3') && err.contains('2'),
        "the message must name both counts, got: {err}",
    );

    // The same disagreement reached the other way: as many bag lines as
    // declared, but one bag defined twice and another not at all.
    let repeated = "s td 3 2 3\nb 1 1 2\nb 1 2 3\nb 3 3\n";
    let err = parse_pace_td(repeated, GraphKind::Primal, 3)
        .map(|_| ())
        .expect_err("a bag defined twice leaves another undefined")
        .to_string();
    assert!(
        err.contains('1'),
        "the message must name the repeated bag, got: {err}",
    );

    // The well-formed file: one line per declared bag, and the two lists agree.
    let whole = parse_pace_td(
        "s td 3 2 3\nb 1 1 2\nb 2 2 3\nb 3 3\n1 2\n2 3\n",
        GraphKind::Primal,
        3,
    )
    .expect("a file defining each declared bag once");
    assert_eq!(whole.bags.len(), whole.adj.len());
    for (position, bag) in whole.bags.iter().enumerate() {
        assert_eq!(bag.id, position, "bag ids index the adjacency");
    }
    assert!(whole.adj[1].contains(&0) && whole.adj[1].contains(&2));
}

/// The treewidth is one less than the largest bag, and a decomposition with
/// nothing in it is width zero rather than an underflow: no bags, one empty bag
/// and one single-vertex bag all decompose something that needs no separator at
/// all.
#[test]
fn treewidth_is_the_largest_bag_less_one_and_zero_when_there_is_nothing_to_decompose() {
    let cases = [
        (Vec::new(), 0, "no bags at all"),
        (vec![Vec::new()], 0, "one bag with nothing in it"),
        (vec![vec![0]], 0, "one bag holding one vertex"),
        (
            vec![vec![0, 1], vec![1, 2], vec![2, 3]],
            1,
            "a path of two-vertex bags",
        ),
        (vec![vec![0, 1], vec![1, 2, 3]], 2, "a widest bag of three"),
    ];
    for (bags, expected, what) in cases {
        let decomposition = td(GraphKind::Primal, bags, 4);
        assert_eq!(
            decomposition.treewidth(),
            expected,
            "{what} has width {expected}",
        );
    }
}

/// A decomposition of the incidence graph carries a vertex per clause as well
/// as per variable, and a consumer indexing a variable-length array by them
/// would run off the end. Which ids are variables is the vertex count the
/// decomposition was built over, exclusive: the last variable id is one below
/// it.
#[test]
fn an_incidence_decomposition_hides_its_clause_vertices_while_a_primal_one_keeps_every_id() {
    let bag_vertices = vec![0, 2, 3, 5];
    let num_vars = 3;

    let incidence = td(GraphKind::Incidence, vec![bag_vertices.clone()], num_vars);
    let kept: Vec<u32> = incidence
        .variable_vertices(&incidence.bags[0])
        .collect::<Vec<_>>();
    assert_eq!(
        kept,
        vec![0, 2],
        "vertex 2 is the last variable and vertex 3 the first clause",
    );

    let primal = td(GraphKind::Primal, vec![bag_vertices.clone()], num_vars);
    let kept: Vec<u32> = primal.variable_vertices(&primal.bags[0]).collect();
    assert_eq!(
        kept, bag_vertices,
        "a primal decomposition has no clause vertices to leave out",
    );
}
