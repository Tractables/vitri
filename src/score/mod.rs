//! Structural scores for a (vtree, formula) pair — what candidate selection
//! ranks on, read off the tree's shape without compiling anything.
//!
//! Three tables underlie all of them: the clause-LCA counts (each clause
//! bucketed at the single node where its variables first meet); per variable,
//! the shallowest clause-LCA it appears under, which fixes the segment of the
//! tree that variable crosses; and per node, the variables outside it that
//! share a clause with one inside. Clause load, its spread, peak context width
//! and the combined cost [`vtree_cost`] are reductions of those;
//! [`VtreeScores`] fuses the five the portfolio reads over shared scans.
//!
//! Every metric estimates a compilation COST, so lower is better in all of
//! them, and each is a prediction from shape — never a measurement.
//!
//! Nothing here knows how a vtree was built: the module reads [`crate::vtree`]
//! and [`crate::cnf`] and nothing else, which is what lets construction,
//! candidate ranking and dot rendering all score through this one owner, and
//! lets a consumer score a vtree of its own the same way.

use crate::cnf::{Clause, CnfFormula};
use crate::error::VitriError;
use crate::vtree::{VarId, Vtree, VtreeIdx};
use std::collections::{HashMap, VecDeque};

/// Check that `vtree` has a leaf for every variable `formula` names, which is
/// what every scan below indexes on.
///
/// The declared variable space is the fast answer: a formula that fits inside
/// the vtree's cannot name a variable outside it. A wider declared space is not
/// yet a mismatch — the formula may never use the ids the vtree is missing — so
/// only then do the clauses decide.
fn covered_by(vtree: &Vtree, formula: &CnfFormula) -> Result<(), VitriError> {
    let indexed = vtree.num_vars() as usize;
    if formula.num_vars as usize <= indexed {
        return Ok(());
    }
    for clause in &formula.clauses {
        for lit in &clause.literals {
            if lit.var.idx() >= indexed {
                return Err(VitriError::mismatch(format!(
                    "vtree indexes {indexed} variables but the formula names DIMACS variable {}; \
                     the vtree does not belong to this formula",
                    lit.var.to_dimacs(),
                )));
            }
        }
    }
    Ok(())
}

/// What this crate's own scoring loops assert when they cannot report an
/// error: they score a vtree against the formula they have just built it from,
/// so the covering check cannot fail on them.
pub(crate) const BUILT_FROM_THIS_FORMULA: &str = "vtree was built from this formula";

/// Call `f` with the vtree node where each non-empty clause's variables meet
/// (the LCA of its literals' leaves), in clause order.
///
/// The one scan every table below is a reduction of. A clause with no literals
/// meets nowhere and is skipped, so the clauses reported here are the non-empty
/// ones, in the order `formula` lists them.
fn for_each_clause_lca(vtree: &Vtree, formula: &CnfFormula, mut f: impl FnMut(usize, VtreeIdx)) {
    for (clause_idx, clause) in formula.clauses.iter().enumerate() {
        if let Some(lca) = clause_lca(vtree, clause) {
            f(clause_idx, lca);
        }
    }
}

/// The node where a clause's variables meet; `None` for the empty clause.
fn clause_lca(vtree: &Vtree, clause: &Clause) -> Option<VtreeIdx> {
    clause
        .literals
        .iter()
        .map(|lit| vtree.leaf_of(lit.var))
        .reduce(|a, b| vtree.lca(a, b))
}

/// For each non-empty clause, increment the count at the vtree node where the
/// clause's variables meet. Returns a vector of length `vtree.num_nodes()` with
/// clause counts per node.
fn clause_lca_counts(vtree: &Vtree, formula: &CnfFormula) -> Vec<u32> {
    let mut clause_at = vec![0u32; vtree.num_nodes()];
    for_each_clause_lca(vtree, formula, |_, lca| clause_at[lca.idx()] += 1);
    clause_at
}

/// Clause-LCA counts and the formula-clause indices contributing to each node.
#[cfg(test)]
fn clause_lca_buckets(vtree: &Vtree, formula: &CnfFormula) -> (Vec<u32>, Vec<Vec<usize>>) {
    (
        clause_lca_counts(vtree, formula),
        clause_lca_members(vtree, formula),
    )
}

fn clause_lca_members(vtree: &Vtree, formula: &CnfFormula) -> Vec<Vec<usize>> {
    let mut clauses_at = vec![Vec::new(); vtree.num_nodes()];
    for_each_clause_lca(vtree, formula, |clause_idx, lca| {
        clauses_at[lca.idx()].push(clause_idx);
    });
    clauses_at
}

/// The clause-LCA counts, together with the node each non-empty clause landed
/// on, in the order [`for_each_clause_lca`] reports them.
///
/// For a caller that has to go back from an overloaded node to the clauses
/// sitting on it, which the counts alone cannot answer.
pub(crate) fn clause_lca_nodes(vtree: &Vtree, formula: &CnfFormula) -> (Vec<VtreeIdx>, Vec<u32>) {
    let mut per_clause = Vec::with_capacity(formula.clauses.len());
    let mut clause_at = vec![0u32; vtree.num_nodes()];
    for_each_clause_lca(vtree, formula, |_, lca| {
        per_clause.push(lca);
        clause_at[lca.idx()] += 1;
    });
    (per_clause, clause_at)
}

/// [`clause_lca_counts`] under the name the rest of the crate uses for it: a
/// node's "clause load" is its clause-LCA count. This alias is the bridge
/// between the two vocabularies.
pub(crate) fn vtree_clause_load_per_node(vtree: &Vtree, formula: &CnfFormula) -> Vec<u32> {
    clause_lca_counts(vtree, formula)
}

/// Combined structural cost of a vtree: lower is better.
///
/// Every internal node `t` splits the formula's variables into the ones below
/// it and the rest. Let `w0(t)` and `w1(t)` be the smallest and second-smallest
/// of its inside-context width, outside-context width, and crossing-clause
/// count. The width terms are
///
/// ```text
/// T = log₂ Σ_t 2^w0(t)
/// E = max(0, log₂ Σ_t 2^(w0(t) + min(7, w1(t) - w0(t))) - T
///            - log₂(1 + max(0, log₂ Σ_t 2^(cross(left(t)) + cross(right(t))) - T)))
/// ```
///
/// where leaf crossing counts are capped at one and every sum omits zero
/// exponents. `C` is the clause-load cost: maximum load cubed, plus the product
/// of the two child-subtree clause counts at every join, plus each node's load
/// times the integer log of its leaf count. The remaining terms are
///
/// ```text
/// H     = log₂(1 + load_stddev) max(0, T - 16)
/// chain = log₂(1 + max(0, 5 depth - leaves - 1))
/// join  = max(0, max_t matching(t) load(t) / clause_count - 4)
/// D     = max(0, L - R - 3), when `5 depth <= leaves + 1`, and 0 otherwise
/// O     = log₂(1 + max(0, log₂ Σ_t 2^outside(t) - T - 12))
/// G     = max(0, -log₂(max(1 / leaves, 1 - depth / (leaves - 1))) - 2)
/// J     = max(0, join - 12)
/// ```
///
/// `matching(t)` is the smaller of the two maximum matchings from clauses
/// whose LCA is `t` to variables in its left and right subtrees. `join` is used
/// only when `5 depth <= leaves + 1`. `L` and `R` are the log-sum-exp
/// reductions of the inside-context widths at each internal node's left and
/// right child respectively. `O` penalizes a tight bound that is optimistic
/// relative to the outside-context bound. `G` applies only near the linear
/// end of the depth range.
///
/// For a join `t`, let `O0(t)` and `O1(t)` be the sets of outside-context
/// variables at its two children. Define
///
/// ```text
/// U(t) = w0(left(t)) + w0(right(t))
///        - min(|O0(t) ∩ O1(t)|, w0(left(t)), w0(right(t)))
/// P = max_t matching(t) load(t) log₂(1 + U(t)) / clause_count
/// A = mean of up to two largest |O0(t) ∩ O1(t)| values
/// S = max_t |O0(t) △ O1(t)|
/// Q = 0.55 min(0.25, max(0, P - 7.672358059638748))
///     + 1.5 min(1, max(0, 37 - A))
///     + 3.84 min(1, max(0, A - 22.5))
///     + 1.5 min(1, max(0, 63 - S)).
/// ```
///
/// The returned cost is
///
/// ```text
/// T + 9 log₂(1 + C)/5 + E/2 + H/25
///   + 3 chain/40 - join/2 + D/2 + 8O/5 + 4G + 32J + Q.
/// ```
///
/// It is 0 when no clause crosses a vtree cut.
///
/// Public: a caller comparing its own vtree against one this crate produced
/// scores both through this one entry rather than reimplementing the metric.
///
/// # Errors
///
/// [`VitriError::Mismatch`] if `formula` names a variable `vtree` has no leaf
/// for, which is the one way the two arguments can fail to be about the same
/// formula.
pub fn vtree_cost(vtree: &Vtree, formula: &CnfFormula) -> Result<f64, VitriError> {
    Ok(VtreeScores::compute(vtree, formula, None)?.cost)
}

fn log2_sum_exp(values: &[f64]) -> f64 {
    let peak = values
        .iter()
        .copied()
        .filter(|&value| value > 0.0)
        .reduce(f64::max);
    let Some(peak) = peak else {
        return 0.0;
    };
    peak + values
        .iter()
        .copied()
        .filter(|&value| value > 0.0)
        .map(|value| 2f64.powf(value - peak))
        .sum::<f64>()
        .log2()
}

fn separator_terms(
    vtree: &Vtree,
    ctx_in: &[u32],
    ctx_out: &[u32],
    cross: &[u32],
) -> (f64, f64, f64, Vec<u32>) {
    let mut tight_widths = vec![0u32; vtree.num_nodes()];
    let mut second = vec![0u32; vtree.num_nodes()];
    let mut cross_width = vec![0u32; vtree.num_nodes()];
    for t in vtree.bottomup() {
        let i = t.idx();
        let mut bounds = [ctx_in[i], ctx_out[i], cross[i]];
        bounds.sort_unstable();
        let leaf_cap = if vtree.node(t).is_leaf() { 1 } else { u32::MAX };
        tight_widths[i] = bounds[0].min(leaf_cap);
        second[i] = bounds[1].min(leaf_cap);
        cross_width[i] = cross[i].min(leaf_cap);
    }

    let mut tight_terms = Vec::with_capacity(vtree.num_nodes() / 2);
    let mut capped_terms = Vec::with_capacity(vtree.num_nodes() / 2);
    let mut pair_cross_terms = Vec::with_capacity(vtree.num_nodes() / 2);
    let mut out_terms = Vec::with_capacity(vtree.num_nodes() / 2);
    for (t, left, right) in vtree.internal_bottomup() {
        let i = t.idx();
        tight_terms.push(f64::from(tight_widths[i]));
        capped_terms
            .push(f64::from(tight_widths[i]) + f64::from((second[i] - tight_widths[i]).min(7)));
        pair_cross_terms
            .push(f64::from(cross_width[left.idx()]) + f64::from(cross_width[right.idx()]));
        out_terms.push(f64::from(ctx_out[i]));
    }
    let tight = log2_sum_exp(&tight_terms);
    let capped_gap = (log2_sum_exp(&capped_terms) - tight).max(0.0);
    let pair_cross_gap = (log2_sum_exp(&pair_cross_terms) - tight).max(0.0);
    let excess = (capped_gap - (1.0 + pair_cross_gap).log2()).max(0.0);
    (tight, excess, log2_sum_exp(&out_terms), tight_widths)
}

const UNIQUE_PRESSURE_THRESHOLD: f64 = 7.672_358_059_638_748;

struct ChildBoundaryFeatures {
    outside_overlap_top2_mean: f64,
    outside_symmetric_difference_max: u32,
    tight_unique_sum: Vec<u32>,
}

fn child_boundary_features(
    vtree: &Vtree,
    tight_widths: &[u32],
    outside_widths: &[u32],
    sibling_overlap: &[u32],
) -> ChildBoundaryFeatures {
    let mut largest_overlap = 0u32;
    let mut second_overlap = 0u32;
    let mut internal_count = 0u32;
    let mut symmetric_difference_max = 0u32;
    let mut tight_unique_sum = vec![0u32; vtree.num_nodes()];
    for (node, left, right) in vtree.internal_bottomup() {
        internal_count += 1;
        let overlap = sibling_overlap[node.idx()];
        if overlap >= largest_overlap {
            second_overlap = largest_overlap;
            largest_overlap = overlap;
        } else if overlap > second_overlap {
            second_overlap = overlap;
        }
        symmetric_difference_max = symmetric_difference_max
            .max(outside_widths[left.idx()] + outside_widths[right.idx()] - 2 * overlap);
        let tight_overlap = overlap
            .min(tight_widths[left.idx()])
            .min(tight_widths[right.idx()]);
        tight_unique_sum[node.idx()] =
            tight_widths[left.idx()] + tight_widths[right.idx()] - tight_overlap;
    }
    let outside_overlap_top2_mean = match internal_count {
        0 => 0.0,
        1 => f64::from(largest_overlap),
        _ => f64::from(largest_overlap + second_overlap) / 2.0,
    };
    ChildBoundaryFeatures {
        outside_overlap_top2_mean,
        outside_symmetric_difference_max: symmetric_difference_max,
        tight_unique_sum,
    }
}

fn successor_guard_correction(
    tight_unique_pressure_max: f64,
    outside_overlap_top2_mean: f64,
    outside_symmetric_difference_max: u32,
) -> f64 {
    0.55 * (tight_unique_pressure_max - UNIQUE_PRESSURE_THRESHOLD).clamp(0.0, 0.25)
        + 1.5 * (37.0 - outside_overlap_top2_mean).clamp(0.0, 1.0)
        + 3.84 * (outside_overlap_top2_mean - 22.5).clamp(0.0, 1.0)
        + 1.5 * (63.0 - f64::from(outside_symmetric_difference_max)).clamp(0.0, 1.0)
}

fn vtree_depth(vtree: &Vtree) -> u32 {
    let mut peak = 0;
    let mut stack = vec![(vtree.root(), 0)];
    while let Some((node, depth)) = stack.pop() {
        peak = peak.max(depth);
        if !vtree.node(node).is_leaf() {
            let (left, right) = vtree.children(node);
            stack.push((left, depth + 1));
            stack.push((right, depth + 1));
        }
    }
    peak
}

fn context_direction_sums(vtree: &Vtree, ctx_in: &[u32]) -> (f64, f64) {
    let mut left_terms = Vec::with_capacity(vtree.num_nodes() / 2);
    let mut right_terms = Vec::with_capacity(vtree.num_nodes() / 2);
    for (_, left, right) in vtree.internal_bottomup() {
        left_terms.push(f64::from(ctx_in[left.idx()]));
        right_terms.push(f64::from(ctx_in[right.idx()]));
    }
    (log2_sum_exp(&left_terms), log2_sum_exp(&right_terms))
}

fn directional_context_excess(vtree: &Vtree, ctx_in: &[u32], depth: u32) -> f64 {
    if 5 * u64::from(depth) > u64::from(vtree.num_leaves()) + 1 {
        return 0.0;
    }
    let (left, right) = context_direction_sums(vtree, ctx_in);
    (left - right - 3.0).max(0.0)
}

fn output_gap_bits(tight: f64, outside: f64) -> f64 {
    (1.0 + (outside - tight - 12.0).max(0.0)).log2()
}

fn extreme_chain_guard(leaves: u32, depth: u32) -> f64 {
    let leaves = f64::from(leaves.max(2));
    let depth_ratio = (f64::from(depth) / (leaves - 1.0)).min(1.0);
    let remaining = (1.0 / leaves).max(1.0 - depth_ratio);
    (-remaining.log2() - 2.0).max(0.0)
}

fn extreme_local_join_guard(join_excess: f64) -> f64 {
    (join_excess - 12.0).max(0.0)
}

fn clause_load_cost(vtree: &Vtree, clause_at: &[u32]) -> f64 {
    let mut subtree_clauses = vec![0u64; vtree.num_nodes()];
    let mut subtree_leaves = vec![0u32; vtree.num_nodes()];
    let mut child_products = 0.0;
    let mut scope = 0.0;
    for t in vtree.bottomup() {
        let i = t.idx();
        if vtree.node(t).is_leaf() {
            subtree_clauses[i] = u64::from(clause_at[i]);
            subtree_leaves[i] = 1;
            continue;
        }
        let (left, right) = vtree.children(t);
        subtree_clauses[i] =
            u64::from(clause_at[i]) + subtree_clauses[left.idx()] + subtree_clauses[right.idx()];
        subtree_leaves[i] = subtree_leaves[left.idx()] + subtree_leaves[right.idx()];
        child_products += subtree_clauses[left.idx()] as f64 * subtree_clauses[right.idx()] as f64;
        scope += f64::from(clause_at[i]) * f64::from(subtree_leaves[i].ilog2());
    }
    let max_load = f64::from(max_from_counts(clause_at));
    max_load.powi(3) + child_products + scope
}

fn maximum_matching_size(adjacency: &[Vec<usize>]) -> u32 {
    let mut pair_left = vec![None; adjacency.len()];
    let mut pair_right = HashMap::new();
    let mut left_seen = vec![0u32; adjacency.len()];
    let mut right_seen = HashMap::new();
    let mut parent_right = HashMap::new();
    let mut visit = 0u32;
    let mut size = 0u32;

    for start in 0..adjacency.len() {
        if pair_left[start].is_some() {
            continue;
        }
        visit = visit.checked_add(1).unwrap_or_else(|| {
            left_seen.fill(0);
            right_seen.clear();
            1
        });
        let mut queue = VecDeque::from([start]);
        left_seen[start] = visit;
        let mut endpoint = None;
        'search: while let Some(left) = queue.pop_front() {
            for &right in &adjacency[left] {
                if right_seen.get(&right) == Some(&visit) {
                    continue;
                }
                right_seen.insert(right, visit);
                parent_right.insert(right, left);
                match pair_right.get(&right).copied() {
                    None => {
                        endpoint = Some(right);
                        break 'search;
                    }
                    Some(mate) if left_seen[mate] != visit => {
                        left_seen[mate] = visit;
                        queue.push_back(mate);
                    }
                    Some(_) => {}
                }
            }
        }
        let Some(mut right) = endpoint else {
            continue;
        };
        loop {
            let left = parent_right[&right];
            let previous = pair_left[left];
            pair_left[left] = Some(right);
            pair_right.insert(right, left);
            let Some(previous) = previous else {
                break;
            };
            right = previous;
        }
        size += 1;
    }
    size
}

fn subtree_intervals(vtree: &Vtree) -> (Vec<u32>, Vec<u32>) {
    let mut entry = vec![0u32; vtree.num_nodes()];
    let mut exit = vec![0u32; vtree.num_nodes()];
    let mut next = 0u32;
    let mut stack = vec![(vtree.root(), false)];
    while let Some((node, leaving)) = stack.pop() {
        if leaving {
            exit[node.idx()] = next;
            continue;
        }
        entry[node.idx()] = next;
        next += 1;
        stack.push((node, true));
        if !vtree.node(node).is_leaf() {
            let (left, right) = vtree.children(node);
            stack.push((right, false));
            stack.push((left, false));
        }
    }
    (entry, exit)
}

#[cfg(test)]
fn local_join_match_excess(
    vtree: &Vtree,
    formula: &CnfFormula,
    clauses_at: &[Vec<usize>],
    clause_count: u64,
) -> f64 {
    local_join_features(
        vtree,
        formula,
        clauses_at,
        clause_count,
        true,
        &vec![0; vtree.num_nodes()],
    )
    .0
}

fn local_join_features(
    vtree: &Vtree,
    formula: &CnfFormula,
    clauses_at: &[Vec<usize>],
    clause_count: u64,
    shallow: bool,
    tight_unique_sum: &[u32],
) -> (f64, f64) {
    if clauses_at.is_empty() {
        return (0.0, 0.0);
    }
    let (entry, exit) = subtree_intervals(vtree);
    let mut peak_excess = 0.0f64;
    let mut tight_unique_pressure_max = 0.0f64;
    for (t, left, _) in vtree.internal_bottomup() {
        let clause_ids = &clauses_at[t.idx()];
        if clause_ids.is_empty() {
            continue;
        }
        let load = clause_ids.len() as u64;
        let unique_scale = (1.0 + f64::from(tight_unique_sum[t.idx()])).log2();
        let density_upper = load as f64 * load as f64 / clause_count.max(1) as f64;
        let can_clear_join = shallow && density_upper > 4.0;
        let can_clear_pressure = density_upper * unique_scale > UNIQUE_PRESSURE_THRESHOLD;
        // `matching <= load`, so neither correction can activate at this node.
        if !can_clear_join && !can_clear_pressure {
            continue;
        }
        let mut left_adjacency = Vec::with_capacity(clause_ids.len());
        let mut right_adjacency = Vec::with_capacity(clause_ids.len());
        for &clause_idx in clause_ids {
            let mut left_vars = Vec::new();
            let mut right_vars = Vec::new();
            for lit in &formula.clauses[clause_idx].literals {
                let var = lit.var.idx();
                let leaf = vtree.leaf_of(lit.var);
                if entry[left.idx()] <= entry[leaf.idx()] && entry[leaf.idx()] < exit[left.idx()] {
                    left_vars.push(var);
                } else {
                    right_vars.push(var);
                }
            }
            left_vars.sort_unstable();
            left_vars.dedup();
            right_vars.sort_unstable();
            right_vars.dedup();
            left_adjacency.push(left_vars);
            right_adjacency.push(right_vars);
        }
        let matching =
            maximum_matching_size(&left_adjacency).min(maximum_matching_size(&right_adjacency));
        let density = f64::from(matching) * clause_ids.len() as f64 / clause_count.max(1) as f64;
        if shallow {
            peak_excess = peak_excess.max(density - 4.0);
        }
        tight_unique_pressure_max = tight_unique_pressure_max.max(density * unique_scale);
    }
    (peak_excess, tight_unique_pressure_max)
}

struct UnifiedCostTables<'a> {
    clause_at: &'a [u32],
    ctx_in: &'a [u32],
    ctx_out: &'a [u32],
    sibling_overlap: &'a [u32],
    cross: &'a [u32],
}

fn unified_cost_from_tables(
    vtree: &Vtree,
    formula: &CnfFormula,
    tables: UnifiedCostTables<'_>,
    load_stddev: f64,
    depth: u32,
) -> f64 {
    let (tight, excess, outside, tight_widths) =
        separator_terms(vtree, tables.ctx_in, tables.ctx_out, tables.cross);
    if tight == 0.0 {
        return 0.0;
    }
    let child_boundaries =
        child_boundary_features(vtree, &tight_widths, tables.ctx_out, tables.sibling_overlap);
    let clause_load_cost = clause_load_cost(vtree, tables.clause_at);
    let leaves = f64::from(vtree.num_leaves());
    let chain = (1.0 + (5.0 * f64::from(depth) - leaves - 1.0).max(0.0)).log2();
    let high_load = (1.0 + load_stddev).log2() * (tight - 16.0).max(0.0);
    let clause_count: u64 = tables.clause_at.iter().map(|&load| u64::from(load)).sum();
    let shallow = 5 * u64::from(depth) <= u64::from(vtree.num_leaves()) + 1;
    let needs_matching = vtree.internal_bottomup().any(|(node, _, _)| {
        let load = f64::from(tables.clause_at[node.idx()]);
        let density_upper = load * load / clause_count.max(1) as f64;
        let unique_scale = (1.0 + f64::from(child_boundaries.tight_unique_sum[node.idx()])).log2();
        (shallow && density_upper > 4.0) || density_upper * unique_scale > UNIQUE_PRESSURE_THRESHOLD
    });
    let clauses_at = if needs_matching {
        clause_lca_members(vtree, formula)
    } else {
        Vec::new()
    };
    let (join, tight_unique_pressure) = local_join_features(
        vtree,
        formula,
        &clauses_at,
        clause_count,
        shallow,
        &child_boundaries.tight_unique_sum,
    );
    let directional_context = directional_context_excess(vtree, tables.ctx_in, depth);
    let output_gap = output_gap_bits(tight, outside);
    let extreme_chain = extreme_chain_guard(vtree.num_leaves(), depth);
    let extreme_join = extreme_local_join_guard(join);
    let successor_guard = successor_guard_correction(
        tight_unique_pressure,
        child_boundaries.outside_overlap_top2_mean,
        child_boundaries.outside_symmetric_difference_max,
    );
    tight
        + excess / 2.0
        + 9.0 * (1.0 + clause_load_cost).log2() / 5.0
        + high_load / 25.0
        + 3.0 * chain / 40.0
        - join / 2.0
        + directional_context / 2.0
        + 8.0 * output_gap / 5.0
        + 4.0 * extreme_chain
        + 32.0 * extreme_join
        + successor_guard
}

/// Maximum clause load: the largest number of clauses whose LCA is any single
/// vtree node.
pub(crate) fn vtree_max_clause_load(vtree: &Vtree, formula: &CnfFormula) -> u32 {
    max_from_counts(&clause_lca_counts(vtree, formula))
}

/// Core of [`vtree_max_clause_load`] over a precomputed clause-LCA count table.
fn max_from_counts(clause_at: &[u32]) -> u32 {
    clause_at.iter().copied().max().unwrap_or(0)
}

/// Standard deviation of clause loads across vtree nodes, over a precomputed
/// clause-LCA count table: for each node, the "clause load" is the number of
/// clauses whose LCA is that node. The canonical arithmetic, so
/// `VtreeScores::compute` and the pin that checks it against a separately
/// spelled-out computation cannot drift apart.
fn stddev_from_counts(clause_at: &[u32]) -> f64 {
    load_stats(clause_at, |_| true).stddev
}

/// What a load table says about the nodes carrying a load.
pub(crate) struct LoadStats {
    /// Mean load over the counted nodes, `0.0` when none is counted.
    pub(crate) mean: f64,
    /// Sample standard deviation of that load, `0.0` below two counted nodes.
    pub(crate) stddev: f64,
    /// How many nodes were counted.
    pub(crate) count: usize,
}

/// Summarize a per-node load table over the loaded nodes `keep` admits.
///
/// A node with no load is never counted: it is a node no clause chose, not a
/// node that carries an unusually light one, so counting it would pull the mean
/// toward zero and report a spread that is mostly the empty tree. `keep` narrows
/// that further for a caller reading only part of the tree.
pub(crate) fn load_stats(loads: &[u32], keep: impl Fn(VtreeIdx) -> bool) -> LoadStats {
    let mut sum: f64 = 0.0;
    let mut sum_sq: f64 = 0.0;
    let mut count: usize = 0;
    for (idx, &load) in loads.iter().enumerate() {
        if load > 0 && keep(VtreeIdx(idx as u32)) {
            sum += load as f64;
            sum_sq += (load as f64) * (load as f64);
            count += 1;
        }
    }

    if count == 0 {
        return LoadStats {
            mean: 0.0,
            stddev: 0.0,
            count: 0,
        };
    }

    let mean = sum / count as f64;
    let stddev = if count == 1 {
        0.0
    } else {
        ((sum_sq - sum * mean) / (count - 1) as f64).max(0.0).sqrt()
    };
    LoadStats {
        mean,
        stddev,
        count,
    }
}

/// Context width per vtree node: the number of *distinct* variables in
/// `subtree(t)` that also appear in a clause crossing `t`'s boundary (a clause
/// whose LCA is a strict ancestor of `t`) — the inside end of the separator at
/// `t`. Unlike the `clause_load_*` metrics (which only count clauses bucketed
/// at their LCA), this measures how many variables leak across each split.
/// `2^ctx[t]` is not a bound on the diagram at `t` (a single inside variable
/// under the clauses `a ∨ c` and `¬a ∨ d` already has three subfunctions), and
/// the peak alone is a rough predictor of compile size; [`vtree_cost`] reads
/// it together with the outside end.
///
/// A variable `v` crosses node `t` iff `t` lies strictly between `leaf(v)` and
/// the *shallowest* clause-LCA among clauses containing `v` (shallowest = the
/// widest-spanning clause, so it gives the longest crossing segment). We find
/// that shallowest LCA per variable (closest to root = largest `topo_pos`),
/// then walk `leaf → ancestor`, incrementing each node on the segment.
///
/// Returns the per-node context-width array, length `vtree.num_nodes()`.
/// Cost: O(|clause literals| × vtree_depth).
///
/// `show` restricts the count to the shown variables, which is the binding cost
/// under PROJECTED counting: a hidden variable crossing a cut is ∃-forgotten
/// when its scope completes, collapsing that part of the frontier, whereas a
/// shown variable persists to the root. So a vtree whose wide cuts are dominated
/// by hidden variables compiles cheaply under ∃-forget even though its all-var
/// peak is large — and conversely a low all-var peak can hide a show-heavy
/// separator that blows up. The crossing structure is computed over ALL clauses
/// either way; only the per-node accumulation is filtered.
pub(crate) fn vtree_context_width_per_node(
    vtree: &Vtree,
    formula: &CnfFormula,
    show: Option<&crate::cnf::ShowMask>,
) -> Vec<u32> {
    let high_lca = clause_high_lca(vtree, formula);
    context_width_from_high_lca(vtree, &high_lca, show)
}

/// The context-width walk over a `high_lca` table the caller already has, which
/// is what lets `VtreeScores::compute` count both widths off ONE shared table.
fn context_width_from_high_lca(
    vtree: &Vtree,
    high_lca: &[Option<VtreeIdx>],
    show: Option<&crate::cnf::ShowMask>,
) -> Vec<u32> {
    let mut ctx = vec![0u32; vtree.num_nodes()];
    for (vi, &lca) in high_lca.iter().enumerate() {
        if let Some(mask) = show
            && !mask.as_slice().get(vi).copied().unwrap_or(false)
        {
            continue;
        }
        if let Some(l) = lca {
            let leaf = vtree.leaf_of(VarId(vi as u32));
            let mut cur = vtree.node(leaf).parent();
            while let Some(node) = cur {
                if node == l {
                    break; // reached the clause LCA — stop before it
                }
                ctx[node.idx()] += 1;
                cur = vtree.node(node).parent();
            }
        }
    }

    ctx
}

/// Outside context width per vtree node: the number of *distinct* variables
/// OUTSIDE `subtree(t)` that share a clause with a variable inside it — the
/// outside end of the same clauses [`vtree_context_width_per_node`] counts the
/// inside end of. The subfunctions the compiler can form at `t` are indexed by
/// an assignment to these variables.
///
/// Per variable `v`: every node that contains a clause-mate of `v` but not `v`
/// itself, which is every node strictly below `lca(v, u)` on the path up from
/// `leaf(u)`, for each mate `u`. A stamp per variable keeps a node counted
/// once for `v` however many mates reach it and ends each walk at the first
/// node already stamped, so the work is the number of (node, variable) pairs
/// marked plus one pass over every clause per variable it contains.
///
/// Returns the per-node array, length `vtree.num_nodes()`. A leaf's entry
/// counts the mates of its own variable.
#[cfg(test)]
pub(crate) fn vtree_outside_context_width_per_node(
    vtree: &Vtree,
    formula: &CnfFormula,
) -> Vec<u32> {
    outside_context_tables(vtree, formula).widths
}

struct OutsideContextTables {
    widths: Vec<u32>,
    sibling_overlap: Vec<u32>,
}

fn outside_context_tables(vtree: &Vtree, formula: &CnfFormula) -> OutsideContextTables {
    let n_vars = vtree.num_vars() as usize;
    let (pos, neg) = crate::cnf::occ::occurrence_lists(&formula.clauses, n_vars);
    let nn = vtree.num_nodes();
    let mut ctx_out = vec![0u32; nn];
    let mut sibling_overlap = vec![0u32; nn];
    // `stamp[t] == v` marks node `t` as settled for variable `v`: either it
    // contains `v`, or a mate's walk has already counted `v` there.
    let mut stamp: Vec<u32> = vec![u32::MAX; nn];
    // Unlike `stamp`, this marks only nodes where `v` was outside. It lets a
    // parent count variables outside both children without retaining one set
    // per node.
    let mut outside_stamp: Vec<u32> = vec![u32::MAX; nn];
    for (v, (in_pos, in_neg)) in pos.iter().zip(&neg).enumerate() {
        if in_pos.is_empty() && in_neg.is_empty() {
            continue;
        }
        let v_id = v as u32;
        let mut cur = Some(vtree.leaf_of(VarId(v_id)));
        while let Some(node) = cur {
            stamp[node.idx()] = v_id;
            cur = vtree.node(node).parent();
        }
        for &ci in in_pos.iter().chain(in_neg) {
            for lit in &formula.clauses[ci].literals {
                if lit.var.idx() == v {
                    continue;
                }
                let mut cur = Some(vtree.leaf_of(lit.var));
                while let Some(node) = cur {
                    if stamp[node.idx()] == v_id {
                        break;
                    }
                    stamp[node.idx()] = v_id;
                    ctx_out[node.idx()] += 1;
                    outside_stamp[node.idx()] = v_id;
                    if let Some(parent) = vtree.node(node).parent() {
                        let (left, right) = vtree.children(parent);
                        let sibling = if node == left { right } else { left };
                        if outside_stamp[sibling.idx()] == v_id {
                            sibling_overlap[parent.idx()] += 1;
                        }
                    }
                    cur = vtree.node(node).parent();
                }
            }
        }
    }
    OutsideContextTables {
        widths: ctx_out,
        sibling_overlap,
    }
}

/// Crossing clauses per vtree node: the number of clauses with a variable
/// inside `subtree(t)` and one outside it, i.e. with at least two literals and
/// an LCA strictly above `t`. The third count the separator at `t` can be
/// measured by, beside the two variable counts.
///
/// Per clause: its LCA is stamped, then each literal's leaf walks up until it
/// reaches a node already stamped for this clause, counting the nodes it
/// passes. The nodes counted are exactly the union of the leaf-to-LCA paths
/// below the LCA, each once.
///
/// Returns the per-node array, length `vtree.num_nodes()`.
pub(crate) fn vtree_crossing_clauses_per_node(vtree: &Vtree, formula: &CnfFormula) -> Vec<u32> {
    let nn = vtree.num_nodes();
    let mut cross = vec![0u32; nn];
    let mut stamp: Vec<usize> = vec![usize::MAX; nn];
    for (ci, clause) in formula.clauses.iter().enumerate() {
        if clause.literals.len() < 2 {
            continue;
        }
        let lca = clause_lca(vtree, clause).expect("a clause with two literals has an LCA");
        stamp[lca.idx()] = ci;
        for lit in &clause.literals {
            let mut cur = vtree.leaf_of(lit.var);
            while stamp[cur.idx()] != ci {
                stamp[cur.idx()] = ci;
                cross[cur.idx()] += 1;
                cur = vtree
                    .node(cur)
                    .parent()
                    .expect("a node below the clause LCA has a parent");
            }
        }
    }
    cross
}

/// Shallowest (closest-to-root) clause-LCA per variable; `None` = the var never
/// crosses a node boundary (only appears in unit/empty clauses). Shared by the
/// all-var and `keep`-restricted context-width metrics — the crossing structure
/// is identical; only the per-node accumulation differs.
fn clause_high_lca(vtree: &Vtree, formula: &CnfFormula) -> Vec<Option<VtreeIdx>> {
    let n_vars = vtree.num_vars() as usize;
    let mut high_lca: Vec<Option<VtreeIdx>> = vec![None; n_vars];
    for clause in &formula.clauses {
        if clause.literals.len() < 2 {
            continue; // unit/empty clause crosses no node boundary
        }
        let lca = clause_lca(vtree, clause).expect("a clause with two literals has an LCA");
        let lpos = vtree.topo_pos(lca);
        for lit in &clause.literals {
            let vi = lit.var.idx();
            let replace = match high_lca[vi] {
                Some(cur) => lpos > vtree.topo_pos(cur),
                None => true,
            };
            if replace {
                high_lca[vi] = Some(lca);
            }
        }
    }
    high_lca
}

/// All five structural selection metrics for one realized vtree. Candidate
/// scoring stores this value and shares the intermediate tables across fields.
///
/// # Every field is LOWER-IS-BETTER
///
/// All five estimate a *cost* of compiling `formula` under `vtree`, so a smaller
/// number is a better vtree in that dimension. None of them is measured — they
/// are structural predictions, computed without compiling anything.
///
/// | field | estimates | unit |
/// | --- | --- | --- |
/// | [`clause_load_stddev`](Self::clause_load_stddev) | how EVENLY clauses spread over the tree: the standard deviation of per-node clause load, where a clause's node is the LCA of its variables' leaves. A lopsided tree piles work onto one node. | clauses |
/// | [`max_clause_load`](Self::max_clause_load) | the WORST single node: the largest number of clauses landing on any one vtree node. | clauses |
/// | [`peak_context_width_all`](Self::peak_context_width_all) | the widest CUT seen from inside: the largest number, over all nodes, of variables in a subtree that also occur in a clause crossing out of it. A rough predictor on its own. | variables |
/// | [`peak_context_width_show`](Self::peak_context_width_show) | the same peak counting only SHOW (projected-kept) variables, or `None` for a non-projected instance. | variables |
/// | [`cost`](Self::cost) | the combined structural cost used for plain candidate ranking. See [`vtree_cost`]. | score |
///
/// # Visibility
///
/// `pub` so a consumer can read the same five numbers the selector ranks on
/// without a second copy of the metric code, and can score any
/// `(vtree, formula)` pair — including a vtree against a formula it was not
/// built from, which [`compute`](Self::compute) answers for rather than
/// panicking on. It is the score payload of every entry in an emitted candidate
/// set ([`crate::candidates`]), which a consumer re-ranking that set reads
/// directly.
/// `Serialize` so an emitted candidate set carries the scores verbatim into
/// `components.json` — the exported numbers ARE these fields, not a
/// hand-maintained JSON mirror that could drift from what selection ranked on.
/// `Deserialize` so reading the manifest back yields the same type it was
/// written from.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VtreeScores {
    /// Standard deviation of per-node clause load. Lower is better.
    pub clause_load_stddev: f64,
    /// Largest clause load on any single node. Lower is better.
    pub max_clause_load: u32,
    /// Largest all-variable context width over all nodes. Lower is better.
    pub peak_context_width_all: u32,
    /// Largest show-variable context width, or `None` without a show mask.
    /// Lower is better.
    ///
    /// Present or absent for a whole run at once: the show mask comes from the
    /// selection context, which is fixed before any candidate is built, so a
    /// set of scores from one run never mixes the two.
    pub peak_context_width_show: Option<u32>,
    /// Combined structural cost, [`vtree_cost`]. Lower is better.
    pub cost: f64,
}

impl VtreeScores {
    /// Compute all five fields for `vtree` against `formula` from shared
    /// clause-LCA tables. `show_mask` is `Some` only for projected
    /// (show-variable) selection; without it `peak_context_width_show` is `None`.
    ///
    /// # Errors
    ///
    /// [`VitriError::Mismatch`] if `formula` names a variable `vtree` has no
    /// leaf for, which is the one way the two arguments can fail to be about
    /// the same formula.
    pub fn compute(
        vtree: &Vtree,
        formula: &CnfFormula,
        show_mask: Option<&crate::cnf::ShowMask>,
    ) -> Result<Self, VitriError> {
        covered_by(vtree, formula)?;
        let depth = vtree_depth(vtree);
        let clause_at = clause_lca_counts(vtree, formula);
        let max_clause_load = max_from_counts(&clause_at);
        let high_lca = clause_high_lca(vtree, formula);
        let ctx_in = context_width_from_high_lca(vtree, &high_lca, None);
        let outside = outside_context_tables(vtree, formula);
        let cross = vtree_crossing_clauses_per_node(vtree, formula);
        let clause_load_stddev = stddev_from_counts(&clause_at);
        Ok(Self {
            clause_load_stddev,
            max_clause_load,
            peak_context_width_all: ctx_in.iter().copied().max().unwrap_or(0),
            peak_context_width_show: show_mask.map(|m| {
                context_width_from_high_lca(vtree, &high_lca, Some(m))
                    .into_iter()
                    .max()
                    .unwrap_or(0)
            }),
            cost: unified_cost_from_tables(
                vtree,
                formula,
                UnifiedCostTables {
                    clause_at: &clause_at,
                    ctx_in: &ctx_in,
                    ctx_out: &outside.widths,
                    sibling_overlap: &outside.sibling_overlap,
                    cross: &cross,
                },
                clause_load_stddev,
                depth,
            ),
        })
    }
}

#[cfg(test)]
mod tests;

/// How evenly a formula's clause widths and variable occurrences are spread,
/// and the near-uniform verdict two of this crate's decisions read off them.
///
/// A formula whose clause widths and variable occurrences are both near-uniform
/// is shaped like a graph-colouring encoding. Two independent decisions here
/// consult that: Arjun's bounded-variable-addition policy under
/// [`ArjunSbva::Auto`](crate::preprocess::ArjunSbva::Auto), which skips the pass
/// on such an input, and the vtree portfolio's candidate gate. Both call this,
/// so a caller reporting these numbers is reporting what those decisions saw —
/// not a second measurement that agrees with them today.
///
/// Both coefficients are dispersion relative to the mean, so `0.0` is perfectly
/// uniform and there is no upper bound. A formula too small to have a spread —
/// fewer than two clauses, fewer than two occurring variables — scores `0.0`,
/// which reads as uniform.
///
/// `#[non_exhaustive]`: the verdict may come to read a third statistic, and a
/// caller that only prints these two should not have to be recompiled for that.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct StructureProfile {
    /// Coefficient of variation of clause width — the standard deviation of the
    /// clause lengths over their mean.
    pub clause_width_cv: f64,
    /// Coefficient of variation of per-variable occurrence count, over the
    /// variables that occur at all. A variable in no clause is not a variable
    /// with an occurrence count of zero for this purpose; it is absent.
    pub var_occurrence_cv: f64,
    /// Whether both coefficients are inside the thresholds that make an input
    /// look like a graph-colouring encoding.
    pub coloring_like: bool,
}

impl StructureProfile {
    /// Construct a profile from already-measured coefficients.
    ///
    /// This is the counterpart to [`StructureProfile::measure`] for an
    /// embedding that already owns the source formula's statistics and should
    /// not scan or reconstruct that formula merely to pass its profile into a
    /// selection context.
    pub fn from_coefficients(clause_width_cv: f64, var_occurrence_cv: f64) -> Self {
        StructureProfile {
            clause_width_cv,
            var_occurrence_cv,
            coloring_like: crate::cnf::stats::coloring_like_predicate(
                var_occurrence_cv,
                clause_width_cv,
            ),
        }
    }

    /// Measure `formula`. One scan of the clause set for each coefficient.
    ///
    /// This is a measurement of the formula it is handed, so a preprocessed
    /// formula and the raw one it came from can profile differently — which of
    /// the two a decision should read is that decision's to settle.
    pub fn measure(formula: &CnfFormula) -> Self {
        let clause_width_cv = crate::cnf::stats::clause_width_cv(formula);
        let var_occurrence_cv = crate::cnf::stats::var_occurrence_cv(formula);
        StructureProfile::from_coefficients(clause_width_cv, var_occurrence_cv)
    }
}
