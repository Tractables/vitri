//! Scoring tested through the arithmetic `score` keeps to itself. The metrics
//! a caller can reach, and what they say about a known shape, are tested from
//! `src/tests/score.rs` instead.

mod fused;

use crate::score::{
    vtree_context_width_per_node, vtree_crossing_clauses_per_node,
    vtree_outside_context_width_per_node,
};
use crate::tests::score_fixture::{fixture_formula, fixture_vtree};
use crate::vtree::{VarId, VtreeIdx};

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
