//! When two vtrees are the same tree.
//!
//! Node ids are numbering, not identity: a tree reached two ways can be
//! numbered two ways, so equality is the shape and the variable at each
//! corresponding leaf.

use super::*;

/// Tree identity is the tree, not its numbering. A rotation maintains a valid
/// bottom-up order instead of rebuilding the one a fresh construction produces,
/// so the same tree reached two ways can serialize two ways — which is the
/// whole reason `same_tree` exists rather than comparing the text.
#[test]
fn a_rotated_tree_equals_the_same_shape_built_fresh() {
    // `linear(3)` is `v = (A, w)` with `w = (B, C)`, over variables 0, 1, 2.
    let mut rotated = Vtree::linear(3);
    let (a, w) = rotated.children(rotated.root());
    assert_eq!(rotated.leaf_var(a), VarId(0), "left child is variable 0");
    assert!(!rotated.node(w).is_leaf(), "right child is the inner pair");

    // Left-rotating the root gives `v = (w, C)` with `w = (A, B)` — the tree
    // `((A, B), C)`.
    let root = rotated.root();
    rotate::rotate_left(&mut rotated, root).expect("the root's right child is internal");

    // The same shape, constructed rather than rotated into.
    let fresh = Vtree::from_vtree_text("vtree 5\nL 0 1\nL 1 2\nI 2 0 1\nL 3 3\nI 4 2 3\n")
        .expect("a well-formed vtree text");

    assert!(rotated.same_tree(&fresh), "one tree, reached two ways");
    assert!(fresh.same_tree(&rotated), "and the comparison is symmetric");
    assert_ne!(
        rotated.to_vtree_text(),
        fresh.to_vtree_text(),
        "the counterexample: equal trees, unequal serializations",
    );

    // A different tree over the same variables is not equal.
    assert!(!rotated.same_tree(&Vtree::linear(3)));
}

/// Two constructions that arrive at one tree compare equal, and every way of
/// being a different tree — a different bracketing, a different variable at a
/// leaf, a different number of leaves — compares unequal.
#[test]
fn two_separately_built_vtrees_of_the_same_shape_compare_equal_and_different_shapes_do_not() {
    // `((v0 v1) (v2 v3))` read from a file, against the same tree built by the
    // balanced constructor — two constructions sharing no code.
    let balanced = Vtree::balanced(4);
    let from_file =
        Vtree::from_vtree_text("vtree 7\nL 0 1\nL 1 2\nI 2 0 1\nL 3 3\nL 4 4\nI 5 3 4\nI 6 2 5\n")
            .expect("a well-formed vtree text");
    assert!(from_file.same_tree(&balanced), "one tree, built two ways");
    assert!(balanced.same_tree(&balanced), "and a tree equals itself");

    // Two constructors reaching one chain, over the order one of them names.
    let forward = Vtree::linear_from_order(&[VarId(0), VarId(1), VarId(2)]);
    assert!(
        forward.same_tree(&Vtree::linear(3)),
        "one tree, built two ways"
    );
    assert!(
        !forward.same_tree(&Vtree::reverse_linear(3)),
        "the same shape carrying different variables is a different tree",
    );

    assert!(
        !balanced.same_tree(&Vtree::linear(4)),
        "a different bracketing of the same variables is a different tree",
    );
    assert!(
        !balanced.same_tree(&Vtree::balanced(8)),
        "a wider tree is a different tree",
    );
}
