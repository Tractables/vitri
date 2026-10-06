//! Scoring tested through the arithmetic `score` keeps to itself. The metrics
//! a caller can reach, and what they say about a known shape, are tested from
//! `src/tests/score.rs` instead.

mod fused;

use crate::cnf::{Clause, CnfFormula};
use crate::score::{
    Layout, UNIQUE_PRESSURE_THRESHOLD, child_boundary_features, clause_lca_counts,
    crossing_clauses, directional_context_excess, extreme_chain_guard, extreme_local_join_guard,
    local_join_features, maximum_matching_size, output_gap_bits, outside_context_tables,
    successor_guard_correction, vtree_context_width_per_node, vtree_depth,
};
use crate::tests::common::lit;
use crate::tests::score_fixture::{fixture_formula, fixture_vtree};
use crate::vtree::{VarId, Vtree, VtreeIdx};

/// Clause-LCA counts and the formula-clause indices contributing to each node.
fn clause_lca_buckets(vtree: &Vtree, formula: &CnfFormula) -> (Vec<u32>, Vec<Vec<usize>>) {
    let layout = Layout::new(vtree, formula);
    (layout.loads(vtree), layout.members(vtree))
}

/// The shallow local-join excess alone, at the weights the fused pass uses.
fn local_join_match_excess(
    vtree: &Vtree,
    formula: &CnfFormula,
    clauses_at: &[Vec<usize>],
    clause_count: u64,
) -> f64 {
    local_join_features(
        vtree,
        formula,
        &Layout::new(vtree, formula),
        clauses_at,
        clause_count,
        true,
        &vec![0; vtree.num_nodes()],
    )
    .0
}

/// The outside-context width per node, which is the half of
/// [`outside_context_tables`] these tests check.
fn vtree_outside_context_width_per_node(vtree: &Vtree, formula: &CnfFormula) -> Vec<u32> {
    outside_context_tables(vtree, formula, &Layout::new(vtree, formula)).widths
}

#[test]
fn matching_reassigns_an_earlier_row() {
    let adjacency = vec![vec![1_000_000, 7], vec![1_000_000]];
    assert_eq!(maximum_matching_size(&adjacency), 2);
}

#[test]
fn local_join_density_uses_matching_and_load_at_the_same_node() {
    let vtree = Vtree::balanced(32);
    let clauses = (1..=16)
        .map(|var| Clause::new(vec![lit(var, true), lit(var + 16, true)]))
        .collect();
    let formula = CnfFormula::from_parts(32, clauses);
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
    let formula = CnfFormula::from_parts(
        4,
        vec![
            Clause::new(vec![lit(1, true), lit(3, true)]),
            Clause::new(vec![lit(2, true), lit(3, true)]),
        ],
    );
    let left_leaf = vtree.leaf_of(VarId::from_dimacs(1));
    let right_leaf = vtree.leaf_of(VarId::from_dimacs(2));
    let parent = vtree
        .node(left_leaf)
        .parent()
        .expect("a balanced four-leaf tree has a parent here");
    assert_eq!(vtree.node(right_leaf).parent(), Some(parent));

    let outside = outside_context_tables(&vtree, &formula, &Layout::new(&vtree, &formula));

    assert_eq!(outside.widths[left_leaf.idx()], 1);
    assert_eq!(outside.widths[right_leaf.idx()], 1);
    assert_eq!(outside.sibling_overlap[parent.idx()], 1);
}

#[test]
fn child_boundary_summary_uses_the_two_largest_overlaps() {
    let vtree = Vtree::balanced(4);
    let leaf = |var| vtree.leaf_of(VarId::new(var).unwrap());
    let left_parent = vtree.node(leaf(1)).parent().expect("not the root");
    let right_parent = vtree.node(leaf(3)).parent().expect("not the root");
    let root = vtree.root();
    let mut tight = vec![0; vtree.num_nodes()];
    let mut outside = vec![0; vtree.num_nodes()];
    let mut overlap = vec![0; vtree.num_nodes()];
    tight[leaf(1).idx()] = 5;
    tight[leaf(2).idx()] = 4;
    tight[left_parent.idx()] = 12;
    tight[right_parent.idx()] = 11;
    outside[leaf(1).idx()] = 7;
    outside[leaf(2).idx()] = 6;
    outside[leaf(3).idx()] = 2;
    outside[leaf(4).idx()] = 2;
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
/// v1 and v2, `B` of v3 and v4, `R` the root.
///
/// * inside: every variable's widest clause meets at `R`, so each is counted
///   at the one node between its leaf and `R` — 2 at `A` and `B`, 0 at every
///   leaf and at `R`.
/// * outside: v1's mates are v2 and v3, v2's are v1 and v4, v3's are v4 and
///   v1, v4's are v3 and v2 — 2 at every leaf; `A` sees v3 and v4, `B` sees
///   v1 and v2; nothing is outside `R`.
/// * crossing: v1 sits in c1, c3, c5 and v2 in c1, c4, c5 — 3 at each leaf;
///   v3 in c2, c3 and v4 in c2, c4 — 2 at each; c3 and c4 cross `A` and `B`;
///   no clause crosses `R`.
#[test]
fn fixture_separator_tables_match_hand_computation() {
    let formula = fixture_formula();
    let vtree = fixture_vtree();
    let leaf = |v: u32| vtree.leaf_of(VarId::new(v).unwrap());
    let parent = |t: VtreeIdx| vtree.node(t).parent().expect("not the root");
    let a = parent(leaf(1));
    let b = parent(leaf(3));
    let r = vtree.root();
    // (node, inside, outside, crossing)
    let expected = [
        (leaf(1), 0, 2, 3),
        (leaf(2), 0, 2, 3),
        (leaf(3), 0, 2, 2),
        (leaf(4), 0, 2, 2),
        (a, 2, 2, 2),
        (b, 2, 2, 2),
        (r, 0, 0, 0),
    ];
    let inside = vtree_context_width_per_node(&vtree, &formula, None);
    let outside = vtree_outside_context_width_per_node(&vtree, &formula);
    let crossing = crossing_clauses(&vtree, &formula, &Layout::new(&vtree, &formula));
    assert_eq!(vtree.num_nodes(), expected.len());
    for (t, i, o, c) in expected {
        assert_eq!(inside[t.idx()], i, "inside at {t:?}");
        assert_eq!(outside[t.idx()], o, "outside at {t:?}");
        assert_eq!(crossing[t.idx()], c, "crossing at {t:?}");
    }
}

/// The per-node tables of one (vtree, formula) pair, recomputed from their
/// definitions over explicit variable sets: the clause-LCA counts, inside and
/// outside widths, the sibling overlap and the crossing-clause counts, in that
/// order.
fn tables_by_definition(vtree: &Vtree, formula: &CnfFormula) -> [Vec<u32>; 5] {
    use std::collections::BTreeSet;
    let nn = vtree.num_nodes();
    let mut below: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); nn];
    for t in vtree.bottomup() {
        below[t.idx()] = if vtree.node(t).is_leaf() {
            BTreeSet::from([vtree.leaf_var(t).idx()])
        } else {
            let (left, right) = vtree.children(t);
            &below[left.idx()] | &below[right.idx()]
        };
    }
    let strictly_above = |a: VtreeIdx, t: VtreeIdx| {
        let mut cur = vtree.node(t).parent();
        while let Some(node) = cur {
            if node == a {
                return true;
            }
            cur = vtree.node(node).parent();
        }
        false
    };
    let vars = |c: &Clause| -> BTreeSet<usize> { c.literals.iter().map(|l| l.var.idx()).collect() };
    // The deepest node holding every variable of the clause: the smallest such set.
    let lca = |c: &Clause| {
        let vs = vars(c);
        vtree
            .bottomup()
            .filter(|t| vs.is_subset(&below[t.idx()]))
            .min_by_key(|t| below[t.idx()].len())
            .expect("the root holds every variable")
    };
    let mut clause_at = vec![0u32; nn];
    let mut ctx_in = vec![0u32; nn];
    let mut ctx_out = vec![0u32; nn];
    let mut overlap = vec![0u32; nn];
    let mut cross = vec![0u32; nn];
    let lcas: Vec<VtreeIdx> = formula.clauses().iter().map(lca).collect();
    let mut outside: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); nn];
    for t in vtree.bottomup() {
        let inside = &below[t.idx()];
        for (c, &at) in formula.clauses().iter().zip(&lcas) {
            let vs = vars(c);
            if at == t {
                clause_at[t.idx()] += 1;
            }
            let touches = !vs.is_disjoint(inside);
            if c.literals.len() >= 2 && touches && strictly_above(at, t) {
                cross[t.idx()] += 1;
            }
            if touches {
                outside[t.idx()].extend(vs.difference(inside));
            }
        }
        ctx_out[t.idx()] = outside[t.idx()].len() as u32;
        if !vtree.node(t).is_leaf() {
            ctx_in[t.idx()] = inside
                .iter()
                .filter(|&&v| {
                    formula.clauses().iter().zip(&lcas).any(|(c, &at)| {
                        c.literals.len() >= 2 && vars(c).contains(&v) && strictly_above(at, t)
                    })
                })
                .count() as u32;
        }
    }
    for (t, left, right) in vtree.internal_bottomup() {
        overlap[t.idx()] = outside[left.idx()]
            .intersection(&outside[right.idx()])
            .count() as u32;
    }
    [clause_at, ctx_in, ctx_out, overlap, cross]
}

#[test]
fn per_node_tables_match_their_definitions_on_random_and_rotated_trees() {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = |bound: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % bound
    };
    for round in 0..60 {
        let num_vars = 2 + next(30) as u32;
        let clauses = (0..1 + next(40))
            .map(|_| {
                let mut chosen: Vec<u32> = Vec::new();
                for _ in 0..1 + next(5) {
                    let v = 1 + next(u64::from(num_vars)) as u32;
                    if !chosen.contains(&v) {
                        chosen.push(v);
                    }
                }
                Clause::new(chosen.into_iter().map(|v| lit(v, next(2) == 0)).collect())
            })
            .collect();
        let formula = CnfFormula::from_parts(num_vars, clauses);
        let mut vtree = Vtree::random(num_vars, round);
        // Rotations leave the node array out of topological order, which every
        // table has to read through.
        for _ in 0..next(20) {
            let at = VtreeIdx(next(vtree.num_nodes() as u64) as u32);
            if next(2) == 0 {
                crate::vtree::rotate::rotate_left(&mut vtree, at);
            } else {
                crate::vtree::rotate::rotate_right(&mut vtree, at);
            }
        }
        let [clause_at, ctx_in, ctx_out, overlap, cross] = tables_by_definition(&vtree, &formula);
        let layout = Layout::new(&vtree, &formula);
        let outside = outside_context_tables(&vtree, &formula, &layout);
        assert_eq!(
            clause_lca_counts(&vtree, &formula),
            clause_at,
            "loads, round {round}"
        );
        assert_eq!(
            vtree_context_width_per_node(&vtree, &formula, None),
            ctx_in,
            "inside widths, round {round}"
        );
        assert_eq!(outside.widths, ctx_out, "outside widths, round {round}");
        assert_eq!(outside.sibling_overlap, overlap, "overlap, round {round}");
        assert_eq!(
            crossing_clauses(&vtree, &formula, &layout),
            cross,
            "crossing, round {round}"
        );
        let members = layout.members(&vtree);
        let per_node: Vec<u32> = members.iter().map(|m| m.len() as u32).collect();
        assert_eq!(per_node, clause_at, "members, round {round}");
    }
}
