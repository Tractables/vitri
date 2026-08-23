//! Variable tree (vtree): the structural backbone of a compiled diagram.
//!
//! This module owns the vtree *structure* (topology, ordering, LCA,
//! (de)serialization); the construction heuristics that decide which structure
//! to build live in [`crate::decompose`].
//!
//! ## Node layout
//!
//! A vtree is built with its nodes in bottom-up level order: all leaves first
//! (`0..num_leaves`), then internal nodes (`num_leaves..n`), so
//! `child.idx() < parent.idx()` holds for every edge — which is what makes a
//! bottom-up pass one forward loop and an LCA an O(depth) walk. A rotation
//! relinks nodes without moving them, so on a rotated tree the array order is no
//! longer topological and [`Vtree::bottomup`] is: every traversal, and the
//! serializer, read that order rather than `0..num_nodes`.

/// The variable a vtree leaf carries, and the literal built over it. Defined
/// in [`crate::cnf`], which is where a formula's variables come from, and
/// re-exported here so `crate::vtree::VarId` keeps resolving.
pub use crate::cnf::{Literal, VarId};

/// Index into the vtree node array.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd)]
pub struct VtreeIdx(pub u32);

impl VtreeIdx {
    /// The index as a `usize`.
    #[inline(always)]
    pub fn idx(self) -> usize {
        self.0 as usize
    }
}

/// Which rotation direction a `fixup_topo_after_rotate` call corresponds to —
/// selects which grandchild subtree may violate children-before-parents after
/// the rotation.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub(crate) enum RotationKind {
    /// A left rotation.
    Left,
    /// A right rotation.
    Right,
}

/// A node in the vtree (variable tree).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VtreeNode {
    /// A leaf holding a single variable.
    Leaf {
        /// The variable at this leaf.
        var: VarId,
        /// Parent node index, or `None` at the root.
        parent: Option<VtreeIdx>,
    },
    /// An internal node with two children.
    Internal {
        /// Left child index.
        left: VtreeIdx,
        /// Right child index.
        right: VtreeIdx,
        /// Parent node index, or `None` at the root.
        parent: Option<VtreeIdx>,
    },
}

impl VtreeNode {
    /// This node's parent index, or `None` if it is the root.
    pub fn parent(&self) -> Option<VtreeIdx> {
        match self {
            VtreeNode::Leaf { parent, .. } => *parent,
            VtreeNode::Internal { parent, .. } => *parent,
        }
    }

    /// Whether this node is a leaf.
    pub fn is_leaf(&self) -> bool {
        matches!(self, VtreeNode::Leaf { .. })
    }
}

/// The precondition every constructor below shares: a vtree has a root, so it
/// has at least one leaf. `#[track_caller]` puts the panic at the constructor
/// the caller actually named rather than here.
#[track_caller]
fn require_nonempty(num_vars: u32) {
    assert!(num_vars > 0, "a vtree needs at least one variable");
}

/// A variable tree (vtree).
///
/// Nodes are laid out leaves-first then internals at construction — see the
/// module docs for the `child.idx() < parent.idx()` ordering that gives, and for
/// why a rotated tree is read through [`Vtree::bottomup`] instead.
///
/// # Representation
///
/// The node list, the root and the variable-to-leaf inversion are the tree
/// itself; the traversal orders beside them are derived from it, and every
/// mutating operation re-derives them, which is why none of the six is
/// reachable from outside. Read the tree through [`Vtree::node`],
/// [`Vtree::root`], [`Vtree::children`], [`Vtree::leaf_of`],
/// [`Vtree::leaf_var`], [`Vtree::bottomup`] and [`Vtree::lca`]. Three things
/// hold for as long as the `Vtree` lives:
///
/// - **A node's index is its identity.** The node list is never reordered or
///   resized after construction, so a [`VtreeIdx`] a caller holds keeps
///   pointing at the same node — across rotations included. (What a rotation
///   *does* change is the shape: which nodes those indices are linked to.)
/// - **Links run both ways and stay consistent.** Every node names its parent
///   (`None` at the root alone), every internal node names its two children,
///   and a parent's children contain the child that named it. The root is the
///   one node without a parent, and every node reaches it by following parents.
/// - **Leaves invert.** [`leaf_var(leaf_of(v)) == v`](Vtree::leaf_of) for every
///   variable the vtree covers. The inversion is indexed by variable, not by
///   leaf, so the variable space it spans ([`Vtree::num_vars`]) exceeds
///   [`Vtree::num_leaves`] when a caller builds a vtree whose leaves skip
///   variable ids.
#[derive(Clone, Debug)]
pub struct Vtree {
    /// All vtree nodes: at construction, leaves first (`0..num_leaves`) then
    /// internal nodes in bottom-up level order (so `child.idx() < parent.idx()`
    /// for every edge). A rotation relinks nodes without reordering this list,
    /// which is why `topo` and not the list order is the topological one.
    nodes: Vec<VtreeNode>,
    /// Index of the root node — the one node whose `parent` is `None`.
    root: VtreeIdx,
    /// Maps a [`VarId`] to the index of the leaf carrying it. Entries for ids
    /// that carry no leaf are meaningless, which is why [`Vtree::leaf_of`] is
    /// documented for covered variables only.
    var_to_leaf: Vec<VtreeIdx>,
    /// Actual number of leaf nodes. When None, equals `var_to_leaf.len()`.
    /// Set explicitly when `VarIds` are sparse (not all entries in `var_to_leaf`
    /// correspond to actual leaves).
    leaf_count: Option<u32>,
    /// Bottom-up topological order over `nodes`. Decoupled from node identity
    /// (a node's index in `nodes` never changes after construction; its
    /// position in `topo` may change after a rotation). Maintained so that
    /// every parent appears after both its children.
    ///
    /// **Root-last property**: for every node `t`, `topo_pos[t]` is the
    /// maximum of `topo_pos[d]` over `d ∈ {t} ∪ descendants(t)`. Each
    /// subtree's root sits at the latest topo position among its members.
    /// This is not currently asserted after every operation, but it is
    /// preserved inductively by any full postorder rebuild.
    ///
    /// **Subtree contiguity is NOT guaranteed**: after a sequence of rotations
    /// and fixups, a subtree's members may occupy a non-contiguous set of
    /// positions in `topo`. Consumers must walk parent pointers / child links
    /// to enumerate a subtree, not slice `topo` by position range.
    topo: Vec<VtreeIdx>,
    /// Inverse of `topo`: `topo_pos[idx.idx()]` is the position of node `idx`
    /// in `topo`. Used by `lca()` and as a topological-rank comparator.
    /// Inherits the root-last property from `topo`.
    topo_pos: Vec<u32>,
    /// `topo` filtered to internal nodes only. Recomputed alongside `topo`.
    internal_topo: Vec<VtreeIdx>,
    /// `topo` filtered to leaf nodes only. Recomputed alongside `topo`.
    leaf_topo: Vec<VtreeIdx>,
}

impl Vtree {
    /// Number of leaf nodes. Since nodes are stored leaves-first, indices
    /// `0..num_leaves()` are leaves and `num_leaves()..n` are internal nodes.
    #[inline]
    pub fn num_leaves(&self) -> u32 {
        self.leaf_count.unwrap_or(self.var_to_leaf.len() as u32)
    }

    /// Every node once, children before parents — the order a bottom-up pass
    /// over the tree must visit them in, and the reason a consumer can compute
    /// a value per node in a single loop with no recursion or explicit stack.
    ///
    /// Reverse it (the iterator is double-ended) for a top-down pass. This is
    /// the maintained topological order, not `0..num_nodes`: after a rotation
    /// the two disagree, and only this one is still topological.
    pub fn bottomup(&self) -> impl DoubleEndedIterator<Item = VtreeIdx> + ExactSizeIterator + '_ {
        self.topo.iter().copied()
    }

    /// Bottom-up traversal of leaf nodes only, yielding (`node_idx`, `var_id`).
    /// Walks the cached `leaf_topo` slice; preserves topological order of leaves.
    pub fn leaf_bottomup(
        &self,
    ) -> impl DoubleEndedIterator<Item = (VtreeIdx, VarId)> + ExactSizeIterator + '_ {
        self.leaf_topo.iter().map(move |&t| {
            let var = match self.node(t) {
                VtreeNode::Leaf { var, .. } => *var,
                _ => unreachable!("leaf_topo entry is not a leaf"),
            };
            (t, var)
        })
    }

    /// Bottom-up traversal of internal nodes only, yielding (`node_idx`, `left_child`, `right_child`).
    /// Walks the cached `internal_topo` slice; remains valid after rotations.
    pub(crate) fn internal_bottomup(
        &self,
    ) -> impl DoubleEndedIterator<Item = (VtreeIdx, VtreeIdx, VtreeIdx)> + ExactSizeIterator + '_
    {
        self.internal_topo.iter().map(move |&t| match self.node(t) {
            VtreeNode::Internal { left, right, .. } => (t, *left, *right),
            _ => unreachable!("internal_topo entry is not internal"),
        })
    }

    /// Build a balanced binary vtree over `num_vars` variables (`0..num_vars`).
    /// Variables are split in half recursively in natural order (0, 1, …, n-1).
    ///
    /// # Panics
    ///
    /// Panics if `num_vars` is zero.
    pub fn balanced(num_vars: u32) -> Self {
        require_nonempty(num_vars);

        let vars: Vec<VarId> = (0..num_vars).map(VarId).collect();
        let mut nodes = VtreeArena::new();

        let root = Self::build_balanced_recursive(&vars, &mut nodes);

        Self::from_nodes(nodes.into_nodes(), root, num_vars)
    }

    /// Recursively build a balanced vtree over `vars`, appending nodes into
    /// `nodes` and returning the index of the constructed subtree's root.
    pub(crate) fn build_balanced_recursive(vars: &[VarId], nodes: &mut VtreeArena) -> VtreeIdx {
        if vars.len() == 1 {
            return nodes.leaf(vars[0]);
        }

        let mid = vars.len() / 2;
        let left = Self::build_balanced_recursive(&vars[..mid], nodes);
        let right = Self::build_balanced_recursive(&vars[mid..], nodes);

        nodes.internal(left, right)
    }

    /// Build a linear vtree over `num_vars` variables (`0..num_vars`) in
    /// forward order. Structure: each internal node has a single leaf and a
    /// subtree containing the remaining variables, so variable 0 sits at the
    /// leftmost leaf and variable `num_vars - 1` deepest on the right — the
    /// OBDD variable order.
    ///
    /// # Panics
    ///
    /// Panics if `num_vars` is zero.
    pub fn linear(num_vars: u32) -> Self {
        require_nonempty(num_vars);
        let vars: Vec<VarId> = (0..num_vars).map(VarId).collect();
        Self::linear_from_order(&vars)
    }

    /// [`Vtree::linear`]'s mirror: the same chain shape over the reversed
    /// order, so variable `num_vars - 1` sits at the leftmost leaf.
    ///
    /// # Panics
    ///
    /// Panics if `num_vars` is zero.
    pub fn reverse_linear(num_vars: u32) -> Self {
        require_nonempty(num_vars);
        let vars: Vec<VarId> = (0..num_vars).rev().map(VarId).collect();
        Self::linear_from_order(&vars)
    }

    /// Build a linear vtree from a given variable order.
    ///
    /// # Panics
    ///
    /// Panics if `vars` is empty.
    pub fn linear_from_order(vars: &[VarId]) -> Self {
        require_nonempty(vars.len() as u32);
        let num_vars = vars.iter().map(|v| v.0).max().unwrap() + 1;
        let mut nodes = VtreeArena::new();

        let root = Self::build_linear_iterative(vars, &mut nodes);

        Self::from_nodes(nodes.into_nodes(), root, num_vars)
    }

    fn build_linear_iterative(vars: &[VarId], nodes: &mut VtreeArena) -> VtreeIdx {
        // Build a right-linear chain: each internal node has a single-variable
        // left child and the accumulated subtree as its right child.
        let mut right = nodes.leaf(*vars.last().unwrap());

        for &var in vars[..vars.len() - 1].iter().rev() {
            let left = nodes.leaf(var);
            right = nodes.internal(left, right);
        }

        right
    }

    /// Build a random vtree over `num_vars` variables (`0..num_vars`).
    /// Repeatedly picks two random trees from a forest and joins them, until one tree remains.
    pub fn random(num_vars: u32, seed: u64) -> Self {
        use rand::SeedableRng;
        use rand::rngs::SmallRng;
        let mut rng = SmallRng::seed_from_u64(seed);
        Self::random_with_rng(num_vars, &mut rng)
    }

    /// Build a random vtree using an externally provided RNG.
    ///
    /// # Panics
    ///
    /// Panics if `num_vars` is zero.
    pub(crate) fn random_with_rng(num_vars: u32, rng: &mut impl rand::Rng) -> Self {
        use rand::RngExt;
        require_nonempty(num_vars);

        let mut nodes = VtreeArena::new();

        use rand::seq::SliceRandom;
        let mut var_ids: Vec<u32> = (0..num_vars).collect();
        var_ids.shuffle(rng);
        let mut forest: Vec<VtreeIdx> = var_ids.iter().map(|&v| nodes.leaf(VarId(v))).collect();

        while forest.len() > 1 {
            let i = rng.random_range(0..forest.len());
            let left = forest.swap_remove(i);
            let j = rng.random_range(0..forest.len());
            let right = forest.swap_remove(j);

            forest.push(nodes.internal(left, right));
        }

        let root = forest[0];

        Self::from_nodes(nodes.into_nodes(), root, num_vars)
    }

    /// Re-index all nodes in bottom-up level order: leaves first, then
    /// internals, left-to-right within each level, root last.
    fn reindex_bottomup(
        root: VtreeIdx,
        old_nodes: Vec<VtreeNode>,
        mut var_to_leaf: Vec<VtreeIdx>,
    ) -> Self {
        use std::collections::VecDeque;

        let n = old_nodes.len();
        let mut old_to_new = vec![VtreeIdx(0); n];

        let mut levels: Vec<Vec<VtreeIdx>> = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back(root);
        while !queue.is_empty() {
            let level_size = queue.len();
            let mut level = Vec::with_capacity(level_size);
            for _ in 0..level_size {
                let idx = queue.pop_front().unwrap();
                level.push(idx);
                if let VtreeNode::Internal { left, right, .. } = &old_nodes[idx.idx()] {
                    queue.push_back(*left);
                    queue.push_back(*right);
                }
            }
            levels.push(level);
        }

        // Two passes: all leaves bottom-up, then all internals bottom-up —
        // leaves land at 0..num_leaves, internals after, preserving
        // child.idx() < parent.idx() for every edge.
        let mut new_nodes = Vec::with_capacity(n);

        // Pass 1: all leaves, bottom-up
        for level in levels.iter().rev() {
            for &old_idx in level {
                if let VtreeNode::Leaf { var, .. } = &old_nodes[old_idx.idx()] {
                    let new_idx = VtreeIdx(new_nodes.len() as u32);
                    old_to_new[old_idx.idx()] = new_idx;
                    new_nodes.push(VtreeNode::Leaf {
                        var: *var,
                        parent: None,
                    });
                    var_to_leaf[var.idx()] = new_idx;
                }
            }
        }
        let actual_leaf_count = new_nodes.len() as u32;

        // Pass 2: all internals, bottom-up (children already have lower indices)
        for level in levels.iter().rev() {
            for &old_idx in level {
                if let VtreeNode::Internal { left, right, .. } = &old_nodes[old_idx.idx()] {
                    let new_left = old_to_new[left.idx()];
                    let new_right = old_to_new[right.idx()];
                    let new_idx = VtreeIdx(new_nodes.len() as u32);
                    old_to_new[old_idx.idx()] = new_idx;
                    new_nodes.push(VtreeNode::Internal {
                        left: new_left,
                        right: new_right,
                        parent: None,
                    });
                    Self::set_parent(&mut new_nodes, new_left, new_idx);
                    Self::set_parent(&mut new_nodes, new_right, new_idx);
                }
            }
        }

        let new_root = old_to_new[root.idx()];
        let leaf_count = if actual_leaf_count != var_to_leaf.len() as u32 {
            Some(actual_leaf_count)
        } else {
            None
        };
        let n = new_nodes.len();
        // After reindex_bottomup, the node array is laid out so that idx ==
        // bottom-up topological position. Initialize topo / topo_pos to identity,
        // and split into leaf/internal partitions in bottom-up order.
        let topo: Vec<VtreeIdx> = (0..n as u32).map(VtreeIdx).collect();
        let topo_pos: Vec<u32> = (0..n as u32).collect();
        let mut leaf_topo = Vec::with_capacity(actual_leaf_count as usize);
        let mut internal_topo = Vec::with_capacity(n - actual_leaf_count as usize);
        for &t in &topo {
            if new_nodes[t.idx()].is_leaf() {
                leaf_topo.push(t);
            } else {
                internal_topo.push(t);
            }
        }
        Vtree {
            nodes: new_nodes,
            root: new_root,
            var_to_leaf,
            leaf_count,
            topo,
            topo_pos,
            internal_topo,
            leaf_topo,
        }
    }

    /// Localized topo update after a single rotation: `O(subtree)` instead of
    /// a full `O(n_nodes)` postorder rebuild.
    pub(crate) fn fixup_topo_after_rotate(
        &mut self,
        info: &rotate::RotationInfo,
        kind: RotationKind,
    ) {
        self.fixup_topo_pointers_only_after_rotate(info, kind);
        self.refresh_filtered_topo();
    }

    /// Pointer-only variant of `fixup_topo_after_rotate`: updates `topo` /
    /// `topo_pos` but skips the `O(n_nodes)` `refresh_filtered_topo` walk. Use
    /// when the caller won't read `internal_topo` / `leaf_topo` until a later
    /// explicit `refresh_filtered_topo()`.
    pub(crate) fn fixup_topo_pointers_only_after_rotate(
        &mut self,
        info: &rotate::RotationInfo,
        kind: RotationKind,
    ) {
        let w_pos = self.topo_pos[info.w_idx.idx()] as usize;
        // The single new children-before-parents constraint introduced by a
        // rotation:
        //   Left rotation  v=(A,w),w=(B,C) → v=(w,C),w=(A,B): need A < w.
        //   Right rotation v=(w,C),w=(A,B) → v=(A,w),w=(B,C): need C < w.
        let misplaced_root = match kind {
            RotationKind::Left => info.a_idx,
            RotationKind::Right => info.c_idx,
        };
        // By root-last, topo_pos[misplaced_root] is the maximum topo position
        // over the entire misplaced subtree.
        let m_end = self.topo_pos[misplaced_root.idx()] as usize;

        if m_end < w_pos {
            return;
        }

        debug_assert!(
            m_end > w_pos,
            "misplaced_root and w cannot share a topo position"
        );

        // Slice [w_pos ..= m_end] currently starts with w (at w_pos) and ends
        // with the misplaced subtree's root (at m_end). After rotate_left(1),
        // w sits at m_end (one past every element of the misplaced subtree
        // that lay in the slice), and elements in (w_pos..=m_end] shift one
        // position to the left. This is a single contiguous memmove.
        //
        // Subtree contiguity is NOT required: even if non-misplaced elements
        // lie in (w_pos..m_end), the slice rotation preserves children-before-
        // parents for every edge in the post-rotation tree. The full proof is
        // in the `vtree::rotate` module doc.
        self.topo[w_pos..=m_end].rotate_left(1);

        for (offset, &node) in self.topo[w_pos..=m_end].iter().enumerate() {
            self.topo_pos[node.idx()] = (w_pos + offset) as u32;
        }
    }

    /// Refilter `internal_topo` and `leaf_topo` from the current `topo`.
    /// `O(n_nodes)`; called by `fixup_topo_after_rotate` to keep the filtered
    /// views consistent without rebuilding the full topo array.
    pub(crate) fn refresh_filtered_topo(&mut self) {
        self.internal_topo.clear();
        self.leaf_topo.clear();
        for &t in &self.topo {
            if self.nodes[t.idx()].is_leaf() {
                self.leaf_topo.push(t);
            } else {
                self.internal_topo.push(t);
            }
        }
    }

    /// Topological position of `idx` (0 = first in bottom-up order, root = last).
    #[inline]
    pub(crate) fn topo_pos(&self, idx: VtreeIdx) -> u32 {
        self.topo_pos[idx.idx()]
    }

    /// Takes `nodes` by mutable slice rather than `&mut self` so it can be
    /// called during vtree construction before the owning `Vtree` is assembled.
    pub(crate) fn set_parent(nodes: &mut [VtreeNode], child: VtreeIdx, parent: VtreeIdx) {
        match &mut nodes[child.idx()] {
            VtreeNode::Leaf { parent: p, .. } => *p = Some(parent),
            VtreeNode::Internal { parent: p, .. } => *p = Some(parent),
        }
    }

    /// Construct a Vtree from a raw node list and root index, reindexing bottom-up.
    ///
    /// The derived tables come from the child links alone: parent links are
    /// wired here, and `var_to_leaf` is filled for every leaf the root reaches,
    /// so a construction hands over child links and nothing else. `num_vars`
    /// sizes `var_to_leaf` — a variable space wider than the leaf set is what
    /// makes [`num_leaves`](Vtree::num_leaves) differ from its length.
    pub(crate) fn from_nodes(nodes: Vec<VtreeNode>, root: VtreeIdx, num_vars: u32) -> Self {
        let var_to_leaf = vec![VtreeIdx(0); num_vars as usize];
        Self::reindex_bottomup(root, nodes, var_to_leaf)
    }

    /// The node at `idx`.
    #[inline]
    pub fn node(&self, idx: VtreeIdx) -> &VtreeNode {
        &self.nodes[idx.idx()]
    }

    /// The root: the one node with no parent, and where a top-down walk starts.
    #[inline]
    pub fn root(&self) -> VtreeIdx {
        self.root
    }

    /// The leaf carrying `var` — the inverse of [`Vtree::leaf_var`].
    ///
    /// For a variable the vtree covers. A vtree whose leaves skip variable ids
    /// answers for the ids in between too, and that answer is meaningless: it
    /// is a leaf, but not one carrying `var`.
    #[inline]
    pub fn leaf_of(&self, var: VarId) -> VtreeIdx {
        self.var_to_leaf[var.idx()]
    }

    /// The variable space this vtree spans: `max(VarId) + 1`, which a formula
    /// scored against it has to fit inside.
    ///
    /// Equal to [`Vtree::num_leaves`] unless the leaves skip variable ids.
    #[inline]
    pub fn num_vars(&self) -> u32 {
        self.var_to_leaf.len() as u32
    }

    /// Total number of vtree nodes (leaves + internals).
    pub fn num_nodes(&self) -> usize {
        self.nodes.len()
    }

    /// The left and right child of an internal node, in that order — the step
    /// a top-down walk of the tree takes.
    ///
    /// # Panics
    ///
    /// Panics if `idx` refers to a leaf node.
    pub fn children(&self, idx: VtreeIdx) -> (VtreeIdx, VtreeIdx) {
        match &self.nodes[idx.idx()] {
            VtreeNode::Internal { left, right, .. } => (*left, *right),
            VtreeNode::Leaf { .. } => panic!("children() called on leaf node"),
        }
    }

    /// Whether `other` is the same tree: the same shape, carrying the same
    /// variable at every corresponding leaf — corresponding meaning reached by
    /// the same sequence of left/right steps from the root.
    ///
    /// This is what "the same vtree" means. Node indices, positions in
    /// [`Vtree::bottomup`] and the ids in [`Vtree::to_vtree_text`] are
    /// numbering, not identity, and two constructions that arrive at one tree
    /// are free to number it differently.
    ///
    /// Reach for this in particular after
    /// [`rotate_left`](crate::vtree::rotate::rotate_left) or
    /// [`rotate_right`](crate::vtree::rotate::rotate_right): a rotation
    /// maintains a valid bottom-up order rather than rebuilding the one a fresh
    /// construction would produce, so a rotated tree and the same shape built
    /// from scratch serialize differently while being equal here.
    pub fn same_tree(&self, other: &Vtree) -> bool {
        let mut pairs = vec![(self.root(), other.root())];
        while let Some((a, b)) = pairs.pop() {
            match (self.node(a), other.node(b)) {
                (VtreeNode::Leaf { var: va, .. }, VtreeNode::Leaf { var: vb, .. }) => {
                    if va != vb {
                        return false;
                    }
                }
                (VtreeNode::Internal { .. }, VtreeNode::Internal { .. }) => {
                    let (a_left, a_right) = self.children(a);
                    let (b_left, b_right) = other.children(b);
                    pairs.push((a_left, b_left));
                    pairs.push((a_right, b_right));
                }
                _ => return false,
            }
        }
        true
    }

    /// The variable this leaf stands for — the inverse of [`Vtree::leaf_of`].
    ///
    /// # Panics
    ///
    /// Panics if `idx` refers to an internal node.
    pub fn leaf_var(&self, idx: VtreeIdx) -> VarId {
        match &self.nodes[idx.idx()] {
            VtreeNode::Leaf { var, .. } => *var,
            VtreeNode::Internal { .. } => panic!("leaf_var() called on internal node"),
        }
    }

    /// The other child of `idx`'s parent: the subtree a decomposition at that
    /// parent puts on the far side of `idx`.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is the root node (it has no parent, hence no sibling).
    pub fn sibling(&self, idx: VtreeIdx) -> VtreeIdx {
        let parent = self.node(idx).parent().expect("sibling() called on root");
        let (left, right) = self.children(parent);
        if left == idx { right } else { left }
    }

    /// Lowest common ancestor of two vtree nodes. O(depth), no allocation.
    ///
    /// # Panics
    ///
    /// Panics if `a` and `b` do not belong to the same tree (their paths never converge).
    pub fn lca(&self, mut a: VtreeIdx, mut b: VtreeIdx) -> VtreeIdx {
        while a != b {
            if self.topo_pos[a.idx()] < self.topo_pos[b.idx()] {
                a = self.node(a).parent().expect("nodes should share a root");
            } else {
                b = self.node(b).parent().expect("nodes should share a root");
            }
        }
        a
    }
}

/// The arena every construction appends its nodes to.
mod arena;
pub(crate) use arena::VtreeArena;

/// The `.vtree` text codec, in both directions.
mod text;

pub mod rotate; // In-place vtree left/right rotations + topo fixup

#[cfg(test)]
mod tests;
