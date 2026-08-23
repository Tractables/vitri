use super::*;

#[test]
fn primal_graph_simplicial() {
    // Triangle: vars 0,1,2 all co-occur in one clause
    let f = make_formula(3, vec![vec![1, 2, 3]]);
    let g = PrimalGraph::new(3, &f.clauses);
    assert!(g.is_simplicial(0));
    assert!(g.is_simplicial(1));
    assert!(g.is_simplicial(2));
}

#[test]
fn primal_graph_not_simplicial() {
    // Path: 0-1, 1-2 but not 0-2
    let f = make_formula(3, vec![vec![1, 2], vec![2, 3]]);
    let g = PrimalGraph::new(3, &f.clauses);
    assert!(!g.is_simplicial(1));
    assert!(g.is_simplicial(0));
    assert!(g.is_simplicial(2));
}
