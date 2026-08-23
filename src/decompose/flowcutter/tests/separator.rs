use crate::decompose::flowcutter::*;
use crate::decompose::flowcutter_rs::flowcutter_compute_separator;
use crate::decompose::{FcBudget, GraphKind};

#[test]
fn separator_on_path_graph_is_small() {
    let n = 10;
    let edges: Vec<(u32, u32)> = (0..9).map(|i| (i, i + 1)).collect();
    let r = flowcutter_compute_separator(n, &edges, 10_000, 3, 0).expect("separator on path");
    assert_eq!(r.separator.len() + r.side_a.len() + r.side_b.len(), n);
    assert!(!r.side_a.is_empty());
    assert!(!r.side_b.is_empty());
    assert!(
        r.separator.len() <= 2,
        "path separator too big: {:?}",
        r.separator
    );
}

/// Regression: FlowCutter on a dense incidence graph must not SIGSEGV from a
/// bag-adjacency blow-up.
///
/// The fixture is generated in-crate ([`super::dense_fixture`]): 43
/// variables carrying 43162 clauses, the shape a heavily preprocessed model
/// counting instance collapses to. Its incidence graph has small treewidth but
/// heavy clique-tree overlap: `output_tree_decompostion_of_order` (in the
/// vendored FlowCutter) materialised a junction-tree intersection graph of
/// ~1e9 arcs — `tail2`/`head2`/`weight` plus a billion-entry `max_id_heap`,
/// tens of GB — and the process died touching unbacked pages. The min-degree
/// elimination order (the first order FlowCutter tests) triggers it, so a tiny
/// `iters` budget still reproduces it on unfixed code.
///
/// Fix: `IFlowCutter::output_tree_decompostion_of_order` caps the arc count
/// (`kMaxBagAdjacencyArcs`) and throws past it; `test_new_order` swallows the
/// throw without polluting `best_bag_size`, so the multilevel-partition path
/// still yields a valid decomposition. The call here must therefore RETURN
/// (Ok or Err) rather than crash the test binary.
#[test]
#[ignore = "slow: ~5s of min-degree ordering on a 43k-clause pathological fixture, several times the rest of the suite. Segfault regression guard; run with --include-ignored."]
fn flowcutter_incidence_no_segv_on_bag_adjacency_explosion() {
    let formula = super::dense_fixture::bag_adjacency_explosion();
    // Small iters: the blow-up is in the first (min-degree) order, so a tight
    // budget reproduces the pre-fix crash while keeping the post-fix path fast.
    let result = flowcutter_td(
        &formula,
        GraphKind::Incidence,
        FcBudget::Steps {
            steps: 5_000,
            iters: 2,
        },
    );
    // The guarantee is process survival: reaching this line means no SIGSEGV.
    // Either outcome is acceptable (Err = candidate skipped, Ok = multilevel TD).
    assert!(
        result.is_ok() || result.is_err(),
        "an incidence-graph decomposition must return rather than crash"
    );
}

#[test]
fn separator_on_disconnected_returns_none_or_valid() {
    // Two disjoint triangles.  FlowCutter may return an empty separator
    // (graph already disconnected) which our wrapper turns into None.
    let n = 6;
    let edges = vec![(0, 1), (1, 2), (0, 2), (3, 4), (4, 5), (3, 5)];
    let r = flowcutter_compute_separator(n, &edges, 10_000, 3, 0);
    if let Some(r) = r {
        assert!(!r.side_a.is_empty());
        assert!(!r.side_b.is_empty());
    }
}
