use crate::decompose::TdBag;
use crate::decompose::td_ops::*;
use crate::decompose::{GraphKind, TreeDecomposition};
use crate::tests::td_fixture::{assert_rip, is_connected, make_test_td};
use rustc_hash::FxHashSet;

#[test]
fn glue_at_separator_preserves_rip_and_covers_all_vars() {
    // Side A: two bags covering vars {0, 1, 2, 3}; S = {2, 3}.
    let td_a = TreeDecomposition {
        kind: GraphKind::Primal,
        bags: vec![
            TdBag {
                id: 0,
                vertices: vec![0, 1, 2],
            },
            TdBag {
                id: 1,
                vertices: vec![1, 2, 3],
            },
        ],
        adj: vec![vec![1], vec![0]],
        num_vars: 6,
    };
    // Side B: two bags covering vars {2, 3, 4, 5}; S = {2, 3}.
    let td_b = TreeDecomposition {
        kind: GraphKind::Primal,
        bags: vec![
            TdBag {
                id: 0,
                vertices: vec![2, 3, 4],
            },
            TdBag {
                id: 1,
                vertices: vec![3, 4, 5],
            },
        ],
        adj: vec![vec![1], vec![0]],
        num_vars: 6,
    };
    let sep = vec![2u32, 3u32];
    let glued = glue_at_separator(td_a, td_b, &sep, 6).expect("glue should succeed");

    let mut seen: FxHashSet<u32> = FxHashSet::default();
    for bag in &glued.bags {
        for &v in &bag.vertices {
            seen.insert(v);
        }
    }
    for v in 0..6 {
        assert!(seen.contains(&v), "var {} missing from glued TD", v);
    }

    assert_rip(&glued);

    // Root bag (index 0) is the separator bag.
    assert_eq!(glued.bags[0].vertices, vec![2, 3]);
}

#[test]
fn glue_at_separator_handles_sep_not_in_any_single_bag() {
    // Side A has S vars spread across two bags: {0,1,2} and {1,3} — no single
    // bag contains {1, 2, 3}.  Augmentation should add var 2 to bag 1 (on the
    // path from its src bag 0 to the chosen anchor bag 1).
    let td_a = TreeDecomposition {
        kind: GraphKind::Primal,
        bags: vec![
            TdBag {
                id: 0,
                vertices: vec![0, 1, 2],
            },
            TdBag {
                id: 1,
                vertices: vec![1, 3],
            },
        ],
        adj: vec![vec![1], vec![0]],
        num_vars: 7,
    };
    // Side B has a single bag with all of S.
    let td_b = TreeDecomposition {
        kind: GraphKind::Primal,
        bags: vec![TdBag {
            id: 0,
            vertices: vec![1, 2, 3, 4, 5, 6],
        }],
        adj: vec![vec![]],
        num_vars: 7,
    };
    let sep = vec![1u32, 2u32, 3u32];
    let glued = glue_at_separator(td_a, td_b, &sep, 7).expect("glue should succeed");

    assert_rip(&glued);

    // Separator bag at index 0 contains exactly S.
    assert_eq!(glued.bags[0].vertices, vec![1, 2, 3]);
}

/// A side whose bags fall into two components has no path of bags between
/// them, so a separator variable living in the far one cannot be carried to the
/// anchor along one. Written into both ends regardless, its bags are
/// disconnected and the glued decomposition is not one.
#[test]
fn augmenting_a_disconnected_side_for_a_separator_keeps_the_running_intersection() {
    // Side A in two components, with one separator variable in each: whichever
    // bag is anchored, the other separator variable is across the gap.
    let td_a = TreeDecomposition {
        kind: GraphKind::Primal,
        bags: vec![
            TdBag {
                id: 0,
                vertices: vec![0, 1],
            },
            TdBag {
                id: 1,
                vertices: vec![2, 5],
            },
        ],
        adj: vec![vec![], vec![]],
        num_vars: 6,
    };
    // Side B holds the whole separator in one bag.
    let td_b = TreeDecomposition {
        kind: GraphKind::Primal,
        bags: vec![TdBag {
            id: 0,
            vertices: vec![0, 3, 4, 5],
        }],
        adj: vec![vec![]],
        num_vars: 6,
    };

    let glued = glue_at_separator(td_a, td_b, &[0u32, 5u32], 6).expect("glue should succeed");
    assert_rip(&glued);
    assert!(
        is_connected(&glued),
        "a decomposition is one tree, not several",
    );

    let mut seen: FxHashSet<u32> = FxHashSet::default();
    for bag in &glued.bags {
        for &v in &bag.vertices {
            seen.insert(v);
        }
    }
    for v in 0..6 {
        assert!(seen.contains(&v), "var {v} missing from the glued tree");
    }
}

#[test]
fn project_td_keeping_global_ids_preserves_ids() {
    let td = make_test_td();
    let keep: FxHashSet<u32> = [0, 1, 2, 3].iter().copied().collect();
    let proj = project_td_keeping_global_ids(&td, &keep, 6).unwrap();

    let mut seen: FxHashSet<u32> = FxHashSet::default();
    for bag in &proj.bags {
        for &v in &bag.vertices {
            assert!(
                keep.contains(&v),
                "projected bag contains non-kept var {}",
                v
            );
            seen.insert(v);
        }
    }
    for v in [0, 1, 2, 3] {
        assert!(seen.contains(&v), "var {} missing after projection", v);
    }
    assert_eq!(proj.num_vars, 6);
    assert_rip(&proj);
}

#[test]
fn project_td_full_set() {
    let td = make_test_td();
    let all: FxHashSet<u32> = (0..6).collect();
    let proj = project_td(&td, &all).unwrap();
    assert_eq!(proj.td.bags.len(), 3);
    assert_eq!(proj.td.num_vars, 6);
    assert_eq!(proj.local_to_global, vec![0, 1, 2, 3, 4, 5]);
}

#[test]
fn project_td_subset_removes_empty_bags() {
    let td = make_test_td();
    // Keep only variables {0, 1, 2} — bag2 ({3,4,5}) becomes empty.
    let keep: FxHashSet<u32> = [0, 1, 2].iter().copied().collect();
    let proj = project_td(&td, &keep).unwrap();
    // bag0 and bag1 survive (bag1 has {1,2} after projection).
    assert_eq!(proj.td.bags.len(), 2);
    assert_eq!(proj.td.num_vars, 3);
    assert_eq!(proj.local_to_global, vec![0, 1, 2]);
}

#[test]
fn project_td_contracts_through_empty() {
    // Keep {0, 1, 4, 5} — bag1 becomes empty, bag0 and bag2 should be connected.
    let td = TreeDecomposition {
        kind: GraphKind::Primal,
        bags: vec![
            TdBag {
                id: 0,
                vertices: vec![0, 1],
            },
            TdBag {
                id: 1,
                vertices: vec![2, 3],
            },
            TdBag {
                id: 2,
                vertices: vec![4, 5],
            },
        ],
        adj: vec![vec![1], vec![0, 2], vec![1]],
        num_vars: 6,
    };
    let keep: FxHashSet<u32> = [0, 1, 4, 5].iter().copied().collect();
    let proj = project_td(&td, &keep).unwrap();
    assert_eq!(proj.td.bags.len(), 2);
    assert!(proj.td.adj[0].contains(&1));
    assert!(proj.td.adj[1].contains(&0));
}

#[test]
fn project_td_single_variable() {
    let td = make_test_td();
    let keep: FxHashSet<u32> = [3].iter().copied().collect();
    let proj = project_td(&td, &keep).unwrap();
    // Variable 3 is in bag1 and bag2, so both survive (each with just {0} after renumbering).
    assert!(!proj.td.bags.is_empty());
    assert_eq!(proj.td.num_vars, 1);
}
