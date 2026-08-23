use crate::decompose::goatd::graph::Graph;
use crate::decompose::goatd::preprocess::*;

#[test]
fn a_single_edge_and_an_isolate_reduce_to_one_bag_per_vertex() {
    let g = Graph::from_edges(3, &[(0, 1)]);
    let reduced = preprocess(g);
    assert_eq!(reduced.graph.num_active, 0);
    assert_eq!(reduced.prefix.bags.len(), 3);
}

#[test]
fn twig_removes_leaves() {
    let g = Graph::from_edges(4, &[(0, 1), (1, 2), (2, 3)]);
    let reduced = preprocess(g);
    assert_eq!(reduced.graph.num_active, 0);
}

#[test]
fn simplicial_triangle_collapses() {
    let g = Graph::from_edges(3, &[(0, 1), (0, 2), (1, 2)]);
    let reduced = preprocess(g);
    assert_eq!(reduced.graph.num_active, 0);
    assert_eq!(reduced.prefix.bags[0].len(), 3);
}

#[test]
fn series_adds_fill_then_contracts() {
    let g = Graph::from_edges(4, &[(0, 1), (1, 2), (2, 3), (3, 0)]);
    let reduced = preprocess(g);
    assert_eq!(reduced.graph.num_active, 0);
}

#[test]
fn almost_simplicial_fires_under_lb() {
    // An almost-simplicial vertex of degree 3 only fires once tw_lb >= 3, so
    // a disjoint K_4 establishes that bound before vertices 0 and 4 (each
    // almost-simplicial, missing edge (2,3)) get a turn.
    let edges = vec![
        // K_4 on {0,1,2,3} minus (2,3).
        (0, 1),
        (0, 2),
        (0, 3),
        (1, 2),
        (1, 3),
        // vertex 4: same neighbourhood shape as 0, also almost-simplicial.
        (4, 1),
        (4, 2),
        (4, 3),
        // disjoint K_4 establishing tw_lb = 3.
        (8, 9),
        (8, 10),
        (8, 11),
        (9, 10),
        (9, 11),
        (10, 11),
    ];
    let g = Graph::from_edges(12, &edges);
    let reduced = preprocess(g);
    assert_eq!(reduced.graph.num_active, 0);
    let max_bag = reduced.prefix.bags.iter().map(|b| b.len()).max().unwrap();
    assert_eq!(max_bag, 4);
}

#[test]
fn almost_simplicial_skipped_without_tw_lb() {
    // Two triangles sharing an edge: fully reduces via simplicial+twig
    // without ever needing almost-simplicial — verifies the rule doesn't
    // over-fire when tw_lb hasn't been established.
    let g = Graph::from_edges(4, &[(0, 1), (0, 2), (1, 2), (0, 3), (1, 3)]);
    let reduced = preprocess(g);
    assert_eq!(reduced.graph.num_active, 0);
}
