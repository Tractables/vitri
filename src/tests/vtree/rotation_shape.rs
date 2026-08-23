//! What a rotation does to the shape, read through the public entries.

use super::*;

/// The leaves of the whole tree, left to right. A rotation changes which
/// variables are grouped together but never their order along the bottom of the
/// tree — that is the sense in which it re-brackets one decomposition rather
/// than choosing a different one.
fn leaves_left_to_right(vtree: &Vtree) -> Vec<u32> {
    leaves_under(vtree, vtree.root())
}

/// Every rotation that applies, at every node of four different shapes.
#[test]
fn a_rotation_preserves_the_left_to_right_leaf_order() {
    for base in [
        Vtree::balanced(6),
        Vtree::linear(6),
        Vtree::reverse_linear(6),
        Vtree::random(6, 7),
    ] {
        let expected = leaves_left_to_right(&base);
        for i in 0..base.num_nodes() {
            let node = VtreeIdx(i as u32);

            let mut left = base.clone();
            if rotate::rotate_left(&mut left, node).is_some() {
                assert_eq!(
                    leaves_left_to_right(&left),
                    expected,
                    "a left rotation at {node:?} reordered the leaves",
                );
            }

            let mut right = base.clone();
            if rotate::rotate_right(&mut right, node).is_some() {
                assert_eq!(
                    leaves_left_to_right(&right),
                    expected,
                    "a right rotation at {node:?} reordered the leaves",
                );
            }
        }
    }
}

/// A rotation needs an internal node on the side it lifts from, so at a leaf —
/// or at an internal node whose children are both leaves — there is nothing to
/// lift. It reports that by declining, and a declined rotation must not have
/// half-applied itself on the way to finding out.
#[test]
fn a_rotation_that_cannot_apply_leaves_the_vtree_unchanged() {
    let base = Vtree::balanced(4);
    let leaf = base.leaf_of(VarId(0));
    let pair = base
        .node(leaf)
        .parent()
        .expect("a leaf of a 4-variable tree");

    for (what, node) in [
        ("a leaf", leaf),
        ("an internal node whose children are both leaves", pair),
    ] {
        type Rotation = fn(&mut Vtree, VtreeIdx) -> Option<rotate::RotationInfo>;
        let directions: [(&str, Rotation); 2] = [
            ("left", rotate::rotate_left),
            ("right", rotate::rotate_right),
        ];
        for (direction, apply) in directions {
            let mut attempted = base.clone();
            assert!(
                apply(&mut attempted, node).is_none(),
                "a {direction} rotation at {what} has nothing to lift",
            );
            assert!(
                attempted.same_tree(&base),
                "a {direction} rotation that declined at {what} changed the tree",
            );
            assert_eq!(
                attempted.to_vtree_text(),
                base.to_vtree_text(),
                "a {direction} rotation that declined at {what} renumbered the tree",
            );
        }
    }
}
