//! Binarization: turn one bag's already-built child subtrees plus its local
//! variable leaves into a single binary vtree subtree.
//!
//! The last step of each bag's conversion in `algo`, picked by
//! [`super::ItemOrdering`] — a balanced split, a left-deep chain, a greedy
//! clause-cut bisection, a multilevel hypergraph bisection with clauses as
//! hyperedges, an interior/boundary split, or the edge-aligned combiner that
//! cuts children by shared local variables and lifts each shared variable to
//! the lowest ancestor of exactly the branches using it.
//!
//! Every combiner consumes each item exactly once: an item dropped on a
//! degenerate split is a variable missing from the finished vtree, which is
//! what the balanced and midpoint fallbacks exist to prevent. Nodes are
//! appended to the caller's arena with `parent` left `None`, and the return
//! value names the new subtree's root.

use crate::cnf::CnfFormula;
use crate::vtree::{VtreeArena, VtreeIdx};

/// Which item covers each variable, `u32::MAX` for one no item covers.
///
/// Both bisecting combiners route a clause to the items its variables sit in,
/// and this is the table they read. A variable listed by two items would land
/// on the later one; the callers pass disjoint sets, so that does not arise.
fn var_to_item(item_vars: &[Vec<u32>], num_vars: u32) -> Vec<u32> {
    let mut var_to_item: Vec<u32> = vec![u32::MAX; num_vars as usize];
    for (i, vars) in item_vars.iter().enumerate() {
        for &v in vars {
            if (v as usize) < var_to_item.len() {
                var_to_item[v as usize] = i as u32;
            }
        }
    }
    var_to_item
}

/// Greedy-gain bisection of `members`, which index a symmetric weight table
/// `weight` reads.
///
/// Everything starts on the left; the member whose move most reduces the cut
/// weight moves right, repeatedly, until the right side holds half of them. The
/// scan keeps the lowest position on a tie, so the split is a function of the
/// weights alone and two runs over the same table agree.
///
/// Returns one flag per position of `members`: `true` means right.
fn greedy_gain_bisect(members: &[usize], weight: impl Fn(usize, usize) -> u32) -> Vec<bool> {
    let n = members.len();
    let target_right = n / 2;
    // gain[p] over positions p in `members`; start all-left.
    let mut gain: Vec<i64> = (0..n)
        .map(|p| {
            let i = members[p];
            let w: i64 = members
                .iter()
                .filter(|&&q| q != i)
                .map(|&q| weight(i, q) as i64)
                .sum();
            -w
        })
        .collect();
    let mut in_right = vec![false; n];
    let mut right_count = 0;
    while right_count < target_right {
        let mut best = usize::MAX;
        let mut best_gain = i64::MIN;
        for p in 0..n {
            if !in_right[p] && gain[p] > best_gain {
                best_gain = gain[p];
                best = p;
            }
        }
        in_right[best] = true;
        right_count += 1;
        let bi = members[best];
        for p in 0..n {
            if !in_right[p] {
                gain[p] += 2 * weight(members[p], bi) as i64;
            }
        }
    }
    in_right
}

/// One side of a split: the items on it, with their variable sets.
struct Side {
    items: Vec<VtreeIdx>,
    vars: Vec<Vec<u32>>,
}

/// Deal the items and their variable sets onto the two sides `in_right` names,
/// each side keeping the caller's order.
fn split_sides(items: &[VtreeIdx], item_vars: &[Vec<u32>], in_right: &[bool]) -> (Side, Side) {
    let mut left = Side {
        items: Vec::new(),
        vars: Vec::new(),
    };
    let mut right = Side {
        items: Vec::new(),
        vars: Vec::new(),
    };
    for (i, &to_right) in in_right.iter().enumerate() {
        let side = if to_right { &mut right } else { &mut left };
        side.items.push(items[i]);
        side.vars.push(item_vars[i].clone());
    }
    (left, right)
}

/// Combine items using multilevel hypergraph bisection: clauses touching ≥2
/// items become hyperedges for the multilevel partitioner. Falls back to
/// [`combine_into_balanced`] for 3 or fewer items, a length-mismatched
/// `item_vars`, or no hyperedges. `effort_scale` is the bisector's
/// construction-effort multiplier (see [`crate::budget::vtree_effort_scale`]).
pub(super) fn combine_hypergraph_bisect(
    items: &[VtreeIdx],
    item_vars: &[Vec<u32>],
    formula: &CnfFormula,
    effort_scale: f64,
    nodes: &mut VtreeArena,
) -> VtreeIdx {
    if items.len() <= 3 || item_vars.len() != items.len() {
        return combine_into_balanced(items, nodes);
    }

    let var_to_item = var_to_item(item_vars, formula.num_vars);

    let n = items.len();
    let mut hyperedges: Vec<Vec<u32>> = Vec::new();
    for clause in &formula.clauses {
        let mut pins: Vec<u32> = Vec::new();
        for lit in &clause.literals {
            let v = lit.var.0 as usize;
            if v < var_to_item.len() && var_to_item[v] != u32::MAX {
                let item_idx = var_to_item[v];
                if !pins.contains(&item_idx) {
                    pins.push(item_idx);
                }
            }
        }
        if pins.len() >= 2 {
            pins.sort_unstable();
            hyperedges.push(pins);
        }
    }

    if hyperedges.is_empty() {
        return combine_into_balanced(items, nodes);
    }

    let part = super::super::multilevel_hg_bisect::multilevel_hg_bisect(
        n,
        &hyperedges,
        None,
        super::super::BisectDials {
            imbalance: super::super::multilevel_hg_bisect::IMBALANCE_BALANCED,
            base_seed: 0,
            effort_scale,
        },
    );

    let in_right: Vec<bool> = part.iter().map(|&p| p != 0).collect();
    let (left, right) = split_sides(items, item_vars, &in_right);

    // Fallback if partition is degenerate
    if left.items.is_empty() || right.items.is_empty() {
        return combine_into_balanced(items, nodes);
    }

    let l = combine_hypergraph_bisect(&left.items, &left.vars, formula, effort_scale, nodes);
    let r = combine_hypergraph_bisect(&right.items, &right.vars, formula, effort_scale, nodes);

    nodes.internal(l, r)
}

/// Edge-aligned faithful combine of one TD node's children subtrees and its
/// bag-local variable leaves — see [`super::Binarization::Edge`] for the
/// algorithm.
///
/// `child_items[i]` is the already-built vtree subtree for the i-th TD child and
/// `child_vars[i]` is the full set of variables in that subtree. `leaf_items[j]`
/// is the vtree leaf for local variable `leaf_vars[j]` (a variable assigned to
/// THIS TD node). `primal_adj[v]` is the primal-graph neighbour list of variable
/// `v` (empty for variables with no recorded neighbours), used to route interior
/// leaves toward the child subtree holding most of their clause partners.
///
/// Deterministic: all ordering is by the caller's (deterministic) item order and
/// by ascending index on ties.
pub(super) fn combine_edge_aligned(
    child_items: &[VtreeIdx],
    child_vars: &[Vec<u32>],
    leaf_items: &[VtreeIdx],
    leaf_vars: &[u32],
    primal_adj: &[Vec<u32>],
    nodes: &mut VtreeArena,
) -> VtreeIdx {
    use std::collections::HashSet;
    debug_assert_eq!(child_items.len(), child_vars.len());
    debug_assert_eq!(leaf_items.len(), leaf_vars.len());

    let k = child_items.len();
    let m = leaf_items.len();

    // Per-child membership sets (built once, reused across the recursion).
    let child_sets: Vec<HashSet<u32>> = child_vars
        .iter()
        .map(|vs| vs.iter().copied().collect())
        .collect();

    // For each local leaf: the children (indices) that contain it, and a
    // per-child clause-partner affinity score (|primal_neighbours ∩ child_set|).
    // `using[j]` drives separator lifting; `aff[j][i]` drives interior routing.
    let mut using: Vec<Vec<usize>> = vec![Vec::new(); m];
    let mut aff: Vec<Vec<u32>> = vec![vec![0u32; k]; m];
    for j in 0..m {
        let v = leaf_vars[j];
        for (i, set) in child_sets.iter().enumerate() {
            if set.contains(&v) {
                using[j].push(i);
            }
        }
        let nbrs = primal_adj
            .get(v as usize)
            .map(|s| s.as_slice())
            .unwrap_or(&[]);
        for (i, set) in child_sets.iter().enumerate() {
            aff[j][i] = nbrs.iter().filter(|&&u| set.contains(&u)).count() as u32;
        }
    }

    // Pairwise "shared local variable" weight between children — the objective
    // the edge-aligned bisection minimises across each cut.
    let mut shared = vec![0u32; k * k];
    for u in &using {
        for a in 0..u.len() {
            for b in (a + 1)..u.len() {
                shared[u[a] * k + u[b]] += 1;
                shared[u[b] * k + u[a]] += 1;
            }
        }
    }

    fn combine_leaf_set(
        sel: &[usize],
        leaf_items: &[VtreeIdx],
        nodes: &mut VtreeArena,
    ) -> VtreeIdx {
        let items: Vec<VtreeIdx> = sel.iter().map(|&j| leaf_items[j]).collect();
        combine_into_balanced(&items, nodes)
    }

    // Bisect a child subset into (left, right) minimising shared-variable cut
    // weight. Falls back to a balanced index split when the subset has no
    // shared structure.
    fn bisect_children(subset: &[usize], shared: &[u32], k: usize) -> (Vec<usize>, Vec<usize>) {
        let in_right = greedy_gain_bisect(subset, |a, b| shared[a * k + b]);
        let mut left = Vec::new();
        let mut right = Vec::new();
        for (p, &to_right) in in_right.iter().enumerate() {
            if to_right {
                right.push(subset[p]);
            } else {
                left.push(subset[p]);
            }
        }
        // Degenerate guard: never return an empty side (would drop a subtree).
        if left.is_empty() || right.is_empty() {
            let mid = subset.len() / 2;
            return (subset[..mid].to_vec(), subset[mid..].to_vec());
        }
        (left, right)
    }

    /// Everything the recursion below reads but never varies: the subtrees it
    /// draws from, which variables each child covers, and the two weight
    /// tables that route leaves and cut children. Only the two subsets and the
    /// node arena change from call to call.
    struct Ctx<'a> {
        child_items: &'a [VtreeIdx],
        leaf_items: &'a [VtreeIdx],
        child_sets: &'a [std::collections::HashSet<u32>],
        leaf_vars: &'a [u32],
        aff: &'a [Vec<u32>],
        shared: &'a [u32],
        k: usize,
    }

    // Recursive edge-aligned build over a subset of children + the local leaves
    // routed to that subset.
    fn build(
        child_subset: &[usize],
        leaf_subset: &[usize],
        ctx: &Ctx<'_>,
        nodes: &mut VtreeArena,
    ) -> VtreeIdx {
        if child_subset.is_empty() {
            // Only leaves remain (all interior to this node).
            return combine_leaf_set(leaf_subset, ctx.leaf_items, nodes);
        }
        if child_subset.len() == 1 {
            let c = child_subset[0];
            if leaf_subset.is_empty() {
                return ctx.child_items[c];
            }
            // Leaves routed here use only this child (or are interior) — attach
            // them adjacent to the child subtree.
            let leaves = combine_leaf_set(leaf_subset, ctx.leaf_items, nodes);
            let idx = nodes.internal(leaves, ctx.child_items[c]);
            return idx;
        }

        let (left_c, right_c) = bisect_children(child_subset, ctx.shared, ctx.k);

        let mut left_leaves = Vec::new();
        let mut right_leaves = Vec::new();
        let mut boundary = Vec::new();
        for &j in leaf_subset {
            let v = ctx.leaf_vars[j];
            let uses_left = left_c.iter().any(|&i| ctx.child_sets[i].contains(&v));
            let uses_right = right_c.iter().any(|&i| ctx.child_sets[i].contains(&v));
            if uses_left && uses_right {
                // Straddles the cut — lift to this (lowest shared) ancestor.
                boundary.push(j);
            } else if uses_left {
                left_leaves.push(j);
            } else if uses_right {
                right_leaves.push(j);
            } else {
                // Interior leaf: route toward the side with more clause partners.
                let ls: u32 = left_c.iter().map(|&i| ctx.aff[j][i]).sum();
                let rs: u32 = right_c.iter().map(|&i| ctx.aff[j][i]).sum();
                if rs > ls {
                    right_leaves.push(j);
                } else {
                    left_leaves.push(j);
                }
            }
        }

        let l = build(&left_c, &left_leaves, ctx, nodes);
        let r = build(&right_c, &right_leaves, ctx, nodes);
        let mut node = nodes.internal(l, r);
        // Place boundary (straddling) leaves as a chain of ancestors above the cut.
        for &j in &boundary {
            let idx = nodes.internal(ctx.leaf_items[j], node);
            node = idx;
        }
        node
    }

    let all_children: Vec<usize> = (0..k).collect();
    let all_leaves: Vec<usize> = (0..m).collect();
    let ctx = Ctx {
        child_items,
        leaf_items,
        child_sets: &child_sets,
        leaf_vars,
        aff: &aff,
        shared: &shared,
        k,
    };
    build(&all_children, &all_leaves, &ctx, nodes)
}

/// Combine `items` into a balanced subtree by halving the list recursively.
///
/// # Panics
///
/// Panics if `items` is empty — there is no subtree over nothing, and the
/// halving would otherwise recurse on two empty halves forever.
pub(super) fn combine_into_balanced(items: &[VtreeIdx], nodes: &mut VtreeArena) -> VtreeIdx {
    assert!(!items.is_empty());
    if items.len() == 1 {
        return items[0];
    }
    let mid = items.len() / 2;
    let l = combine_into_balanced(&items[..mid], nodes);
    let r = combine_into_balanced(&items[mid..], nodes);
    nodes.internal(l, r)
}
