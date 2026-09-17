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

use crate::cnf::CnfFormula;
use crate::error::VitriError;
use crate::vtree::{Vtree, VtreeIdx};
use std::collections::{HashMap, VecDeque};

pub(crate) mod agg;
/// A formula's shape, which structure-sensitive selection reads. Defined in
/// [`crate::cnf`], beside the statistics it is made of, and named here because
/// selection is what consults it.
pub use crate::cnf::StructureProfile;
pub use agg::DEFAULT_MARGIN;
mod per_node;
pub(crate) mod tables;

use per_node::{
    clause_high_lca, clause_lca_members, context_width_from_high_lca, max_from_counts, node_depths,
    outside_context_tables, stddev_from_counts, subtree_intervals, subtree_tables,
};
pub(crate) use per_node::{
    clause_lca_counts, clause_lca_nodes, load_stats, vtree_context_width_per_node,
    vtree_crossing_clauses_per_node,
};

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

/// Combined structural cost of a vtree: lower is better, and 0 when no clause
/// crosses a vtree cut.
///
/// Every internal node `t` splits the formula's variables into the ones below
/// it and the rest. The leading term is `log₂ Σ_t 2^w(t)` over the internal
/// nodes, where `w(t)` is the node's crossing-clause count scaled by its
/// inside-context width over the formula's clause count. The rest are
/// penalties, each named in [`COST_TERM_NAMES`] and computed by
/// [`vtree_cost_terms`], which returns them separately and already weighted:
/// the clause load one node carries and the spread of that load, the
/// second-best split available at each node, chain-shaped and near-linear
/// trees, joins whose two sides share an outside context, and a leading term
/// that looks optimistic beside the outside-context widths. The weights are fitted, so a
/// cost ranks trees over one formula rather than measuring one tree.
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

/// Which whole-tree aggregate ranker selects a portfolio build's candidates.
///
/// A description of the choice, not a loaded ranker: the model behind
/// [`Self::Shipped`] and [`Self::File`] is read and parsed once per process,
/// the first time a build needs it.
///
/// Set on
/// [`PortfolioKnobs::ranker`](crate::decompose::PortfolioKnobs::ranker), where
/// `VITRI_SCORE_AGG` fills it in
/// [`SelectionCtx::with_env_defaults`](crate::decompose::SelectionCtx::with_env_defaults).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Ranker {
    /// The ranker this crate ships: a pairwise model over the cost addends,
    /// fitted on the portfolio's own candidates. The default.
    #[default]
    Shipped,
    /// A ranker read from this JSON file, in the shipped ranker's format.
    File(std::path::PathBuf),
    /// No ranker: candidates are selected on [`vtree_cost`].
    Off,
}

/// Refuse a scoring setting this process cannot honour.
///
/// The same reads a `portfolio` build makes, exposed so a consumer can make
/// them at argv time: without this a setting is only refused once a
/// construction starts, which in a consumer that treats a construction failure
/// as a panic is a worse report of the same typo, minutes later.
///
/// # Errors
///
/// [`VitriError::Env`] when `VITRI_SCORE_AGG` is neither `cost` nor a file
/// that is a ranker this crate can evaluate, or when `VITRI_SCORE_AGG_MARGIN`
/// is set under `cost`, where there is no ranker to narrow, or to something
/// that is not a margin.
pub fn check_score_env() -> Result<(), VitriError> {
    let ranker = agg::model()?;
    agg::margin_from_env(ranker.is_some())?;
    Ok(())
}

/// `log2 Σ 2^v` over the strictly positive entries, max-shifted; 0 when none is
/// positive. The convention every score in this module and the offline ranker
/// tables were built with.
pub(super) fn log2_sum_exp(values: &[f64]) -> f64 {
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

/// The width of one internal cut for the `T` term: the crossing-clause count
/// scaled by the inside width over the clause count. It is at most the inside
/// width, and is the inside width when every clause crosses the cut.
///
/// It is not the smallest of the three bounds, which the excess term still
/// measures against. As the width `T` sums, that minimum favours a cut whose
/// outside width is small, and the chain a shallow edge-binarized reading
/// builds keeps the outside width small at every node while carrying most of
/// the formula across each cut. Those trees do not compile, and ranking them by
/// the minimum ranks them above trees that do.
fn cut_width(ctx_in: u32, cross: u32, clause_count: u64) -> f64 {
    f64::from(cross) * f64::from(ctx_in) / clause_count.max(1) as f64
}

/// The three bounds on the separator at one node — inside width, outside width
/// and crossing-clause count — in increasing order, capped at one for a leaf.
///
/// The one spelling of "the smallest of the three bounds": `separator_terms`
/// reads the first two entries, the per-node tables the first.
fn sorted_bounds(ctx_in: u32, ctx_out: u32, cross: u32, is_leaf: bool) -> [u32; 3] {
    let mut bounds = [ctx_in, ctx_out, cross];
    bounds.sort_unstable();
    if is_leaf {
        for bound in &mut bounds {
            *bound = (*bound).min(1);
        }
    }
    bounds
}

fn separator_terms(
    vtree: &Vtree,
    ctx_in: &[u32],
    ctx_out: &[u32],
    cross: &[u32],
    clause_count: u64,
) -> (f64, f64, f64, Vec<u32>) {
    let mut tight_widths = vec![0u32; vtree.num_nodes()];
    let mut second = vec![0u32; vtree.num_nodes()];
    let mut cross_width = vec![0u32; vtree.num_nodes()];
    let mut width = vec![0f64; vtree.num_nodes()];
    for t in vtree.bottomup() {
        let i = t.idx();
        let is_leaf = vtree.node(t).is_leaf();
        let bounds = sorted_bounds(ctx_in[i], ctx_out[i], cross[i], is_leaf);
        let leaf_cap = if is_leaf { 1 } else { u32::MAX };
        tight_widths[i] = bounds[0];
        second[i] = bounds[1];
        cross_width[i] = cross[i].min(leaf_cap);
        width[i] = if is_leaf {
            f64::from(tight_widths[i])
        } else {
            cut_width(ctx_in[i], cross[i], clause_count)
        };
    }

    let mut tight_terms = Vec::with_capacity(vtree.num_nodes() / 2);
    let mut bound_terms = Vec::with_capacity(vtree.num_nodes() / 2);
    let mut capped_terms = Vec::with_capacity(vtree.num_nodes() / 2);
    let mut pair_cross_terms = Vec::with_capacity(vtree.num_nodes() / 2);
    let mut out_terms = Vec::with_capacity(vtree.num_nodes() / 2);
    for (t, left, right) in vtree.internal_bottomup() {
        let i = t.idx();
        tight_terms.push(width[i]);
        bound_terms.push(f64::from(tight_widths[i]));
        capped_terms
            .push(f64::from(tight_widths[i]) + f64::from((second[i] - tight_widths[i]).min(7)));
        pair_cross_terms
            .push(f64::from(cross_width[left.idx()]) + f64::from(cross_width[right.idx()]));
        out_terms.push(f64::from(ctx_out[i]));
    }
    let tight = log2_sum_exp(&tight_terms);
    // The excess term is the gap between the smallest and the second-smallest
    // of the three bounds, so it is measured against the sum of the smallest.
    let tight_bound = log2_sum_exp(&bound_terms);
    let capped_gap = (log2_sum_exp(&capped_terms) - tight_bound).max(0.0);
    let pair_cross_gap = (log2_sum_exp(&pair_cross_terms) - tight_bound).max(0.0);
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
    node_depths(vtree).into_iter().max().unwrap_or(0)
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
    let subtree = subtree_tables(vtree, clause_at);
    let mut child_products = 0.0;
    let mut scope = 0.0;
    for (t, left, right) in vtree.internal_bottomup() {
        child_products += subtree.clauses[left.idx()] as f64 * subtree.clauses[right.idx()] as f64;
        scope += f64::from(clause_at[t.idx()]) * f64::from(subtree.leaves[t.idx()].ilog2());
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

/// Every clause bucketed at one node, split into the literals whose variables
/// sit in the node's left subtree and the ones that do not, each side a sorted
/// set. `entry` and `exit` are the subtree intervals from
/// [`subtree_intervals`].
///
/// The one place a node's split is read off the tree. A clause is read as the
/// SET of its literals, so a repeated one counts once, whether or not the
/// formula reached here through a parser that had already dropped it.
pub(super) fn split_at_node(
    clause_ids: &[usize],
    formula: &CnfFormula,
    vtree: &Vtree,
    left: VtreeIdx,
    entry: &[u32],
    exit: &[u32],
) -> Vec<(Vec<i32>, Vec<i32>)> {
    clause_ids
        .iter()
        .map(|&clause_idx| {
            let mut left_literals = Vec::new();
            let mut right_literals = Vec::new();
            for lit in &formula.clauses[clause_idx].literals {
                let leaf = vtree.leaf_of(lit.var).idx();
                if entry[left.idx()] <= entry[leaf] && entry[leaf] < exit[left.idx()] {
                    left_literals.push(lit.to_dimacs());
                } else {
                    right_literals.push(lit.to_dimacs());
                }
            }
            for side in [&mut left_literals, &mut right_literals] {
                side.sort_unstable();
                side.dedup();
            }
            (left_literals, right_literals)
        })
        .collect()
}

/// The variables a set of literals is over, sorted, each once.
pub(super) fn variables_of(literals: &[i32]) -> Vec<usize> {
    let mut vars: Vec<usize> = literals.iter().map(|l| l.unsigned_abs() as usize).collect();
    vars.sort_unstable();
    vars.dedup();
    vars
}

/// The unique-boundary scale at a node whose matching corrections can activate,
/// `None` when neither can: `matching <= load` bounds both below their
/// thresholds, so a node this rejects needs no matching computed at all.
///
/// The pre-pass that decides whether the clause lists are built and the loop
/// that reads them ask this same question, so they ask it here.
fn matching_activates(load: u64, unique_sum: u32, clause_count: u64, shallow: bool) -> Option<f64> {
    let load = load as f64;
    let density_upper = load * load / clause_count.max(1) as f64;
    let unique_scale = (1.0 + f64::from(unique_sum)).log2();
    ((shallow && density_upper > 4.0) || density_upper * unique_scale > UNIQUE_PRESSURE_THRESHOLD)
        .then_some(unique_scale)
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
        let Some(unique_scale) =
            matching_activates(load, tight_unique_sum[t.idx()], clause_count, shallow)
        else {
            continue;
        };
        let split = split_at_node(clause_ids, formula, vtree, left, &entry, &exit);
        let left_adjacency: Vec<Vec<usize>> = split.iter().map(|(l, _)| variables_of(l)).collect();
        let right_adjacency: Vec<Vec<usize>> = split.iter().map(|(_, r)| variables_of(r)).collect();
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

/// The eleven weighted addends [`vtree_cost`] sums, in the order it sums them
/// and under the names an offline fit reads them by.
pub const COST_TERM_NAMES: [&str; 11] = [
    "tight",
    "excess_half",
    "clause_load_bits",
    "high_load_25",
    "chain_3_40",
    "join_neg_half",
    "directional_half",
    "output_gap_16",
    "extreme_chain_4",
    "extreme_join_32",
    "successor_guard",
];

/// The eleven addends of [`vtree_cost`], in [`COST_TERM_NAMES`] order, each
/// already carrying its coefficient. They sum to the cost, term by term in
/// that order, so an offline fit reads exactly the quantities the cost adds.
///
/// # Errors
///
/// [`VitriError::Mismatch`] if `formula` names a variable `vtree` has no leaf
/// for.
pub fn vtree_cost_terms(vtree: &Vtree, formula: &CnfFormula) -> Result<[f64; 11], VitriError> {
    covered_by(vtree, formula)?;
    let tables = tables::Tables::build(vtree, formula, false, false);
    Ok(unified_cost_terms(
        vtree,
        formula,
        tables.cost_tables(),
        stddev_from_counts(tables.clause_at()),
        vtree_depth(vtree),
    ))
}

/// The eleven addends of the cost, each already carrying its coefficient, in
/// [`COST_TERM_NAMES`] order.
///
/// Split out from the sum so a caller that ranks trees by a function of the
/// individual terms reads the ones the cost computed rather than a second
/// spelling of them.
pub(in crate::score) fn unified_cost_terms(
    vtree: &Vtree,
    formula: &CnfFormula,
    tables: UnifiedCostTables<'_>,
    load_stddev: f64,
    depth: u32,
) -> [f64; 11] {
    let clause_count: u64 = tables.clause_at.iter().map(|&load| u64::from(load)).sum();
    let (tight, excess, outside, tight_widths) = separator_terms(
        vtree,
        tables.ctx_in,
        tables.ctx_out,
        tables.cross,
        clause_count,
    );
    if tight == 0.0 {
        return [0.0; 11];
    }
    let child_boundaries =
        child_boundary_features(vtree, &tight_widths, tables.ctx_out, tables.sibling_overlap);
    let clause_load_cost = clause_load_cost(vtree, tables.clause_at);
    let leaves = f64::from(vtree.num_leaves());
    let chain = (1.0 + (5.0 * f64::from(depth) - leaves - 1.0).max(0.0)).log2();
    let high_load = (1.0 + load_stddev).log2() * (tight - 16.0).max(0.0);
    let shallow = 5 * u64::from(depth) <= u64::from(vtree.num_leaves()) + 1;
    let needs_matching = vtree.internal_bottomup().any(|(node, _, _)| {
        matching_activates(
            u64::from(tables.clause_at[node.idx()]),
            child_boundaries.tight_unique_sum[node.idx()],
            clause_count,
            shallow,
        )
        .is_some()
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
    [
        tight,
        excess / 2.0,
        9.0 * (1.0 + clause_load_cost).log2() / 5.0,
        high_load / 25.0,
        3.0 * chain / 40.0,
        -join / 2.0,
        directional_context / 2.0,
        8.0 * output_gap / 5.0,
        4.0 * extreme_chain,
        32.0 * extreme_join,
        successor_guard,
    ]
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
        let clause_at = clause_lca_counts(vtree, formula);
        let high_lca = clause_high_lca(vtree, formula);
        let ctx_in = context_width_from_high_lca(vtree, &high_lca, None);
        let outside = outside_context_tables(vtree, formula);
        let cross = vtree_crossing_clauses_per_node(vtree, formula);
        let peak_show = show_mask.map(|m| {
            context_width_from_high_lca(vtree, &high_lca, Some(m))
                .into_iter()
                .max()
                .unwrap_or(0)
        });
        Ok(Self::from_tables(
            vtree,
            formula,
            UnifiedCostTables {
                clause_at: &clause_at,
                ctx_in: &ctx_in,
                ctx_out: &outside.widths,
                sibling_overlap: &outside.sibling_overlap,
                cross: &cross,
            },
            peak_show,
        )
        .0)
    }

    /// Structural statistics and the cost addends, sharing the caller's tables.
    fn from_tables(
        vtree: &Vtree,
        formula: &CnfFormula,
        tables: UnifiedCostTables<'_>,
        peak_context_width_show: Option<u32>,
    ) -> (Self, [f64; 11]) {
        let clause_load_stddev = stddev_from_counts(tables.clause_at);
        let max_clause_load = max_from_counts(tables.clause_at);
        let peak_context_width_all = tables.ctx_in.iter().copied().max().unwrap_or(0);
        let terms = unified_cost_terms(
            vtree,
            formula,
            tables,
            clause_load_stddev,
            vtree_depth(vtree),
        );
        let cost = terms.iter().sum();
        (
            Self {
                clause_load_stddev,
                max_clause_load,
                peak_context_width_all,
                peak_context_width_show,
                cost,
            },
            terms,
        )
    }
}

#[cfg(test)]
mod tests;
