use crate::decompose::multilevel_bisect::graph::{CsrGraph, build_csr};
use crate::decompose::multilevel_bisect::initial::edge_cut;

/// Two partitions of one graph whose cuts differ by a single edge have to
/// score differently, because every caller picks between partitions with `<`.
#[test]
fn a_one_edge_difference_separates_two_partitions() {
    // 0 is adjacent to all of 1, 2, 3; 1 and 2 are also adjacent.
    let graph = build_csr(4, &[(0, 1), (0, 2), (0, 3), (1, 2)]);

    // Side 0 = {0}: all three of its edges cross.
    assert_eq!(edge_cut(&graph, &[0, 1, 1, 1]), 3);
    // Side 0 = {1, 2}: 1–2 stays inside, so only 0–1 and 0–2 cross.
    assert_eq!(edge_cut(&graph, &[1, 0, 0, 1]), 2);
}

/// The score is the summed *weight* of the cut edges — coarsening is what
/// produces weights above 1, and an odd total must survive intact.
#[test]
fn a_cut_edge_contributes_its_whole_weight() {
    let graph = CsrGraph {
        xadj: vec![0, 1, 2],
        adjncy: vec![1, 0],
        vwgt: vec![1, 1],
        adjwgt: vec![5, 5],
    };

    assert_eq!(edge_cut(&graph, &[0, 1]), 5);
    assert_eq!(edge_cut(&graph, &[0, 0]), 0);
}
