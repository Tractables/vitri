//! Shape and traversal invariants of a built vtree.
//!
//! Every shape the builder offers must produce one leaf per variable, a
//! bottom-up order that is a valid topological order, and a sibling
//! relation that agrees with the parent pointers.

use super::*;

/// Each of these documents a panic on an empty variable space, and that is the
/// contract: they are the crate's own building blocks, called where a leaf set
/// has already been established. The library entry that takes a caller's
/// formula reports instead — see `component::build_vtree`.
#[test]
fn every_baseline_constructor_panics_on_an_empty_variable_space() {
    let attempts: [(&str, fn()); 4] = [
        ("balanced", || {
            let _ = Vtree::balanced(0);
        }),
        ("linear", || {
            let _ = Vtree::linear(0);
        }),
        ("reverse-linear", || {
            let _ = Vtree::reverse_linear(0);
        }),
        ("linear_from_order", || {
            let _ = Vtree::linear_from_order(&[]);
        }),
    ];
    for (what, attempt) in attempts {
        assert!(
            std::panic::catch_unwind(attempt).is_err(),
            "{what} documents a panic over no variables",
        );
    }
}

#[test]
fn test_single_variable_vtree() {
    let vtree = Vtree::balanced(1);
    assert_eq!(vtree.num_nodes(), 1);
    assert!(vtree.node(vtree.root()).is_leaf());
    assert_eq!(vtree.leaf_var(vtree.root()), VarId(0));
    assert_eq!(vtree.bottomup().count(), 1);
}

#[test]
fn test_two_variable_vtree() {
    let vtree = Vtree::balanced(2);
    assert_eq!(vtree.num_nodes(), 3);
    assert!(!vtree.node(vtree.root()).is_leaf());
    let (l, r) = vtree.children(vtree.root());
    assert_eq!(vtree.leaf_var(l), VarId(0));
    assert_eq!(vtree.leaf_var(r), VarId(1));
    let bo: Vec<VtreeIdx> = vtree.bottomup().collect();
    assert_eq!(bo, vec![l, r, vtree.root()]);
}

#[test]
fn test_four_variable_vtree() {
    let vtree = Vtree::balanced(4);
    assert_eq!(vtree.num_nodes(), 7);
    assert_eq!(vtree.bottomup().count(), 7);

    assert_eq!(vtree.root(), VtreeIdx(6));

    for var in 0..4u32 {
        let leaf_idx = vtree.leaf_of(VarId(var));
        assert!(vtree.node(leaf_idx).is_leaf());
        assert_eq!(vtree.leaf_var(leaf_idx), VarId(var));
        assert!(leaf_idx.0 < 4, "leaves should come first");
    }
}

#[test]
fn test_bottomup_is_sequential() {
    let vtree = Vtree::balanced(5);
    let bo: Vec<VtreeIdx> = vtree.bottomup().collect();
    let expected: Vec<VtreeIdx> = (0..vtree.num_nodes() as u32).map(VtreeIdx).collect();
    assert_eq!(bo, expected);
}

#[test]
fn test_children_before_parents() {
    for n in 2..=8u32 {
        let vtree = Vtree::balanced(n);
        for i in 0..vtree.num_nodes() {
            if let VtreeNode::Internal { left, right, .. } = vtree.node(VtreeIdx(i as u32)) {
                assert!(left.0 < i as u32, "left child {} >= parent {}", left.0, i);
                assert!(
                    right.0 < i as u32,
                    "right child {} >= parent {}",
                    right.0,
                    i
                );
            }
        }
    }
}

#[test]
fn test_level_order_balanced() {
    for n in [2, 4, 8u32] {
        let vtree = Vtree::balanced(n);
        let num_leaves = n as usize;
        for i in 0..vtree.num_nodes() {
            if vtree.node(VtreeIdx(i as u32)).is_leaf() {
                assert!(
                    i < num_leaves,
                    "leaf at index {} but {} leaves total",
                    i,
                    num_leaves
                );
            } else {
                assert!(
                    i >= num_leaves,
                    "internal at index {} but {} leaves total",
                    i,
                    num_leaves
                );
            }
        }
    }
}

#[test]
fn test_var_to_leaf_mapping() {
    let vtree = Vtree::balanced(5);
    for var in 0..5u32 {
        let leaf = vtree.leaf_of(VarId(var));
        assert_eq!(vtree.leaf_var(leaf), VarId(var));
    }
}

#[test]
fn test_sibling() {
    let vtree = Vtree::balanced(2);
    let (l, r) = vtree.children(vtree.root());
    assert_eq!(vtree.sibling(l), r);
    assert_eq!(vtree.sibling(r), l);
}

#[test]
fn test_linear_structure() {
    let vtree = Vtree::linear(4);
    // 4 leaves + 3 internal = 7 nodes
    assert_eq!(vtree.num_nodes(), 7);

    // Root's left child should be a leaf (x0, first var in forward order)
    let (l, r) = vtree.children(vtree.root());
    assert!(vtree.node(l).is_leaf());
    assert_eq!(vtree.leaf_var(l), VarId(0));
    assert!(!vtree.node(r).is_leaf());

    // All vars mapped correctly
    for var in 0..4u32 {
        let leaf = vtree.leaf_of(VarId(var));
        assert_eq!(vtree.leaf_var(leaf), VarId(var));
    }
}

/// The two chain constructors build the same shape over opposite orders, and
/// which is which is the thing a caller picking an OBDD baseline depends on.
#[test]
fn test_linear_and_reverse_linear_are_mirrors() {
    fn left_spine_vars(vtree: &Vtree) -> Vec<u32> {
        let mut out = Vec::new();
        let mut node = vtree.root();
        while !vtree.node(node).is_leaf() {
            let (l, r) = vtree.children(node);
            out.push(vtree.leaf_var(l).0);
            node = r;
        }
        out.push(vtree.leaf_var(node).0);
        out
    }

    assert_eq!(left_spine_vars(&Vtree::linear(3)), vec![0, 1, 2]);
    assert_eq!(left_spine_vars(&Vtree::reverse_linear(3)), vec![2, 1, 0]);
}

#[test]
fn test_random_structure() {
    let vtree = Vtree::random(5, 42);
    // 5 leaves + 4 internal = 9 nodes
    assert_eq!(vtree.num_nodes(), 9);
    assert_eq!(vtree.bottomup().count(), 9);

    // Root should be last index
    assert_eq!(vtree.root(), VtreeIdx(8));

    // All vars mapped correctly
    for var in 0..5u32 {
        let leaf = vtree.leaf_of(VarId(var));
        assert_eq!(vtree.leaf_var(leaf), VarId(var));
    }
}

#[test]
fn test_random_deterministic_with_seed() {
    let v1 = Vtree::random(6, 123);
    let v2 = Vtree::random(6, 123);
    assert_eq!(v1.num_nodes(), v2.num_nodes());
    assert_eq!(v1.root(), v2.root());
}

#[test]
fn test_single_var_all_shapes() {
    let b = Vtree::balanced(1);
    let l = Vtree::linear(1);
    let r = Vtree::random(1, 0);
    assert_eq!(b.num_nodes(), 1);
    assert_eq!(l.num_nodes(), 1);
    assert_eq!(r.num_nodes(), 1);
}

/// A vtree whose leaves skip variable ids is by design, not a malformed one:
/// per-component construction builds a tree over part of the outer variable
/// space and grafts the parts together, so between the two the leaf count and
/// the variable space are different numbers.
#[test]
fn num_leaves_falls_below_num_vars_when_the_leaves_skip_variable_ids() {
    // Leaves for v0 and v3 only, over a four-variable space.
    let sparse = Vtree::from_nodes(
        vec![
            VtreeNode::Leaf {
                var: VarId(0),
                parent: None,
            },
            VtreeNode::Leaf {
                var: VarId(3),
                parent: None,
            },
            VtreeNode::Internal {
                left: VtreeIdx(0),
                right: VtreeIdx(1),
                parent: None,
            },
        ],
        VtreeIdx(2),
        4,
    );
    assert_eq!(sparse.num_leaves(), 2, "one leaf per variable PRESENT");
    assert_eq!(
        sparse.num_vars(),
        4,
        "the variable space is the one it was built over",
    );
    assert_eq!(sparse.leaf_var(sparse.leaf_of(VarId(3))), VarId(3));

    let dense = Vtree::balanced(4);
    assert_eq!(
        dense.num_leaves(),
        dense.num_vars(),
        "the two agree exactly when no id is skipped",
    );
}
