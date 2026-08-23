use super::*;

/// Apply a rotation two ways — pointer-only + `fixup_topo_after_rotate`
/// vs pointer-only + full `rebuild_topo` — and check that both produce
/// vtrees satisfying the topo invariants. The two `topo` arrays may
/// differ (fixup doesn't promise strict postorder), but each must be a
/// valid bottom-up order satisfying the root-last property.
fn check_fixup_equivalence_left(mut vtree: Vtree, v: VtreeIdx) {
    let mut via_rebuild = vtree.clone();
    let info_a = rotate_left_pointers(&mut vtree, v).expect("applicable");
    vtree.fixup_topo_after_rotate(&info_a, RotationKind::Left);
    assert_invariants(&vtree);

    let info_b = rotate_left_pointers(&mut via_rebuild, v).expect("applicable");
    via_rebuild.rebuild_topo();
    assert_invariants(&via_rebuild);
    assert_eq!(info_a.v_idx, info_b.v_idx);
    assert_eq!(info_a.w_idx, info_b.w_idx);
}

fn check_fixup_equivalence_right(mut vtree: Vtree, v: VtreeIdx) {
    let mut via_rebuild = vtree.clone();
    let info_a = rotate_right_pointers(&mut vtree, v).expect("applicable");
    vtree.fixup_topo_after_rotate(&info_a, RotationKind::Right);
    assert_invariants(&vtree);

    let info_b = rotate_right_pointers(&mut via_rebuild, v).expect("applicable");
    via_rebuild.rebuild_topo();
    assert_invariants(&via_rebuild);
    assert_eq!(info_a.v_idx, info_b.v_idx);
    assert_eq!(info_a.w_idx, info_b.w_idx);
}

#[test]
fn fixup_equivalence_left_at_root() {
    check_fixup_equivalence_left(Vtree::linear(5), Vtree::linear(5).root);
}

#[test]
fn fixup_equivalence_right_at_root_balanced() {
    // balanced(4) is the case the early-exit cannot use (right rotation
    // pre-fixup has C after w in topo) — exercises the slice-rotate path.
    check_fixup_equivalence_right(Vtree::balanced(4), Vtree::balanced(4).root);
}

#[test]
fn fixup_equivalence_right_at_root_balanced_8() {
    check_fixup_equivalence_right(Vtree::balanced(8), Vtree::balanced(8).root);
}

#[test]
fn fixup_equivalence_at_internal_non_root() {
    let vtree = Vtree::balanced(8);
    let target = (0..vtree.num_nodes() as u32)
        .map(VtreeIdx)
        .find(|&t| {
            if let VtreeNode::Internal {
                left,
                right,
                parent,
            } = *vtree.node(t)
            {
                parent.is_some() && !vtree.node(left).is_leaf() && !vtree.node(right).is_leaf()
            } else {
                false
            }
        })
        .expect("balanced(8) has at least one such node");
    check_fixup_equivalence_left(vtree.clone(), target);
    check_fixup_equivalence_right(vtree, target);
}

/// Round-trip: a sequence of rotations followed by their inverses (via
/// `unrotate_*`) restores the structure bit-for-bit and produces a
/// vtree that still satisfies the topo invariants at every step.
#[test]
fn fixup_round_trip_random_sequence() {
    let mut vtree = Vtree::balanced(8);
    let original = vtree.clone();
    let mut history: Vec<(RotationInfo, RotKindLocal)> = Vec::new();

    // Deterministic pseudo-random walk: try a left rotation at every
    // internal node bottom-up, then a right rotation, recording each
    // applicable success.
    let internal: Vec<VtreeIdx> = vtree
        .bottomup_topo()
        .iter()
        .copied()
        .filter(|&t| !vtree.node(t).is_leaf())
        .collect();
    for &t in &internal {
        if let Some(info) = rotate_left(&mut vtree, t) {
            assert_invariants(&vtree);
            history.push((info, RotKindLocal::Left));
        }
        if let Some(info) = rotate_right(&mut vtree, t) {
            assert_invariants(&vtree);
            history.push((info, RotKindLocal::Right));
        }
    }
    while let Some((info, kind)) = history.pop() {
        match kind {
            RotKindLocal::Left => unrotate_left(&mut vtree, &info),
            RotKindLocal::Right => unrotate_right(&mut vtree, &info),
        }
        assert_invariants(&vtree);
    }
    assert_equal(&vtree, &original);
}

/// Edge case: rotation where the misplaced subtree's root is a leaf.
/// This is the smallest possible misplaced subtree. The slice has at
/// most two elements and `rotate_left(1)` is a single swap.
#[test]
fn fixup_handles_single_node_misplaced_subtree() {
    let mut vtree = Vtree::linear(4);
    let root = vtree.root;
    let info = rotate_left(&mut vtree, root).expect("applicable");
    assert_invariants(&vtree);
    unrotate_left(&mut vtree, &info);
    assert_invariants(&vtree);
}

/// Edge case: two consecutive rotations at the same node.
#[test]
fn fixup_handles_consecutive_rotations_at_same_node() {
    let mut vtree = Vtree::balanced(8);
    let root = vtree.root;
    let _ = rotate_left(&mut vtree, root).expect("applicable");
    assert_invariants(&vtree);
    // After a left rotation at root, the new root's right child is what
    // was C (an internal subtree in balanced(8)) — try a left rotation
    // again. Applicability depends on whether new C's right subtree is
    // internal; if not, this is a no-op None.
    let _ = rotate_left(&mut vtree, root);
    assert_invariants(&vtree);
}

// Mirrors the crate-private `RotKind` — tests can't reference it directly.
#[derive(Copy, Clone, Debug)]
enum RotKindLocal {
    Left,
    Right,
}
