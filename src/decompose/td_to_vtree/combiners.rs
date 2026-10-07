//! Binarization: turn one bag's already-built child subtrees plus its local
//! variable leaves into a single binary vtree subtree.
//!
//! The last step of each bag's conversion in `algo`, picked by
//! [`super::Binarization`]. The two combiners here are the ones that read the
//! CNF: a multilevel hypergraph bisection with clauses as hyperedges, and the
//! edge-aligned combiner that cuts children by shared local variables and lifts
//! each shared variable to the lowest ancestor of exactly the branches using
//! it. The third binarization is [`VtreeArena::combine_balanced`], which reads
//! no clause.
//!
//! Every combiner consumes each item exactly once: an item dropped on a
//! degenerate split is a variable missing from the finished vtree, which is
//! what the balanced and midpoint fallbacks exist to prevent. Nodes are
//! appended to the caller's arena with `parent` left `None`, and the return
//! value names the new subtree's root.

use crate::cnf::CnfFormula;
use crate::decompose::BisectionMemo;
use crate::vtree::{VtreeArena, VtreeIdx};
use std::collections::HashSet;

/// Where the hypergraph combiner reads its clauses from, kept for a whole
/// reading: the formula, the clauses each variable occurs in, and a table of
/// which item covers each variable that one call fills and clears again.
///
/// A bag's items cover only the variables below it, so the clauses that can
/// join two of its items are the ones those variables occur in. Reading them
/// through the occurrence lists rather than the whole formula is what keeps a
/// deep bag's call proportional to its own subtree.
pub(super) struct HyperedgeSource<'a> {
    formula: &'a CnfFormula,
    /// Clause indices each variable occurs in, ascending.
    occurrences: &'a [Vec<u32>],
    /// The item covering each variable, `u32::MAX` for one no item covers.
    /// Every entry is `u32::MAX` between calls.
    var_to_item: Vec<u32>,
    /// The clauses one call reads, gathered from the occurrence lists.
    clauses: Vec<u32>,
    /// Where bisections are kept between calls, when the conversion has a memo.
    bisections: Option<&'a BisectionMemo>,
}

impl<'a> HyperedgeSource<'a> {
    /// A source over `formula`, whose occurrence lists `occurrences` are,
    /// keeping its bisections in `bisections` when there is one.
    pub(super) fn new(
        formula: &'a CnfFormula,
        occurrences: &'a [Vec<u32>],
        bisections: Option<&'a BisectionMemo>,
    ) -> Self {
        HyperedgeSource {
            formula,
            occurrences,
            var_to_item: vec![u32::MAX; formula.num_vars() as usize],
            clauses: Vec::new(),
            bisections,
        }
    }

    /// The balanced bisection of `hyperedges` over `num_vertices` items, from
    /// the memo when there is one.
    fn bisect(&self, num_vertices: usize, hyperedges: Vec<Vec<u32>>, effort_scale: f64) -> Vec<u8> {
        let dials = super::super::BisectDials {
            imbalance: super::super::IMBALANCE_BALANCED,
            base_seed: 0,
            deadline: None,
        };
        match self.bisections {
            Some(memo) => memo.bisect(num_vertices, hyperedges, dials, effort_scale),
            None => super::super::multilevel_hg_bisect::multilevel_hg_bisect(
                num_vertices,
                &hyperedges,
                None,
                dials,
                effort_scale,
            ),
        }
        .expect("internally built hypergraph bisection input is valid")
    }

    /// The hyperedges joining the items `members` names, as positions in
    /// `members`: one per clause, in clause order, holding the sorted positions
    /// of the items its variables sit in, kept when it holds at least two.
    ///
    /// A variable listed by two items lands on the later one; the callers pass
    /// disjoint sets, so that does not arise.
    pub(super) fn hyperedges(
        &mut self,
        members: &[usize],
        item_vars: &[Vec<u32>],
    ) -> Vec<Vec<u32>> {
        let covered = |v: u32| (v as usize) < self.occurrences.len();
        let mut occurring = 0usize;
        for (position, &member) in members.iter().enumerate() {
            for &v in item_vars[member].iter().filter(|&&v| covered(v)) {
                self.var_to_item[v as usize] = position as u32;
                occurring += self.occurrences[v as usize].len();
            }
        }
        // The clauses to read, in ascending order. Gathering and sorting the
        // occurrences costs more than a pass over the formula once they are a
        // large share of it, and the pass reads every clause they would.
        let all_clauses = self.formula.clauses().len();
        self.clauses.clear();
        if occurring.saturating_mul(4) >= all_clauses {
            self.clauses.extend(0..all_clauses as u32);
        } else {
            for &member in members {
                for &v in item_vars[member].iter().filter(|&&v| covered(v)) {
                    self.clauses
                        .extend_from_slice(&self.occurrences[v as usize]);
                }
            }
            self.clauses.sort_unstable();
            self.clauses.dedup();
        }
        let mut hyperedges: Vec<Vec<u32>> = Vec::new();
        for &c in &self.clauses {
            let mut pins: Vec<u32> = Vec::new();
            for lit in &self.formula.clauses()[c as usize].literals {
                let v = lit.var.idx();
                if v < self.var_to_item.len() && self.var_to_item[v] != u32::MAX {
                    let item = self.var_to_item[v];
                    if !pins.contains(&item) {
                        pins.push(item);
                    }
                }
            }
            if pins.len() >= 2 {
                pins.sort_unstable();
                hyperedges.push(pins);
            }
        }
        for &member in members {
            for &v in item_vars[member].iter().filter(|&&v| covered(v)) {
                self.var_to_item[v as usize] = u32::MAX;
            }
        }
        hyperedges
    }
}

/// The clauses each variable of `formula` occurs in, ascending, indexed by
/// variable. A clause naming a variable twice is listed once for it.
pub(super) fn clause_occurrences(formula: &CnfFormula) -> Vec<Vec<u32>> {
    let mut occurrences: Vec<Vec<u32>> = vec![Vec::new(); formula.num_vars() as usize];
    for (c, clause) in formula.clauses().iter().enumerate() {
        for lit in &clause.literals {
            if let Some(list) = occurrences.get_mut(lit.var.idx())
                && list.last() != Some(&(c as u32))
            {
                list.push(c as u32);
            }
        }
    }
    occurrences
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

/// Combine items using multilevel hypergraph bisection: clauses touching ≥2
/// items become hyperedges for the multilevel partitioner. Falls back to
/// [`VtreeArena::combine_balanced`] for 3 or fewer items, a length-mismatched
/// `item_vars`, or no hyperedges. `effort_scale` is the bisector's
/// construction-effort multiplier (see [`crate::budget::vtree_effort_scale`]).
pub(super) fn combine_hypergraph_bisect(
    items: &[VtreeIdx],
    item_vars: &[Vec<u32>],
    source: &mut HyperedgeSource<'_>,
    effort_scale: f64,
    nodes: &mut VtreeArena,
) -> VtreeIdx {
    if item_vars.len() != items.len() {
        return nodes.combine_balanced(items);
    }
    let members: Vec<usize> = (0..items.len()).collect();
    bisect_members(&members, items, item_vars, source, effort_scale, nodes)
}

/// [`combine_hypergraph_bisect`] over the items `members` names, each side of
/// a split keeping the order it had in `members`.
fn bisect_members(
    members: &[usize],
    items: &[VtreeIdx],
    item_vars: &[Vec<u32>],
    source: &mut HyperedgeSource<'_>,
    effort_scale: f64,
    nodes: &mut VtreeArena,
) -> VtreeIdx {
    let balanced = |nodes: &mut VtreeArena| {
        let these: Vec<VtreeIdx> = members.iter().map(|&m| items[m]).collect();
        nodes.combine_balanced(&these)
    };
    if members.len() <= 3 {
        return balanced(nodes);
    }

    let hyperedges = source.hyperedges(members, item_vars);
    if hyperedges.is_empty() {
        return balanced(nodes);
    }

    let part = source.bisect(members.len(), hyperedges, effort_scale);

    let mut left: Vec<usize> = Vec::new();
    let mut right: Vec<usize> = Vec::new();
    for (position, &member) in members.iter().enumerate() {
        if part[position] != 0 {
            right.push(member);
        } else {
            left.push(member);
        }
    }

    // Fallback if partition is degenerate
    if left.is_empty() || right.is_empty() {
        return balanced(nodes);
    }

    let l = bisect_members(&left, items, item_vars, source, effort_scale, nodes);
    let r = bisect_members(&right, items, item_vars, source, effort_scale, nodes);

    nodes.internal(l, r)
}

/// Edge-aligned faithful combine of one TD node's children subtrees and its
/// bag-local variable leaves — see [`super::Binarization::Edge`] for the
/// algorithm.
///
/// `child_items[i]` is the already-built vtree subtree for the i-th TD child and
/// `child_sets[i]` contains the variables appearing in that TD child's bags,
/// including variables whose leaves were assigned to an ancestor. `leaf_items[j]`
/// is the vtree leaf for local variable `leaf_vars[j]` (a variable assigned to
/// THIS TD node). `primal_adj[v]` is the primal-graph neighbour list of variable
/// `v` (empty for variables with no recorded neighbours), used to route interior
/// leaves toward the child subtree holding most of their clause partners.
///
/// Deterministic: all ordering is by the caller's (deterministic) item order and
/// by ascending index on ties.
pub(super) fn combine_edge_aligned(
    child_items: &[VtreeIdx],
    child_sets: &[HashSet<u32>],
    leaf_items: &[VtreeIdx],
    leaf_vars: &[u32],
    primal_adj: &[Vec<u32>],
    nodes: &mut VtreeArena,
) -> VtreeIdx {
    debug_assert_eq!(child_items.len(), child_sets.len());
    debug_assert_eq!(leaf_items.len(), leaf_vars.len());

    let k = child_items.len();
    let m = leaf_items.len();

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
        nodes.combine_balanced(&items)
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
        child_sets,
        leaf_vars,
        aff: &aff,
        shared: &shared,
        k,
    };
    build(&all_children, &all_leaves, &ctx, nodes)
}
