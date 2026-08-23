//! The node arena a vtree is built into.

use super::{VarId, Vtree, VtreeIdx, VtreeNode};

/// A vtree under construction: nodes in the order the construction created
/// them, with every `parent` left unset.
///
/// Parents are deliberately not written here. A node's parent does not exist
/// yet when the node is pushed — every construction in this crate builds
/// children before the node that joins them — so the links run one way until
/// [`Vtree::from_nodes`](super::Vtree::from_nodes) derives the parents from the
/// child links and settles the final node order.
///
/// The index a push returns names that node for as long as the arena lives, so
/// a construction can hold onto subtree roots and join them later.
#[derive(Default)]
pub(crate) struct VtreeArena {
    nodes: Vec<VtreeNode>,
}

impl VtreeArena {
    /// An empty arena.
    pub(crate) fn new() -> Self {
        VtreeArena { nodes: Vec::new() }
    }

    /// An empty arena with room for `cap` nodes.
    pub(crate) fn with_capacity(cap: usize) -> Self {
        VtreeArena {
            nodes: Vec::with_capacity(cap),
        }
    }

    /// Append a leaf carrying `var`.
    pub(crate) fn leaf(&mut self, var: VarId) -> VtreeIdx {
        let idx = VtreeIdx(self.nodes.len() as u32);
        self.nodes.push(VtreeNode::Leaf { var, parent: None });
        idx
    }

    /// Append a node joining two subtrees.
    pub(crate) fn internal(&mut self, left: VtreeIdx, right: VtreeIdx) -> VtreeIdx {
        let idx = VtreeIdx(self.nodes.len() as u32);
        self.nodes.push(VtreeNode::Internal {
            left,
            right,
            parent: None,
        });
        idx
    }

    /// Append a copy of `sub` and return the index its root now has here.
    ///
    /// Every leaf is renamed through `var`, which is what makes this the way a
    /// subtree built over its own `0..k` variable space comes back under the
    /// variables it stands for. `sub`'s parent links are not read: the arena
    /// does not carry them, and the finished tree derives them from the child
    /// links, so grafting a subtree cannot carry a stale parent in with it.
    pub(crate) fn graft(&mut self, sub: &Vtree, var: impl Fn(VarId) -> VarId) -> VtreeIdx {
        let offset = self.nodes.len() as u32;
        for node in &sub.nodes {
            match *node {
                VtreeNode::Leaf { var: local, .. } => {
                    self.leaf(var(local));
                }
                VtreeNode::Internal { left, right, .. } => {
                    self.internal(VtreeIdx(left.0 + offset), VtreeIdx(right.0 + offset));
                }
            }
        }
        VtreeIdx(sub.root.0 + offset)
    }

    /// How many nodes have been appended. Also the index the next push returns,
    /// which is what a construction that may discard a subtree records first.
    pub(crate) fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Discard everything appended after the first `len` nodes.
    pub(crate) fn truncate(&mut self, len: usize) {
        self.nodes.truncate(len);
    }

    /// The nodes appended so far.
    pub(crate) fn nodes(&self) -> &[VtreeNode] {
        &self.nodes
    }

    /// The node list, for handing to
    /// [`Vtree::from_nodes`](super::Vtree::from_nodes).
    pub(crate) fn into_nodes(self) -> Vec<VtreeNode> {
        self.nodes
    }
}
