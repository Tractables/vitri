use super::*;

#[test]
fn left_rotate_then_unrotate_is_identity() {
    let mut vtree = Vtree::balanced(4);
    let original = vtree.clone();
    let root = vtree.root;
    let info = rotate_left(&mut vtree, root).unwrap();
    assert_invariants(&vtree);
    unrotate_left(&mut vtree, &info);
    assert_equal(&vtree, &original);
}

#[test]
fn left_rotate_at_leaf_or_leaf_child_returns_none() {
    let mut vtree = Vtree::balanced(4);
    assert!(rotate_left(&mut vtree, VtreeIdx(0)).is_none());
    // Internal node whose right child is a leaf:
    // balanced(4): 6=(4,5), 4=(0,1), 5=(2,3). At node 4, right=1 is a leaf.
    assert!(rotate_left(&mut vtree, VtreeIdx(4)).is_none());
}

#[test]
fn right_rotate_after_left_recovers_original() {
    let mut vtree = Vtree::linear(4);
    let original = vtree.clone();
    let root = vtree.root;
    let _info = rotate_left(&mut vtree, root).unwrap();
    assert_invariants(&vtree);
    let _info_right = rotate_right(&mut vtree, root).unwrap();
    assert_invariants(&vtree);
    assert_equal(&vtree, &original);
}

#[test]
fn right_rotate_then_unrotate_is_identity() {
    let mut vtree = Vtree::linear(4);
    let root = vtree.root;
    let _ = rotate_left(&mut vtree, root).unwrap();
    let snap = vtree.clone();
    let info = rotate_right(&mut vtree, root).unwrap();
    assert_invariants(&vtree);
    unrotate_right(&mut vtree, &info);
    assert_equal(&vtree, &snap);
}

#[test]
fn right_rotate_works_immediately_on_balanced() {
    let mut vtree = Vtree::balanced(4);
    let root = vtree.root;
    let info = rotate_right(&mut vtree, root);
    assert!(
        info.is_some(),
        "right rotation should be unconditionally applicable"
    );
    assert_invariants(&vtree);
}
