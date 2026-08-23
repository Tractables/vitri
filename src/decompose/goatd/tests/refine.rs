use crate::decompose::TreeDecomposition;
use crate::decompose::goatd::refine::*;
use crate::tests::common::make_td;
use crate::tests::td_fixture::{assert_rip, is_connected};

fn trivial_td(num_vars: u32) -> TreeDecomposition {
    make_td(vec![(0..num_vars).collect()], Vec::new(), num_vars)
}

#[test]
fn refine_noop_on_tiny_subproblem() {
    // Under the min-side-size threshold: should return unchanged.
    let td = trivial_td(4);
    let vars: Vec<u32> = (0..4).collect();
    let edges: Vec<(u32, u32)> = vec![(0, 1), (1, 2), (2, 3)];
    let out = refine_td_with_flowcutter_cut(td.clone(), &vars, &edges, None);
    assert_eq!(out.bags.len(), td.bags.len());
    assert_eq!(out.width(), td.width());
}

#[test]
fn refine_preserves_coverage_and_rip_on_path() {
    // 32-var path graph.  FlowCutter should cut it cleanly.
    let num_vars = 32u32;
    let vars: Vec<u32> = (0..num_vars).collect();
    let edges: Vec<(u32, u32)> = (0..num_vars - 1).map(|i| (i, i + 1)).collect();

    // Start from a deliberately bad TD: one giant bag containing every var.
    let td = trivial_td(num_vars);

    let out = refine_td_with_flowcutter_cut(td.clone(), &vars, &edges, None);

    let mut covered = std::collections::HashSet::new();
    for bag in &out.bags {
        for &v in &bag.vertices {
            covered.insert(v);
        }
    }
    for v in 0..num_vars {
        assert!(covered.contains(&v), "variable {} lost after refinement", v);
    }

    assert!(is_connected(&out));
    assert_rip(&out);
}
