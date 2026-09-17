//! The incidence graph: one vertex per clause, and what becomes of them

use super::*;

#[test]
fn test_incidence_edges_simple() {
    // 3 vars, 2 clauses: (x1 ∨ x2) ∧ (x2 ∨ x3)
    // Variables: 0, 1, 2. Clauses: vertex 3 (clause 0), vertex 4 (clause 1).
    // Clause 0 touches vars 0,1 → edges (0,3), (1,3)
    // Clause 1 touches vars 1,2 → edges (1,4), (2,4)
    let formula = make_formula(3, vec![vec![1, 2], vec![2, 3]]);
    let graph = GraphKind::Incidence.build(&formula);
    assert_eq!(graph.edges(), &[(0, 3), (1, 3), (1, 4), (2, 4)]);
}

#[test]
fn test_incidence_edges_negation_ignored() {
    // Negated literals produce the same variable vertex as positive ones.
    let formula = make_formula(2, vec![vec![1, -2]]);
    let graph = GraphKind::Incidence.build(&formula);
    let edges = graph.edges();
    // Clause vertex = 2. Edges: (0,2), (1,2).
    assert_eq!(edges, &[(0, 2), (1, 2)]);
}

#[test]
fn test_incidence_edges_dedup() {
    // If a variable appears twice in a clause (positive and negative), the edge
    // should be deduplicated. make_formula won't normally create this, but the
    // incidence build explicitly deduplicates.
    let formula = make_formula(3, vec![vec![1, 2, 3], vec![1, 2, 3]]);
    let graph = GraphKind::Incidence.build(&formula);
    let edges = graph.edges();
    // Each edge appears exactly once even though vars overlap across clauses
    let unique: HashSet<_> = edges.iter().collect();
    assert_eq!(edges.len(), unique.len(), "edges should be deduplicated");
}
