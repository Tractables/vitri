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
//! Returns `None` only when the rotation is structurally impossible (`v` or
//! `w` is a leaf). There is no longer a topological-order applicability check
//! because the topo update is decoupled from node identity.
//!
//! # Topo properties preserved by rotations
//!
//! `Vtree::topo` is required to satisfy *children-before-parents* (every
//! parent appears after both its children). The rotation primitives + topo
//! fixup additionally preserve a stronger property, which
//! `Vtree::fixup_topo_after_rotate` relies on:
//!
//! ## Root-last property
//!
//! For every vtree node `t`,
//!
//! ```text
//! topo_pos[t] == max( topo_pos[d]  for d in {t} ∪ descendants(t) )
//! ```
//!
//! Each subtree's root sits at the latest topo position among its members.
//! This is *not* asserted after every operation, but it holds inductively
//! through any legal sequence of rebuilds + rotations:
//!
//! - **Base**: a full postorder rebuild of `topo` visits each subtree root
//!   after all of its descendants. Property holds trivially.
//! - **Pointer-only rotation**: pointer surgery only edits the parent/child
//!   links of `v` and `w`. The descendant *sets* of subtrees `A`, `B`, `C`
//!   (and of any node not in `v`'s subtree) are unchanged, so their
//!   max-position witness is unchanged. The property may be temporarily
//!   broken for `v` and `w` themselves until the topo fixup runs.
//! - **Topo fixup**: relocates `w` past the misplaced subtree's segment as
//!   a single block (`topo[w_pos..=m_end].rotate_left(1)`). It does not
//!   reorder anything outside that slice, and within the slice it shifts
//!   every non-`w` element left by exactly one position. So the
//!   max-position witness of every subtree (including `w`'s new subtree
//!   and the misplaced subtree) updates consistently with the shift, and
//!   the property is restored for `w` and `v`.
//!
//! The early-return case in `fixup_topo_after_rotate` relies on this property —
//! `topo_pos[misplaced_root]` being the maximum over the entire misplaced
//! subtree — to decide in O(1) whether any reordering is needed.
//!
//! ## Subtree contiguity is *not* preserved
//!
//! After a sequence of rotations + fixups, a subtree's members may occupy
//! a non-contiguous range of positions in `topo` (an element from a sibling
//! subtree can sit "between" two members of the same subtree). This is
//! intentional: enforcing contiguity would require shifting unrelated
//! elements during fixups. No consumer of `topo` / `topo_pos` /
//! `internal_topo` / `leaf_topo` in this codebase indexes a subtree by
//! position range — they all walk parent pointers / child links or do rank
//! comparisons via `topo_pos`. Children-before-parents and root-last are
//! sufficient for every consumer.
//!
//! Correctness of the slice rotation does **not** depend on contiguity: the
//! shift preserves children-before-parents for every edge in the new tree
//! (each edge either has both endpoints inside the slice — both shift — or
//! both endpoints outside — neither shifts — or one endpoint inside and the
//! shift never inverts the integer ordering between them). See the
//! `fixup_topo_after_rotate` body for the per-case argument.

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

/// Left-rotate, pointer surgery only — does NOT update `Vtree::topo`. Returns
/// `None` if `v` or its right child is a leaf.
///
/// After this call, `topo`/`topo_pos`/`internal_topo`/`leaf_topo` are stale
/// relative to the new shape. Callers that need them must repair the order
/// themselves, through `Vtree::fixup_topo_after_rotate`. For a caller probing
/// many rotations and keeping one: a probe that measures and reverts reads no
/// topo, so repairing it on every probe is wasted work.
pub(crate) fn rotate_left_pointers(vtree: &mut Vtree, v: VtreeIdx) -> Option<RotationInfo> {
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

    Some(RotationInfo {
        v_idx: v,
        w_idx: w,
        a_idx: a,
        b_idx: b,
        c_idx: c,
    })
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
    let info = rotate_left_pointers(vtree, v)?;
    vtree.fixup_topo_after_rotate(&info, RotationKind::Left);
    Some(info)
}

/// Right-rotate, pointer surgery only — does NOT update `Vtree::topo`. Mirror
/// of `rotate_left_pointers`.
pub(crate) fn rotate_right_pointers(vtree: &mut Vtree, v: VtreeIdx) -> Option<RotationInfo> {
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

    Some(RotationInfo {
        v_idx: v,
        w_idx: w,
        a_idx: a,
        b_idx: b,
        c_idx: c,
    })
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
    let info = rotate_right_pointers(vtree, v)?;
    vtree.fixup_topo_after_rotate(&info, RotationKind::Right);
    Some(info)
}
