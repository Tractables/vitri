//! Scoring tested through the arithmetic `score` keeps to itself. The metrics
//! a caller can reach, and what they say about a known shape, are tested from
//! `src/tests/score.rs` instead.

mod fused;

use crate::cnf::{Clause, CnfFormula};
use crate::score::{
    UNIQUE_PRESSURE_THRESHOLD, child_boundary_features, clause_lca_buckets,
    directional_context_excess, extreme_chain_guard, extreme_local_join_guard,
    local_join_match_excess, maximum_matching_size, output_gap_bits, outside_context_tables,
    successor_guard_correction, vtree_context_width_per_node, vtree_crossing_clauses_per_node,
    vtree_depth, vtree_outside_context_width_per_node,
};
use crate::tests::common::lit;
use crate::tests::score_fixture::{fixture_formula, fixture_vtree};
use crate::vtree::{VarId, Vtree, VtreeIdx};

#[test]
fn matching_reassigns_an_earlier_row() {
    let adjacency = vec![vec![1_000_000, 7], vec![1_000_000]];
    assert_eq!(maximum_matching_size(&adjacency), 2);
}

#[test]
fn local_join_density_uses_matching_and_load_at_the_same_node() {
    let vtree = Vtree::balanced(32);
    let clauses = (0..16)
        .map(|var| Clause::new(vec![lit(var, true), lit(var + 16, true)]))
        .collect();
    let formula = CnfFormula {
        num_vars: 32,
        clauses,
    };
    let (clause_at, clauses_at) = clause_lca_buckets(&vtree, &formula);
    let clause_count = clause_at.iter().map(|&load| u64::from(load)).sum();

    assert_eq!(
        local_join_match_excess(&vtree, &formula, &clauses_at, clause_count),
        12.0,
    );
}

#[test]
fn directional_context_penalizes_only_shallow_left_excess() {
    let shallow = Vtree::balanced(32);
    let (left, right) = shallow.children(shallow.root());
    let mut context = vec![0; shallow.num_nodes()];
    context[left.idx()] = 10;
    context[right.idx()] = 1;
    assert_eq!(
        directional_context_excess(&shallow, &context, vtree_depth(&shallow)),
        6.0,
    );

    context.swap(left.idx(), right.idx());
    assert_eq!(
        directional_context_excess(&shallow, &context, vtree_depth(&shallow)),
        0.0,
    );

    let deep = Vtree::linear(32);
    let (left, right) = deep.children(deep.root());
    let mut context = vec![0; deep.num_nodes()];
    context[left.idx()] = 10;
    context[right.idx()] = 1;
    assert_eq!(
        directional_context_excess(&deep, &context, vtree_depth(&deep)),
        0.0,
    );
}

#[test]
fn output_gap_ignores_the_first_twelve_bits() {
    assert_eq!(output_gap_bits(10.0, 22.0), 0.0);
    assert_eq!(output_gap_bits(10.0, 23.0), 1.0);
}

#[test]
fn extreme_chain_guard_activates_near_the_linear_depth() {
    assert_eq!(extreme_chain_guard(32, 5), 0.0);
    assert_eq!(extreme_chain_guard(32, 31), 3.0);
}

#[test]
fn extreme_local_join_guard_starts_after_moderate_excess() {
    assert_eq!(extreme_local_join_guard(12.0), 0.0);
    assert_eq!(extreme_local_join_guard(19.5), 7.5);
}

#[test]
fn outside_context_overlap_counts_a_variable_shared_by_both_children() {
    let vtree = Vtree::balanced(4);
    let formula = CnfFormula {
        num_vars: 4,
        clauses: vec![
            Clause::new(vec![lit(0, true), lit(2, true)]),
            Clause::new(vec![lit(1, true), lit(2, true)]),
        ],
    };
    let left_leaf = vtree.leaf_of(VarId(0));
    let right_leaf = vtree.leaf_of(VarId(1));
    let parent = vtree
        .node(left_leaf)
        .parent()
        .expect("a balanced four-leaf tree has a parent here");
    assert_eq!(vtree.node(right_leaf).parent(), Some(parent));

    let outside = outside_context_tables(&vtree, &formula);

    assert_eq!(outside.widths[left_leaf.idx()], 1);
    assert_eq!(outside.widths[right_leaf.idx()], 1);
    assert_eq!(outside.sibling_overlap[parent.idx()], 1);
}

#[test]
fn child_boundary_summary_uses_the_two_largest_overlaps() {
    let vtree = Vtree::balanced(4);
    let leaf = |var| vtree.leaf_of(VarId(var));
    let left_parent = vtree.node(leaf(0)).parent().expect("not the root");
    let right_parent = vtree.node(leaf(2)).parent().expect("not the root");
    let root = vtree.root();
    let mut tight = vec![0; vtree.num_nodes()];
    let mut outside = vec![0; vtree.num_nodes()];
    let mut overlap = vec![0; vtree.num_nodes()];
    tight[leaf(0).idx()] = 5;
    tight[leaf(1).idx()] = 4;
    tight[left_parent.idx()] = 12;
    tight[right_parent.idx()] = 11;
    outside[leaf(0).idx()] = 7;
    outside[leaf(1).idx()] = 6;
    outside[leaf(2).idx()] = 2;
    outside[leaf(3).idx()] = 2;
    outside[left_parent.idx()] = 20;
    outside[right_parent.idx()] = 15;
    overlap[left_parent.idx()] = 3;
    overlap[right_parent.idx()] = 1;
    overlap[root.idx()] = 10;

    let features = child_boundary_features(&vtree, &tight, &outside, &overlap);

    assert_eq!(features.outside_overlap_top2_mean, 6.5);
    assert_eq!(features.outside_symmetric_difference_max, 15);
    assert_eq!(features.tight_unique_sum[left_parent.idx()], 6);
    assert_eq!(features.tight_unique_sum[root.idx()], 13);
}

#[test]
fn successor_guards_apply_the_fitted_caps() {
    assert_eq!(successor_guard_correction(0.0, 0.0, 0), 3.0);
    assert_eq!(
        successor_guard_correction(UNIQUE_PRESSURE_THRESHOLD, 37.0, 63),
        3.84,
    );
    assert_eq!(
        successor_guard_correction(UNIQUE_PRESSURE_THRESHOLD + 0.25, 37.0, 63),
        3.9775,
    );
    assert_eq!(
        successor_guard_correction(UNIQUE_PRESSURE_THRESHOLD + 1.0, 37.0, 63),
        3.9775,
    );
}

/// The three per-node tables `cost` is reduced from, on [`fixture_vtree`]
/// over [`fixture_formula`], node by node. `A` is the parent of the leaves
/// v0 and v1, `B` of v2 and v3, `R` the root.
///
/// * inside: every variable's widest clause meets at `R`, so each is counted
///   at the one node between its leaf and `R` — 2 at `A` and `B`, 0 at every
///   leaf and at `R`.
/// * outside: v0's mates are v1 and v2, v1's are v0 and v3, v2's are v3 and
///   v0, v3's are v2 and v1 — 2 at every leaf; `A` sees v2 and v3, `B` sees
///   v0 and v1; nothing is outside `R`.
/// * crossing: v0 sits in c1, c3, c5 and v1 in c1, c4, c5 — 3 at each leaf;
///   v2 in c2, c3 and v3 in c2, c4 — 2 at each; c3 and c4 cross `A` and `B`;
///   no clause crosses `R`.
#[test]
fn fixture_separator_tables_match_hand_computation() {
    let formula = fixture_formula();
    let vtree = fixture_vtree();
    let leaf = |v: u32| vtree.leaf_of(VarId(v));
    let parent = |t: VtreeIdx| vtree.node(t).parent().expect("not the root");
    let a = parent(leaf(0));
    let b = parent(leaf(2));
    let r = vtree.root();
    // (node, inside, outside, crossing)
    let expected = [
        (leaf(0), 0, 2, 3),
        (leaf(1), 0, 2, 3),
        (leaf(2), 0, 2, 2),
        (leaf(3), 0, 2, 2),
        (a, 2, 2, 2),
        (b, 2, 2, 2),
        (r, 0, 0, 0),
    ];
    let inside = vtree_context_width_per_node(&vtree, &formula, None);
    let outside = vtree_outside_context_width_per_node(&vtree, &formula);
    let crossing = vtree_crossing_clauses_per_node(&vtree, &formula);
    assert_eq!(vtree.num_nodes(), expected.len());
    for (t, i, o, c) in expected {
        assert_eq!(inside[t.idx()], i, "inside at {t:?}");
        assert_eq!(outside[t.idx()], o, "outside at {t:?}");
        assert_eq!(crossing[t.idx()], c, "crossing at {t:?}");
    }
}
