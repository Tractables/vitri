use crate::decompose::goatd::graph::Graph;
use crate::decompose::goatd::minfill_core::heap_fill::*;
use crate::decompose::goatd::minfill_core::{ElimSink, ElimStop};

#[test]
fn path_graph_eliminates_from_endpoints() {
    let mut g = Graph::from_edges(4, &[(0, 1), (1, 2), (2, 3)]);
    let salt = vec![0u32; 4];
    let mut bags = Vec::new();
    let mut rank = Vec::new();
    let sink = ElimSink::new(&mut bags, &mut rank, 0);
    eliminate_minfill(&mut g, &salt, sink, ElimStop::default());
    assert_eq!(bags.len(), 4);
    let first = bags[0][0];
    assert!(first == 0 || first == 3);
    assert_eq!(g.num_active, 0);
}

#[test]
fn triangle_eliminates_in_three_steps() {
    let mut g = Graph::from_edges(3, &[(0, 1), (0, 2), (1, 2)]);
    let salt = vec![0u32; 3];
    let mut bags = Vec::new();
    let mut rank = Vec::new();
    let sink = ElimSink::new(&mut bags, &mut rank, 0);
    eliminate_minfill(&mut g, &salt, sink, ElimStop::default());
    assert_eq!(bags.len(), 3);
    assert_eq!(bags[0].len(), 3);
}
