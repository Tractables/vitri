//! The graphs a formula builds, and the PACE `.gr` text they render to

use super::*;

#[test]
fn test_pace_graph_to_gr_primal() {
    // 3 variables, 2 clauses: (1 ∨ 2) ∧ (2 ∨ 3)
    // Edges: (0,1), (1,2) — 0-indexed
    let formula = make_formula(3, vec![vec![1, 2], vec![2, 3]]);
    let graph = GraphKind::Primal.build(&formula);
    assert_eq!(graph.kind(), GraphKind::Primal);
    assert_eq!(graph.num_vertices(), 3);
    assert_eq!(graph.edges(), &[(0, 1), (1, 2)]);
    assert_eq!(graph.to_gr(), "c vitri primal graph\np tw 3 2\n1 2\n2 3\n");
}

#[test]
fn test_pace_graph_to_gr_incidence() {
    // The same formula's incidence graph: variables 0..3, then one vertex per
    // clause, and one edge per literal.
    let formula = make_formula(3, vec![vec![1, 2], vec![2, 3]]);
    let graph = GraphKind::Incidence.build(&formula);
    assert_eq!(graph.kind(), GraphKind::Incidence);
    assert_eq!(graph.num_vertices(), 5);
    assert_eq!(graph.edges(), &[(0, 3), (1, 3), (1, 4), (2, 4)]);
    assert_eq!(
        graph.to_gr(),
        "c vitri incidence graph\np tw 5 4\n1 4\n2 4\n2 5\n3 5\n"
    );
}

#[test]
fn test_primal_edges_dedup() {
    // Clause (1 ∨ 2 ∨ 3) generates edges (0,1),(0,2),(1,2); two clauses same edge dedup
    let formula = make_formula(3, vec![vec![1, 2], vec![1, 2, 3]]);
    let graph = GraphKind::Primal.build(&formula);
    let edges = graph.edges();
    let mut sorted = edges.to_vec();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        edges,
        sorted.as_slice(),
        "the primal graph's edges must be sorted and deduped"
    );
}
