//! The rightmost-path reversal construction, and the properties it must have.
//!
//! The construction itself lives here rather than in the library: nothing
//! shipped performs it, and it exists to check a theorem, so it sits with
//! the cases that check it.

use super::*;

/// Walk down the right spine of `vtree` from `start`, returning the variable
/// of each left child (in top-down order) and the variable of the final
/// rightmost leaf. Panics if any node on the path is malformed.
fn right_spine_left_vars(vtree: &Vtree, start: VtreeIdx) -> (Vec<VarId>, VarId) {
    let mut left_vars = Vec::new();
    let mut cur = start;
    loop {
        match vtree.node(cur) {
            VtreeNode::Internal { left, right, .. } => {
                assert!(
                    vtree.node(*left).is_leaf(),
                    "expected leaf-on-left for spine step"
                );
                left_vars.push(vtree.leaf_var(*left));
                cur = *right;
            }
            VtreeNode::Leaf { var, .. } => return (left_vars, *var),
        }
    }
}

#[test]
fn test_reverse_rrp_leaf_is_identity() {
    let vtree = Vtree::balanced(1);
    let t_tilde = vtree.reverse_rightmost_path_recursive();
    assert_eq!(t_tilde.num_nodes(), 1);
    assert!(t_tilde.node(t_tilde.root()).is_leaf());
    assert_eq!(t_tilde.leaf_var(t_tilde.root()), VarId(0));
}

#[test]
fn test_reverse_rrp_two_vars_swaps_children() {
    // T = Internal(leaf(0), leaf(1)). Rightmost path is root → leaf(1), n=2.
    // T̃: leaf(1) on left, T̃_1 = leaf(0) on right.
    let vtree = Vtree::balanced(2);
    let t_tilde = vtree.reverse_rightmost_path_recursive();
    assert_eq!(t_tilde.num_nodes(), 3);
    let (l, r) = t_tilde.children(t_tilde.root());
    assert_eq!(t_tilde.leaf_var(l), VarId(1));
    assert_eq!(t_tilde.leaf_var(r), VarId(0));
}

#[test]
fn test_reverse_rrp_right_linear_chain_reverses_order() {
    // linear_from_order([0,1,2,3]) is right-linear: top-down left-children
    // are 0, 1, 2 and the bottom-right leaf is 3.
    // T̃ should be right-linear with left-children 3, 2, 1 and bottom-right leaf 0.
    let vtree = Vtree::linear_from_order(&[VarId(0), VarId(1), VarId(2), VarId(3)]);
    let t_tilde = vtree.reverse_rightmost_path_recursive();
    assert_eq!(t_tilde.num_nodes(), vtree.num_nodes());
    let (lefts, last) = right_spine_left_vars(&t_tilde, t_tilde.root());
    assert_eq!(lefts, vec![VarId(3), VarId(2), VarId(1)]);
    assert_eq!(last, VarId(0));
}

#[test]
fn test_reverse_rrp_balanced_4_becomes_right_linear_reversed() {
    // balanced(4) has structure Internal((0,1), (2,3)). Rightmost path is
    // root → (2,3) → leaf(3); n=3, with T_1 = (0,1) and T_2 = leaf(2).
    // T̃_1 = transform((0,1)) = (1,0). T̃_2 = leaf(2).
    // T̃ = Internal(leaf(3), Internal(leaf(2), Internal(leaf(1), leaf(0)))).
    let vtree = Vtree::balanced(4);
    let t_tilde = vtree.reverse_rightmost_path_recursive();
    assert_eq!(t_tilde.num_nodes(), vtree.num_nodes());
    let (lefts, last) = right_spine_left_vars(&t_tilde, t_tilde.root());
    assert_eq!(lefts, vec![VarId(3), VarId(2), VarId(1)]);
    assert_eq!(last, VarId(0));
}

#[test]
fn test_reverse_rrp_recursive_side_subtree() {
    // Handcrafted T = Internal(B3, leaf(3)), where B3 = balanced over [0,1,2]
    // (i.e. Internal(leaf(0), Internal(leaf(1), leaf(2)))). The top-level
    // rightmost path has n=2, so the recursion is exercised through T_1 = B3.
    //
    // B3's rightmost path: root → (1,2) → leaf(2); n=3, T_1' = leaf(0), T_2' = leaf(1).
    // T̃_1 = Internal(leaf(2), Internal(leaf(1), leaf(0))).
    // T̃ = Internal(leaf(3), T̃_1) — a right-linear chain with left-children [3,2,1].
    let format = "vtree 7\n\
                  L 0 1\n\
                  L 1 2\n\
                  L 2 3\n\
                  I 3 1 2\n\
                  I 4 0 3\n\
                  L 5 4\n\
                  I 6 4 5\n";
    let vtree = Vtree::from_vtree_text(format).expect("parse failed");
    let t_tilde = vtree.reverse_rightmost_path_recursive();

    assert_eq!(t_tilde.num_nodes(), vtree.num_nodes());
    let (lefts, last) = right_spine_left_vars(&t_tilde, t_tilde.root());
    assert_eq!(lefts, vec![VarId(3), VarId(2), VarId(1)]);
    assert_eq!(last, VarId(0));
}

#[test]
fn test_reverse_rrp_preserves_invariants() {
    let inputs = [
        Vtree::balanced(8),
        Vtree::linear(8),
        Vtree::random(8, 7),
        Vtree::random(11, 31),
    ];
    for vtree in inputs {
        let t_tilde = vtree.reverse_rightmost_path_recursive();

        assert_eq!(t_tilde.num_nodes(), vtree.num_nodes());

        for i in 0..t_tilde.num_nodes() {
            if let VtreeNode::Internal { left, right, .. } = t_tilde.node(VtreeIdx(i as u32)) {
                assert!(left.0 < i as u32, "left {} >= parent {}", left.0, i);
                assert!(right.0 < i as u32, "right {} >= parent {}", right.0, i);
            }
        }

        // Leaves occupy the lower indices, root is the last index.
        let num_leaves = t_tilde.num_leaves() as usize;
        for i in 0..t_tilde.num_nodes() {
            if t_tilde.node(VtreeIdx(i as u32)).is_leaf() {
                assert!(
                    i < num_leaves,
                    "leaf at {} but num_leaves={}",
                    i,
                    num_leaves
                );
            } else {
                assert!(
                    i >= num_leaves,
                    "internal at {} but num_leaves={}",
                    i,
                    num_leaves
                );
            }
        }
        assert_eq!(t_tilde.root().0 as usize, t_tilde.num_nodes() - 1);

        // var_to_leaf round-trips.
        for v in 0..vtree.num_leaves() {
            let leaf = t_tilde.leaf_of(VarId(v));
            assert_eq!(t_tilde.leaf_var(leaf), VarId(v));
        }
    }
}

#[test]
fn test_reverse_rrp_preserves_var_set() {
    let inputs = [
        Vtree::balanced(6),
        Vtree::linear(6),
        Vtree::balanced(7),
        Vtree::random(9, 13),
    ];
    for vtree in inputs {
        let mut before: Vec<VarId> = vtree.leaf_bottomup().map(|(_, v)| v).collect();
        let t_tilde = vtree.reverse_rightmost_path_recursive();
        let mut after: Vec<VarId> = t_tilde.leaf_bottomup().map(|(_, v)| v).collect();
        before.sort();
        after.sort();
        assert_eq!(before, after);
    }
}

impl Vtree {
    /// Build T̃ from T by recursively reversing the rightmost path.
    ///
    /// The construction proves that any TDD respecting T can be transformed
    /// into an SDD of size O(|C|²) respecting T̃. For each subtree:
    /// - the rightmost leaf is hoisted to the top-left of the subtree's image,
    /// - the remaining right-spine is reversed (the variable closest to the
    ///   rightmost leaf in T ends up at the top of the spine in T̃),
    /// - each side (left) subtree of the original spine is itself transformed.
    ///
    /// The leaf set is preserved. Total work is O(n) in the number of nodes.
    #[must_use]
    fn reverse_rightmost_path_recursive(&self) -> Self {
        let mut nodes = VtreeArena::with_capacity(self.num_nodes());
        let new_root = Self::reverse_rrp_rec(self.root(), self, &mut nodes);
        Self::from_nodes(nodes.into_nodes(), new_root, self.num_vars())
    }

    fn reverse_rrp_rec(old_idx: VtreeIdx, old: &Vtree, nodes: &mut VtreeArena) -> VtreeIdx {
        if let VtreeNode::Leaf { var, .. } = old.node(old_idx) {
            return nodes.leaf(*var);
        }

        // Walk the rightmost path from old_idx, collecting left subtrees
        // T_1, T_2, …, T_{n-1} top-down. The walk terminates at the rightmost
        // leaf.
        let mut left_subtrees: Vec<VtreeIdx> = Vec::new();
        let mut cur = old_idx;
        let rightmost_var: VarId = loop {
            match old.node(cur) {
                VtreeNode::Internal { left, right, .. } => {
                    left_subtrees.push(*left);
                    cur = *right;
                }
                VtreeNode::Leaf { var, .. } => break *var,
            }
        };

        let transformed: Vec<VtreeIdx> = left_subtrees
            .iter()
            .map(|&t| Self::reverse_rrp_rec(t, old, nodes))
            .collect();

        // Build the new right-spine. transformed.len() == n-1 ≥ 1.
        // n=2: spine is empty, root.right = T̃_1.
        // n≥3: bottom t'_2 = (T̃_2, T̃_1); for i in 3..=n-1, t'_i = (T̃_i, t'_{i-1}).
        let right_subtree = if transformed.len() == 1 {
            transformed[0]
        } else {
            let mut spine = nodes.internal(transformed[1], transformed[0]);
            for &l in transformed.iter().skip(2) {
                // l is T̃_{i+1}
                spine = nodes.internal(l, spine);
            }
            spine
        };

        let leaf_idx = nodes.leaf(rightmost_var);
        nodes.internal(leaf_idx, right_subtree)
    }
}
