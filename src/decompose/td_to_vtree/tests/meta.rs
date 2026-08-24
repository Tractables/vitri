use crate::decompose::td_to_vtree::meta::*;

/// A path TD 0—1—2 rooted at bag 0 has BFS order [0,1,2]; ranks must be the
/// reverse, so every child ranks strictly below its parent.
#[test]
fn bag_rank_is_bottom_up_topological() {
    // var 0 in bag 0 (root), var 1 in bag 1, var 2 in bag 2 (deepest).
    let meta = BagMetadata::from_assignment(3, &[0, 1, 2], &[0, 1, 2], 3, 1);
    assert_eq!(meta.var_bag_rank(0), Some(2), "root bag ranks last");
    assert_eq!(meta.var_bag_rank(1), Some(1));
    assert_eq!(meta.var_bag_rank(2), Some(0), "deepest bag ranks first");
    assert_eq!(meta.num_bags(), 3);
    assert_eq!(meta.num_vars(), 3);
    assert_eq!(
        meta.treewidth(),
        1,
        "the width comes from the decomposition, not from the one-bag-per-variable assignment",
    );
}

#[test]
fn unassigned_var_has_no_rank() {
    let meta = BagMetadata::from_assignment(2, &[usize::MAX, 0], &[0], 1, 0);
    assert_eq!(meta.var_bag(0), None);
    assert_eq!(meta.var_bag_rank(0), None);
    assert_eq!(meta.var_bag_rank(1), Some(0));
}

#[test]
fn clause_rank_is_max_over_vars() {
    let meta = BagMetadata::from_assignment(3, &[0, 1, 2], &[0, 1, 2], 3, 1);
    // vars {1,2} → ranks {1,0} → max 1.
    assert_eq!(meta.clause_bag_rank([1u32, 2]), Some(1));
    // vars {0,2} → ranks {2,0} → max 2.
    assert_eq!(meta.clause_bag_rank([0u32, 2]), Some(2));
    assert_eq!(
        meta.clause_bag_rank([7u32]),
        None,
        "out-of-range var has no bag"
    );
}
