//! Where two nodes meet.
//!
//! Every clause in a scored formula is placed at the meeting point of its
//! variables' leaves, so this is the primitive the whole score module reduces
//! over.

use super::*;

/// The variables under `node`, sorted. Two trees of the same shape number
/// their nodes differently, so a meeting point is compared by the set it spans
/// rather than by index.
fn vars_below(vtree: &Vtree, node: VtreeIdx) -> Vec<u32> {
    let mut out = leaves_under(vtree, node);
    out.sort_unstable();
    out
}

/// The balanced 4-variable tree `((v1 v2) (v3 v4))`: two leaves meet at the
/// shallowest node holding both of them, which is the node whose two subtrees
/// separate them.
#[test]
fn lca_of_two_leaves_is_the_node_whose_subtrees_separate_them() {
    let vtree = Vtree::balanced(4);
    let leaf = |var: u32| vtree.leaf_of(VarId::new(var).unwrap());
    let parent = |node: VtreeIdx| vtree.node(node).parent().expect("not the root");
    let left_half = parent(leaf(1));
    let right_half = parent(leaf(3));
    assert_ne!(
        left_half, right_half,
        "the fixture puts v1 and v3 in different halves",
    );

    for node in vtree.bottomup() {
        assert_eq!(vtree.lca(node, node), node, "a node meets itself at itself");
    }

    assert_eq!(vtree.lca(leaf(1), leaf(2)), left_half, "siblings");
    assert_eq!(vtree.lca(leaf(3), leaf(4)), right_half, "siblings");

    // An ancestor absorbs its descendant, whichever way round it is asked.
    assert_eq!(vtree.lca(leaf(1), left_half), left_half);
    assert_eq!(vtree.lca(left_half, leaf(1)), left_half);
    assert_eq!(vtree.lca(leaf(1), vtree.root()), vtree.root());

    // Leaves in opposite halves have nowhere lower than the root to meet.
    for a in [leaf(1), leaf(2)] {
        for b in [leaf(3), leaf(4)] {
            assert_eq!(vtree.lca(a, b), vtree.root());
            assert_eq!(vtree.lca(b, a), vtree.root());
        }
    }
}

/// A rotation relinks the tree without renumbering it, so afterwards the node
/// ids no longer run in bottom-up order. The walk that finds a meeting point
/// climbs by maintained topological position rather than by id, which is what
/// keeps it answering the shape rather than the numbering.
#[test]
fn lca_still_answers_correctly_after_a_rotation_has_reordered_the_topo() {
    // `linear(4)` is `(v1 (v2 (v3 v4)))`. Left-rotating the root lifts the
    // inner pair, giving `((v1 v2) (v3 v4))` — the balanced shape, reached by
    // relinking rather than by construction.
    let mut rotated = Vtree::linear(4);
    let root = rotated.root();
    rotate::rotate_left(&mut rotated, root).expect("the root's right child is internal");
    let fresh = Vtree::balanced(4);
    assert!(
        rotated.same_tree(&fresh),
        "the rotation must reach the balanced shape",
    );

    for (a, b) in [(1u32, 2u32), (3, 4), (1, 3), (2, 4), (1, 4), (2, 3)] {
        let in_rotated = rotated.lca(
            rotated.leaf_of(VarId::new(a).unwrap()),
            rotated.leaf_of(VarId::new(b).unwrap()),
        );
        let in_fresh = fresh.lca(
            fresh.leaf_of(VarId::new(a).unwrap()),
            fresh.leaf_of(VarId::new(b).unwrap()),
        );
        assert_eq!(
            vars_below(&rotated, in_rotated),
            vars_below(&fresh, in_fresh),
            "v{a} and v{b} must meet at the same node of the same tree",
        );
    }
}
