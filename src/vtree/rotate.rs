//! Vtree rotation primitives.
//!
//! A rotation is the local move for searching vtree space: a consumer scoring
//! vtrees under a cost model this crate does not have rewrites one edge and
//! rescores, instead of rebuilding a tree from scratch. That caller is why
//! these are public — this crate's own pipeline selects a vtree and stops, so
//! it never rotates one.
//!
//! A rotation changes the SHAPE and nothing else: the same leaves carry the
//! same variables afterwards, so it moves within the space of vtrees over one
//! variable set. [`rotate_left`] and [`rotate_right`] are therefore the two
//! moves a local-minimisation loop is built from — apply one, rescore under
//! your own cost, keep or undo. Undoing is the mirror rotation at the same
//! index; [`RotationInfo`] names the five nodes involved, which is also what a
//! consumer holding per-node state needs in order to invalidate exactly the
//! entries the move invalidated rather than all of them.
//!
//! Both take the index of the internal node to rotate at, which must be a node
//! of the vtree passed alongside it. Both leave the vtree untouched and return
//! `None` when the move is structurally impossible — `v` is a leaf, or the
//! child that would be promoted is.
//!
//! A **left rotation** at internal node `v` promotes `v`'s right child `w`:
//!
//! ```text
//! Before:     v_idx               After:      v_idx  (now w_new)
//!            / \                             / \
//!           A   w_idx                  w_idx   C
//!              / \                      / \
//!             B   C                   A   B
//! ```
//!
//! A **right rotation** at internal node `v` is the inverse — it promotes `v`'s
//! left child `w`:
//!
//! ```text
//! Before:     v_idx               After:      v_idx  (now w_new)
//!            / \                             / \
//!         w_idx C                           A   w_idx
//!         / \                                  / \
//!        A   B                               B   C
//! ```
//!
//! Both operations are O(1) pointer surgery on `(v_idx, w_idx)`. Raw `VtreeIdx`
//! values for nodes never change after construction; the side `topo` list on
//! `Vtree` is updated locally via `Vtree::fixup_topo_after_rotate` (the
//! convenience wrappers below also do this) so subsequent traversals (LCA,
//! bottom-up iteration) remain correct.
//!
//! # Topo order after a rotation
//!
//! `Vtree::topo` satisfies children-before-parents, and the rotations here
//! preserve a stronger property that `Vtree::fixup_topo_after_rotate` relies
//! on: for every node `t`, `topo_pos[t]` is the largest position among `t` and
//! its descendants. The fixup relocates the promoted node past the misplaced
//! subtree's segment as one block, which is what restores it.
//!
//! A subtree's members need not occupy a contiguous range of positions, and
//! nothing reading `topo`, `topo_pos`, `internal_topo` or `leaf_topo` indexes
//! one by range: they walk parent pointers and child links, or compare
//! `topo_pos` as a rank.

use super::{RotationKind, Vtree, VtreeIdx, VtreeNode};

/// The five nodes one rotation touched: the two that swapped depth and the
/// three subtree roots that changed parent.
///
/// Everything outside this set kept its parent, its children and its subtree,
/// so a consumer caching a value per node — a score, a width, a compiled
/// fragment — recomputes only for these and their ancestors, instead of
/// discarding the whole table. The same five indices name the mirror rotation
/// that undoes the move.
///
/// Node indices are stable identities, so these stay valid across further
/// rotations. Field naming follows the **left-rotation** geometry; right
/// rotation stores the same fields but with the corresponding subtrees.
#[derive(Clone, Copy, Debug)]
pub struct RotationInfo {
    /// Outer node index (parent before & after rotation).
    pub v_idx: VtreeIdx,
    /// Inner node index (the promoted/demoted child — same idx before & after).
    pub w_idx: VtreeIdx,
    /// Left rotation: was v's left child. Right rotation: was w's left child.
    pub a_idx: VtreeIdx,
    /// Left rotation: was w's left child. Right rotation: was w's right child.
    pub b_idx: VtreeIdx,
    /// Left rotation: was w's right child. Right rotation: was v's right child.
    pub c_idx: VtreeIdx,
}

/// Left-rotate the vtree at node `v`, promoting `v`'s right child `w`.
///
/// The leaf set is unchanged; only the shape is. [`rotate_right`] at the same
/// index undoes it.
///
/// `v` must be an index into `vtree`. Returns `None`, leaving `vtree`
/// untouched, if `v` or its right child is a leaf; always succeeds otherwise,
/// and the returned [`RotationInfo`] names the nodes the move touched. The
/// vtree is left fully consistent — a caller may read it, score it and rotate
/// it again without any repair step of its own.
pub fn rotate_left(vtree: &mut Vtree, v: VtreeIdx) -> Option<RotationInfo> {
    let (a, w, v_parent) = match vtree.nodes[v.idx()] {
        VtreeNode::Internal {
            left,
            right,
            parent,
        } => (left, right, parent),
        VtreeNode::Leaf { .. } => return None,
    };
    let (b, c) = match vtree.nodes[w.idx()] {
        VtreeNode::Internal { left, right, .. } => (left, right),
        VtreeNode::Leaf { .. } => return None,
    };

    // v_idx becomes w_new: children = (w_idx=v_new, C).
    vtree.nodes[v.idx()] = VtreeNode::Internal {
        left: w,
        right: c,
        parent: v_parent,
    };
    // w_idx becomes v_new: children = (A, B).
    vtree.nodes[w.idx()] = VtreeNode::Internal {
        left: a,
        right: b,
        parent: Some(v),
    };
    Vtree::set_parent(&mut vtree.nodes, a, w);
    Vtree::set_parent(&mut vtree.nodes, c, v);

    let info = RotationInfo {
        v_idx: v,
        w_idx: w,
        a_idx: a,
        b_idx: b,
        c_idx: c,
    };
    vtree.fixup_topo_after_rotate(&info, RotationKind::Left);
    Some(info)
}

/// Right-rotate the vtree at node `v`, promoting `v`'s left child `w`.
///
/// The mirror of [`rotate_left`], and what undoes one at the same index. The
/// leaf set is unchanged; only the shape is.
///
/// `v` must be an index into `vtree`. Returns `None`, leaving `vtree`
/// untouched, if `v` or its left child is a leaf; always succeeds otherwise,
/// and the returned [`RotationInfo`] names the nodes the move touched. The
/// vtree is left fully consistent — a caller may read it, score it and rotate
/// it again without any repair step of its own.
pub fn rotate_right(vtree: &mut Vtree, v: VtreeIdx) -> Option<RotationInfo> {
    let (w, c, v_parent) = match vtree.nodes[v.idx()] {
        VtreeNode::Internal {
            left,
            right,
            parent,
        } => (left, right, parent),
        VtreeNode::Leaf { .. } => return None,
    };
    let (a, b) = match vtree.nodes[w.idx()] {
        VtreeNode::Internal { left, right, .. } => (left, right),
        VtreeNode::Leaf { .. } => return None,
    };

    // v_idx stays as v with children (A, w_idx=w_new).
    vtree.nodes[v.idx()] = VtreeNode::Internal {
        left: a,
        right: w,
        parent: v_parent,
    };
    // w_idx becomes w_new: children = (B, C).
    vtree.nodes[w.idx()] = VtreeNode::Internal {
        left: b,
        right: c,
        parent: Some(v),
    };
    Vtree::set_parent(&mut vtree.nodes, a, v);
    Vtree::set_parent(&mut vtree.nodes, c, w);

    let info = RotationInfo {
        v_idx: v,
        w_idx: w,
        a_idx: a,
        b_idx: b,
        c_idx: c,
    };
    vtree.fixup_topo_after_rotate(&info, RotationKind::Right);
    Some(info)
}
