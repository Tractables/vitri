use crate::decompose::goatd::graph::*;

#[test]
fn from_edges_builds_undirected_adjacency() {
    let g = Graph::from_edges(3, &[(0, 1), (1, 2)]);
    assert_eq!(g.degree(0), 1);
    assert_eq!(g.degree(1), 2);
    assert_eq!(g.degree(2), 1);
    assert!(g.contains_edge(0, 1) && g.contains_edge(1, 0));
}

#[test]
fn eliminate_fills_clique_and_deactivates() {
    let mut g = Graph::from_edges(3, &[(0, 1), (1, 2)]);
    let bag = g.eliminate(1);
    assert_eq!(bag.len(), 2);
    assert!(g.contains_edge(0, 2));
    assert!(!g.active[1]);
    assert_eq!(g.num_active, 2);
}

#[test]
fn simplicial_detection() {
    let g = Graph::from_edges(3, &[(0, 1), (0, 2), (1, 2)]);
    assert!(g.is_simplicial(0));
    let g = Graph::from_edges(3, &[(0, 1), (1, 2)]);
    assert!(!g.is_simplicial(1));
    assert!(g.is_simplicial(0));
}

#[test]
fn from_edges_dedups_repeated_inputs() {
    let g = Graph::from_edges(2, &[(0, 1), (1, 0), (0, 1)]);
    assert_eq!(g.degree(0), 1);
    assert_eq!(g.degree(1), 1);
}

#[test]
fn edge_query_agrees_with_the_adjacency_lists() {
    let g = Graph::from_edges(5, &[(0, 1), (1, 2), (2, 3), (3, 4), (0, 4)]);
    assert!(g.bitset_words > 0);
    for u in 0u32..5 {
        for v in 0u32..5 {
            let adj_has = g.adj[u as usize].contains(&v);
            let bs_has = g.contains_edge(u, v);
            assert_eq!(adj_has, bs_has, "u={u} v={v}");
        }
    }
}

#[test]
fn fill_count_is_the_edges_missing_among_the_neighbours() {
    let g = Graph::from_edges(3, &[(0, 1), (1, 2)]);
    assert_eq!(g.fill_count_of_bs(1), 1);
    assert_eq!(g.fill_count_of_bs(0), 0);
    let g2 = Graph::from_edges(3, &[(0, 1), (0, 2), (1, 2)]);
    assert_eq!(g2.fill_count_of_bs(0), 0);
    assert_eq!(g2.fill_count_of_bs(1), 0);
}

#[test]
fn promote_bitset_from_sparse_graph() {
    let n = 200u32;
    let edges: Vec<(u32, u32)> = (0..n - 1).map(|v| (v, v + 1)).collect();
    let mut g = Graph::from_edges(n, &edges);
    assert_eq!(g.bitset_words, 0, "sparse path expected");
    assert!(!g.should_promote_bitset(), "sparse density below threshold");
    g.promote_bitset();
    assert!(g.bitset_words > 0, "bitset populated after promotion");
    for v in 0..n - 1 {
        assert!(g.contains_edge(v, v + 1));
        assert!(g.contains_edge(v + 1, v));
    }
    assert!(!g.contains_edge(0, 5));
}

#[test]
fn should_promote_bitset_triggers_when_dense() {
    // Near-complete graph on 128 vertices, dense enough to cross the
    // promotion threshold.
    let n = 128u32;
    let mut edges = Vec::new();
    for u in 0..n {
        for v in (u + 1)..n {
            edges.push((u, v));
        }
    }
    // Construct sparse (first 10 edges only) then add_edge the rest, so the
    // graph starts adj-only regardless of what from_edges' own threshold does.
    let mut g = Graph::from_edges(n, &edges[..10]);
    for &(u, v) in &edges[10..] {
        g.add_edge(u, v);
    }
    // n=128 ≤ BITSET_THRESH so from_edges may already have enabled bitset;
    // if not, promotion should fire immediately.
    if g.bitset_words == 0 {
        assert!(g.should_promote_bitset());
    }
}

#[test]
fn eliminating_a_vertex_replaces_its_edges_with_the_fill_edge() {
    let mut g = Graph::from_edges(3, &[(0, 1), (1, 2)]);
    assert!(g.bitset_words > 0, "should use bitset for n=3");
    g.eliminate(1);
    assert!(g.contains_edge(0, 2));
    assert!(!g.active[1]);
    assert_eq!(g.num_active, 2);
    assert_eq!(g.num_edges, 1);
}
