//! Vtree rotations.
//!
//! Shared by both topics: the structural check [`assert_invariants`], the tree
//! comparison [`assert_equal`], and the test-only inverse rotations that undo
//! a rotation by pointer surgery so a round trip can be compared against the
//! tree it started from.

use crate::vtree::RotationKind;
use crate::vtree::Vtree;
use crate::vtree::VtreeIdx;
use crate::vtree::VtreeNode;
use crate::vtree::rotate::*;

mod inverse;
mod topo_fixup;

/// Verify parent pointers are consistent and topo order is a valid
/// bottom-up linearization (children-before-parents) AND satisfies the
/// root-last property used by `Vtree::fixup_topo_after_rotate`.
fn assert_invariants(vtree: &Vtree) {
    let n = vtree.num_nodes();
    assert_eq!(vtree.bottomup_topo().len(), n, "topo length mismatch");
    let mut seen = vec![false; n];
    for &t in vtree.bottomup_topo() {
        assert!(!seen[t.idx()], "duplicate {t:?} in topo");
        seen[t.idx()] = true;
    }
    assert!(seen.iter().all(|&b| b), "topo missing a node");
    let mut filt_seen = vec![false; n];
    for (t, _, _) in vtree.internal_bottomup() {
        assert!(!filt_seen[t.idx()], "duplicate {t:?} in internal_topo");
        filt_seen[t.idx()] = true;
    }
    for (t, _) in vtree.leaf_bottomup() {
        assert!(!filt_seen[t.idx()], "duplicate {t:?} in leaf_topo");
        filt_seen[t.idx()] = true;
    }
    assert!(
        filt_seen.iter().all(|&b| b),
        "internal_topo + leaf_topo missing a node"
    );

    for (t, left, right) in vtree.internal_bottomup() {
        assert_eq!(vtree.node(left).parent(), Some(t));
        assert_eq!(vtree.node(right).parent(), Some(t));
        assert!(
            vtree.topo_pos(left) < vtree.topo_pos(t),
            "left child {left:?} not before parent {t:?} in topo"
        );
        assert!(
            vtree.topo_pos(right) < vtree.topo_pos(t),
            "right child {right:?} not before parent {t:?} in topo"
        );
    }
    let n = vtree.num_nodes();
    let mut subtree_max: Vec<u32> = (0..n as u32).map(|_| 0).collect();
    for &t in vtree.bottomup_topo() {
        let pos = vtree.topo_pos(t);
        let m = match vtree.node(t) {
            VtreeNode::Leaf { .. } => pos,
            VtreeNode::Internal { left, right, .. } => pos
                .max(subtree_max[left.idx()])
                .max(subtree_max[right.idx()]),
        };
        subtree_max[t.idx()] = m;
        assert_eq!(
            m, pos,
            "root-last violated at {t:?}: topo_pos={pos} but subtree max={m}",
        );
    }
}

fn assert_equal(a: &Vtree, b: &Vtree) {
    assert_eq!(a.num_nodes(), b.num_nodes());
    for i in 0..a.num_nodes() {
        let idx = VtreeIdx(i as u32);
        match (a.node(idx), b.node(idx)) {
            (
                VtreeNode::Leaf {
                    var: v1,
                    parent: p1,
                },
                VtreeNode::Leaf {
                    var: v2,
                    parent: p2,
                },
            ) => {
                assert_eq!(v1, v2);
                assert_eq!(p1, p2);
            }
            (
                VtreeNode::Internal {
                    left: l1,
                    right: r1,
                    parent: p1,
                },
                VtreeNode::Internal {
                    left: l2,
                    right: r2,
                    parent: p2,
                },
            ) => {
                assert_eq!(l1, l2);
                assert_eq!(r1, r2);
                assert_eq!(p1, p2);
            }
            _ => panic!("node type mismatch at {idx:?}"),
        }
    }
}

/// Undo a left rotation, pointer surgery only — does NOT update `Vtree::topo`.
/// Mirror of `rotate_left_pointers`; see its doc for usage.
fn unrotate_left_pointers(vtree: &mut Vtree, info: &RotationInfo) {
    let RotationInfo {
        v_idx,
        w_idx,
        a_idx,
        b_idx,
        c_idx,
    } = *info;
    let v_parent = vtree.nodes[v_idx.idx()].parent();

    vtree.nodes[v_idx.idx()] = VtreeNode::Internal {
        left: a_idx,
        right: w_idx,
        parent: v_parent,
    };
    vtree.nodes[w_idx.idx()] = VtreeNode::Internal {
        left: b_idx,
        right: c_idx,
        parent: Some(v_idx),
    };
    Vtree::set_parent(&mut vtree.nodes, a_idx, v_idx);
    Vtree::set_parent(&mut vtree.nodes, c_idx, w_idx);
}

/// Undo a left rotation given its `RotationInfo`. Equivalent to a right rotation
/// at `v_idx` for trees that came from a left rotation.
fn unrotate_left(vtree: &mut Vtree, info: &RotationInfo) {
    unrotate_left_pointers(vtree, info);
    // unrotate_left ≡ right rotation on the post-left-rot tree. The
    // RotationInfo's a/b/c happen to match the right-rotation conventions
    // (right rot's `a` is the post-left-rot's `w.left` = original `a`,
    // similarly for b and c), so we can pass `info` straight through.
    vtree.fixup_topo_after_rotate(info, RotationKind::Right);
}

/// Undo a right rotation, pointer surgery only — does NOT update `Vtree::topo`.
fn unrotate_right_pointers(vtree: &mut Vtree, info: &RotationInfo) {
    let RotationInfo {
        v_idx,
        w_idx,
        a_idx,
        b_idx,
        c_idx,
    } = *info;
    let v_parent = vtree.nodes[v_idx.idx()].parent();

    vtree.nodes[v_idx.idx()] = VtreeNode::Internal {
        left: w_idx,
        right: c_idx,
        parent: v_parent,
    };
    vtree.nodes[w_idx.idx()] = VtreeNode::Internal {
        left: a_idx,
        right: b_idx,
        parent: Some(v_idx),
    };
    Vtree::set_parent(&mut vtree.nodes, a_idx, w_idx);
    Vtree::set_parent(&mut vtree.nodes, c_idx, v_idx);
}

/// Undo a right rotation given its `RotationInfo`.
fn unrotate_right(vtree: &mut Vtree, info: &RotationInfo) {
    unrotate_right_pointers(vtree, info);
    // unrotate_right ≡ left rotation on the post-right-rot tree. The
    // RotationInfo's a/b/c match left-rotation conventions on this side too.
    vtree.fixup_topo_after_rotate(info, RotationKind::Left);
}

impl Vtree {
    /// Recompute `topo`, `topo_pos`, `internal_topo`, `leaf_topo` from the
    /// current parent/child structure. `O(n_nodes)` iterative postorder — the
    /// reference the incremental `fixup_topo_after_rotate` is checked against.
    fn rebuild_topo(&mut self) {
        let n = self.nodes.len();
        self.topo.clear();
        self.topo.reserve(n);
        self.internal_topo.clear();
        self.leaf_topo.clear();
        if self.topo_pos.len() != n {
            self.topo_pos.resize(n, 0);
        }

        let mut stack: Vec<(VtreeIdx, bool)> = Vec::with_capacity(n);
        stack.push((self.root, false));
        while let Some((idx, done)) = stack.pop() {
            if done {
                self.topo_pos[idx.idx()] = self.topo.len() as u32;
                self.topo.push(idx);
                if self.nodes[idx.idx()].is_leaf() {
                    self.leaf_topo.push(idx);
                } else {
                    self.internal_topo.push(idx);
                }
            } else {
                stack.push((idx, true));
                if let VtreeNode::Internal { left, right, .. } = self.nodes[idx.idx()] {
                    // Push right first so left is popped first → left visited
                    // before right (matches the bottom-up order used elsewhere).
                    stack.push((right, false));
                    stack.push((left, false));
                }
            }
        }
        debug_assert_eq!(
            self.topo.len(),
            n,
            "topo missed nodes (disconnected vtree?)"
        );
    }

    /// Bottom-up topological order over all nodes (children before parents).
    /// Stable across rotations: walks the side `topo` array, not raw indices.
    /// A test-side view of the order the production paths consume through
    /// `topo_pos`.
    #[inline]
    fn bottomup_topo(&self) -> &[VtreeIdx] {
        &self.topo
    }
}
