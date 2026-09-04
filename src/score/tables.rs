//! The per-node quantities a whole-tree score reads.
//!
//! At each internal vtree node these are the numbers a ranker can rank a
//! candidate by: the tables [`super`] already builds — the clause-LCA counts,
//! the inside and outside context widths, the crossing-clause counts, the
//! sibling overlap of the two children's outside sets — with the subtree
//! sizes, per-node depths and child combinations added here; how the clauses
//! bucketed AT a node split across its two children, which costs a maximum
//! matching per node; and the cut, the node's variables against the rest of
//! the formula through the primal graph, ending in a GF(2) elimination, which
//! is the expensive one. The second and third groups are computed only when
//! something reads a column from them.
//!
//! [`Tables::build`] takes the whole (vtree, formula) pair once and answers
//! [`Feature`] by node; [`FEATURE_NAMES`] is the naming a fitted model on disk
//! refers to them by, and a file naming a quantity this module has no
//! definition for is refused at load rather than scored against a zero.

use std::collections::{HashMap, HashSet};

use crate::cnf::CnfFormula;
use crate::vtree::{Vtree, VtreeIdx};

use super::{
    child_boundary_features, clause_high_lca, clause_lca_counts, context_width_from_high_lca,
    node_depths, outside_context_tables, sorted_bounds, subtree_tables,
    vtree_crossing_clauses_per_node,
};

// ---------------------------------------------------------------------------
// The quantities
// ---------------------------------------------------------------------------

/// One of the quantities the model reads at an internal node.
///
/// The names are the model file's, and a file naming one this list does not
/// have is refused at load rather than scored against a zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Feature {
    InsideWidth,
    OutsideWidth,
    TightWidth,
    CrossingClauses,
    LocalClauses,
    LeftInsideWidth,
    RightInsideWidth,
    LeftOutsideWidth,
    RightOutsideWidth,
    LeftTightWidth,
    RightTightWidth,
    LeftCrossingClauses,
    RightCrossingClauses,
    SubtreeLeaves,
    SubtreeClauses,
    LeftSubtreeLeaves,
    RightSubtreeLeaves,
    LeftSubtreeClauses,
    RightSubtreeClauses,
    Depth,
    SubtreeHeight,
    ChildTightSum,
    ChildTightProduct,
    ChildTightOverlap,
    ChildTightImbalance,
    ChildTightUniqueSum,
    ChildOutsideOverlap,
    ChildOutsideUnion,
    ChildOutsideSymmetricDifference,
    // The split group: how the clauses bucketed at a node divide across its two
    // children. Read out of [`CutTables`], one pass over those clauses.
    LocalJoinDensitySubtree,
    LocalJoinDensityTotal,
    SignedSplitDistinct,
    UnsignedSplitDistinct,
    SignedSplitEntropyBits,
    // The cut group: the node's variables against the rest of the formula,
    // through the primal graph. Also [`CutTables`], but a much costlier pass.
    CutRank,
    TwinIn,
    TwinOut,
    Below,
}

/// Every quantity under the name the model file uses for it.
pub(super) const FEATURE_NAMES: [(&str, Feature); 38] = [
    ("inside_width", Feature::InsideWidth),
    ("outside_width", Feature::OutsideWidth),
    ("tight_width", Feature::TightWidth),
    ("crossing_clauses", Feature::CrossingClauses),
    ("local_clauses", Feature::LocalClauses),
    ("left_inside_width", Feature::LeftInsideWidth),
    ("right_inside_width", Feature::RightInsideWidth),
    ("left_outside_width", Feature::LeftOutsideWidth),
    ("right_outside_width", Feature::RightOutsideWidth),
    ("left_tight_width", Feature::LeftTightWidth),
    ("right_tight_width", Feature::RightTightWidth),
    ("left_crossing_clauses", Feature::LeftCrossingClauses),
    ("right_crossing_clauses", Feature::RightCrossingClauses),
    ("subtree_leaves", Feature::SubtreeLeaves),
    ("subtree_clauses", Feature::SubtreeClauses),
    ("left_subtree_leaves", Feature::LeftSubtreeLeaves),
    ("right_subtree_leaves", Feature::RightSubtreeLeaves),
    ("left_subtree_clauses", Feature::LeftSubtreeClauses),
    ("right_subtree_clauses", Feature::RightSubtreeClauses),
    ("depth", Feature::Depth),
    ("subtree_height", Feature::SubtreeHeight),
    ("child_tight_sum", Feature::ChildTightSum),
    ("child_tight_product", Feature::ChildTightProduct),
    ("child_tight_overlap", Feature::ChildTightOverlap),
    ("child_tight_imbalance", Feature::ChildTightImbalance),
    ("child_tight_unique_sum", Feature::ChildTightUniqueSum),
    ("child_outside_overlap", Feature::ChildOutsideOverlap),
    ("child_outside_union", Feature::ChildOutsideUnion),
    (
        "child_outside_symmetric_difference",
        Feature::ChildOutsideSymmetricDifference,
    ),
    (
        "local_join_density_subtree",
        Feature::LocalJoinDensitySubtree,
    ),
    ("local_join_density_total", Feature::LocalJoinDensityTotal),
    ("signed_split_distinct", Feature::SignedSplitDistinct),
    ("unsigned_split_distinct", Feature::UnsignedSplitDistinct),
    ("signed_split_entropy_bits", Feature::SignedSplitEntropyBits),
    ("cutrank", Feature::CutRank),
    ("twin_in", Feature::TwinIn),
    ("twin_out", Feature::TwinOut),
    ("below", Feature::Below),
];

impl Feature {
    pub(super) fn from_name(name: &str) -> Option<Feature> {
        FEATURE_NAMES
            .iter()
            .find(|(known, _)| *known == name)
            .map(|&(_, feature)| feature)
    }

    /// Whether this quantity comes from the split pass: one walk over the
    /// clauses bucketed at each node, plus a matching per node.
    pub(super) fn is_from_split(self) -> bool {
        matches!(
            self,
            Feature::LocalJoinDensitySubtree
                | Feature::LocalJoinDensityTotal
                | Feature::SignedSplitDistinct
                | Feature::UnsignedSplitDistinct
                | Feature::SignedSplitEntropyBits
        )
    }

    /// Whether this quantity comes from the cut pass, which is the expensive
    /// one: the primal-graph boundary at every node, and a GF(2) elimination
    /// over its distinct rows.
    pub(super) fn is_from_cut(self) -> bool {
        matches!(
            self,
            Feature::CutRank | Feature::TwinIn | Feature::TwinOut | Feature::Below
        )
    }
}

/// The split and cut quantities, per node.
///
/// The split half describes the clauses bucketed at one node — the ones whose
/// variables first meet there — and how they divide across its two children. A
/// node no clause is bucketed at keeps the zero it starts with, which is what
/// the tool that produced the fitting data records for it.
///
/// The cut half describes the node's variables against the rest of the
/// formula, through the primal graph: how many distinct boundary neighbourhoods
/// each side sees, and the GF(2) rank of the inside-by-outside adjacency. The
/// root has no cut — every variable is inside it — and the tool skips it, so
/// the four cut columns stay zero there.
struct CutTables {
    /// `matching * load` over the clauses bucketed in this subtree.
    density_subtree: Vec<f64>,
    /// `matching * load` over the formula's non-empty clauses.
    density_total: Vec<f64>,
    /// Distinct (left literals, right literals) splits among the node's clauses.
    signed_distinct: Vec<u32>,
    /// The same counting variables rather than literals.
    unsigned_distinct: Vec<u32>,
    /// Entropy of the signed-split distribution, in bits.
    signed_entropy_bits: Vec<f64>,
    /// GF(2) rank of the inside-by-outside primal adjacency, at the cap.
    cutrank: Vec<u32>,
    /// Distinct outside neighbourhoods of the variables inside.
    twin_in: Vec<u32>,
    /// Distinct inside neighbourhoods of the variables outside.
    twin_out: Vec<u32>,
    /// Variables below the node; zero at the root, which has no cut.
    below: Vec<u32>,
    /// Which nodes the cut pass produced a row for. False at the root, and
    /// false everywhere when no column asked for the pass. The four cut
    /// columns carry zeros at the root; an aggregate over the tree leaves that
    /// node out, because the table it is checked against has no row for it.
    has_cut: Vec<bool>,
}

/// Where a rank stops being counted. The fit read the same cap, so a node over
/// it carries the cap as its value in the training data too.
const CUTRANK_CAP: usize = 4096;

impl CutTables {
    fn build(
        vtree: &Vtree,
        formula: &CnfFormula,
        subtree_clauses: &[u64],
        subtree_leaves: &[u32],
        clause_at: &[u32],
        split: bool,
        cut: bool,
    ) -> CutTables {
        let nodes = vtree.num_nodes();
        let zeros = || vec![0u32; nodes];
        let reals = || vec![0f64; nodes];
        let mut tables = CutTables {
            density_subtree: reals(),
            density_total: reals(),
            signed_distinct: zeros(),
            unsigned_distinct: zeros(),
            signed_entropy_bits: reals(),
            cutrank: zeros(),
            twin_in: zeros(),
            twin_out: zeros(),
            below: zeros(),
            has_cut: vec![false; nodes],
        };
        let (entry, exit) = super::subtree_intervals(vtree);
        if split {
            tables.fill_split(vtree, formula, subtree_clauses, clause_at, &entry, &exit);
        }
        if cut {
            tables.fill_cut(vtree, formula, subtree_leaves, &entry, &exit);
        }
        tables
    }

    /// The clauses bucketed at each node, split across its two children.
    fn fill_split(
        &mut self,
        vtree: &Vtree,
        formula: &CnfFormula,
        subtree_clauses: &[u64],
        clause_at: &[u32],
        entry: &[u32],
        exit: &[u32],
    ) {
        let total_clauses: u64 = clause_at.iter().map(|&load| u64::from(load)).sum();
        let clauses_at = super::clause_lca_members(vtree, formula);
        // One allocation for the whole tree: a node's tables are read off these
        // and they are cleared before the next node fills them.
        let mut signed: HashMap<(Vec<i32>, Vec<i32>), u32> = HashMap::new();
        let mut unsigned: HashSet<(Vec<u32>, Vec<u32>)> = HashSet::new();
        for (node, left, _right) in vtree.internal_bottomup() {
            let t = node.idx();
            let clause_ids = &clauses_at[t];
            if clause_ids.is_empty() {
                continue;
            }
            let load = clause_ids.len() as u64;
            signed.clear();
            unsigned.clear();
            let mut left_adjacency: Vec<Vec<usize>> = Vec::with_capacity(clause_ids.len());
            let mut right_adjacency: Vec<Vec<usize>> = Vec::with_capacity(clause_ids.len());
            for &clause_idx in clause_ids {
                let mut left_literals = Vec::new();
                let mut right_literals = Vec::new();
                for lit in &formula.clauses[clause_idx].literals {
                    let leaf = vtree.leaf_of(lit.var).idx();
                    let inside = entry[left.idx()] <= entry[leaf] && entry[leaf] < exit[left.idx()];
                    if inside {
                        left_literals.push(lit.to_dimacs());
                    } else {
                        right_literals.push(lit.to_dimacs());
                    }
                }
                // A clause is read as the SET of its literals, so a repeated one
                // counts once, whether or not the formula reached here through a
                // parser that had already dropped it.
                for side in [&mut left_literals, &mut right_literals] {
                    side.sort_unstable();
                    side.dedup();
                }
                let variables = |literals: &[i32]| {
                    let mut vars: Vec<u32> = literals.iter().map(|l| l.unsigned_abs()).collect();
                    vars.sort_unstable();
                    vars.dedup();
                    vars
                };
                let left_vars = variables(&left_literals);
                let right_vars = variables(&right_literals);
                unsigned.insert((left_vars.clone(), right_vars.clone()));
                *signed
                    .entry((left_literals, right_literals))
                    .or_insert(0u32) += 1;
                left_adjacency.push(left_vars.into_iter().map(|v| v as usize).collect());
                right_adjacency.push(right_vars.into_iter().map(|v| v as usize).collect());
            }
            // Summed smallest first, over a sorted list rather than the map's own
            // order, so the same node scores the same number in every process.
            let mut counts: Vec<u32> = signed.values().copied().collect();
            counts.sort_unstable();
            let entropy: f64 = counts
                .iter()
                .map(|&count| {
                    let share = f64::from(count) / load as f64;
                    -share * share.log2()
                })
                .sum();

            let matched = super::maximum_matching_size(&left_adjacency)
                .min(super::maximum_matching_size(&right_adjacency));
            let matched_f = f64::from(matched);
            self.density_subtree[t] = matched_f * load as f64 / subtree_clauses[t] as f64;
            self.density_total[t] = matched_f * load as f64 / total_clauses.max(1) as f64;
            self.signed_distinct[t] = signed.len() as u32;
            self.unsigned_distinct[t] = unsigned.len() as u32;
            self.signed_entropy_bits[t] = entropy;
        }
    }

    /// The primal-graph boundary at each node.
    ///
    /// A variable's neighbours are the variables it shares a clause with. At a
    /// node, each variable inside contributes the set of its neighbours that are
    /// outside; the distinct such sets are the rows, and their GF(2) rank is the
    /// cut rank. The same counted from the other side gives `twin_out`.
    ///
    /// The root has no outside, so it has no cut. The fit saw no row for it and
    /// read zeros there, and these tables leave zeros there too.
    fn fill_cut(
        &mut self,
        vtree: &Vtree,
        formula: &CnfFormula,
        subtree_leaves: &[u32],
        entry: &[u32],
        exit: &[u32],
    ) {
        // Variable ids are the bit positions, over the declared space, so a
        // declared variable no clause names still occupies its own place.
        let declared = formula.num_vars as usize;
        let space = declared.max(vtree.num_vars() as usize);
        if space == 0 {
            return;
        }
        // Every variable at its leaf's place in the tree order, so "inside this
        // node" is one interval test rather than a set lookup. A declared
        // variable the tree has no leaf for stays outside every node.
        let mut place = vec![u32::MAX; space];
        for (leaf, var) in vtree.leaf_bottomup() {
            place[var.idx()] = entry[leaf.idx()];
        }
        // The primal graph as sorted neighbour lists. Sorted, so a restricted
        // neighbourhood comes out sorted too and two equal ones compare equal.
        let mut adjacency: Vec<Vec<u32>> = vec![Vec::new(); space];
        let mut clause_vars: Vec<u32> = Vec::new();
        for clause in &formula.clauses {
            clause_vars.clear();
            clause_vars.extend(clause.literals.iter().map(|lit| lit.var.0));
            clause_vars.sort_unstable();
            clause_vars.dedup();
            for &x in &clause_vars {
                for &y in &clause_vars {
                    if x != y {
                        adjacency[x as usize].push(y);
                    }
                }
            }
        }
        for neighbours in adjacency.iter_mut() {
            neighbours.sort_unstable();
            neighbours.dedup();
        }

        let mut rows: HashSet<Vec<u32>> = HashSet::new();
        let mut columns: HashSet<Vec<u32>> = HashSet::new();
        let mut restricted: Vec<u32> = Vec::new();
        let mut scratch = RankScratch::default();
        for (node, _left, _right) in vtree.internal_bottomup() {
            let t = node.idx();
            // `entry`/`exit` number every node of the subtree, not its leaves,
            // so the variables below come from the leaf count.
            let (lo, hi) = (entry[t], exit[t]);
            if subtree_leaves[t] as usize == declared {
                continue;
            }
            let inside = |v: u32| {
                let at = place[v as usize];
                lo <= at && at < hi
            };
            rows.clear();
            columns.clear();
            for v in 0..space as u32 {
                let neighbours = &adjacency[v as usize];
                if neighbours.is_empty() {
                    continue;
                }
                let here = inside(v);
                restricted.clear();
                restricted.extend(neighbours.iter().copied().filter(|&n| inside(n) != here));
                if restricted.is_empty() {
                    continue;
                }
                let side = if here { &mut rows } else { &mut columns };
                if !side.contains(&restricted) {
                    side.insert(restricted.clone());
                }
            }
            self.below[t] = subtree_leaves[t];
            self.twin_in[t] = rows.len() as u32;
            self.twin_out[t] = columns.len() as u32;
            self.cutrank[t] = scratch.rank(rows.iter(), space);
            self.has_cut[t] = true;
        }
    }
}

/// The working space the rank pass reuses from node to node: one basis vector
/// per pivot bit, and the index that finds the vector holding a given pivot.
#[derive(Default)]
struct RankScratch {
    /// For each bit position, which basis vector has it as its leading bit, or
    /// `u32::MAX` for none.
    at: Vec<u32>,
    /// The basis vectors themselves, kept allocated across nodes.
    basis: Vec<Vec<u64>>,
    /// The bit positions currently set in `at`, so a reset touches only those.
    used: Vec<usize>,
    /// The row being reduced.
    row: Vec<u64>,
}

impl RankScratch {
    /// The GF(2) rank of `rows`, counted to [`CUTRANK_CAP`] and no further.
    ///
    /// A row is a set of variables, which becomes a bit vector over the whole
    /// variable space. Each row is reduced against the basis by its leading bit
    /// until it either lands on a bit no basis vector leads with, and joins the
    /// basis, or reaches zero, and does not.
    fn rank<'a, I>(&mut self, rows: I, space: usize) -> u32
    where
        I: Iterator<Item = &'a Vec<u32>>,
    {
        let RankScratch {
            at,
            basis,
            used,
            row,
        } = self;
        let words = space.div_ceil(64);
        for &bit in used.iter() {
            at[bit] = u32::MAX;
        }
        used.clear();
        if at.len() < space {
            at.resize(space, u32::MAX);
        }
        let mut rank = 0usize;
        for members in rows {
            row.clear();
            row.resize(words, 0);
            for &v in members {
                row[v as usize / 64] |= 1 << (v as usize % 64);
            }
            while let Some(pivot) = leading_bit(row) {
                let slot = at[pivot];
                if slot == u32::MAX {
                    at[pivot] = rank as u32;
                    used.push(pivot);
                    match basis.get_mut(rank) {
                        Some(vector) => {
                            vector.clear();
                            vector.extend_from_slice(row);
                        }
                        None => basis.push(row.clone()),
                    }
                    rank += 1;
                    break;
                }
                for (word, other) in row.iter_mut().zip(&basis[slot as usize]) {
                    *word ^= other;
                }
            }
            if rank >= CUTRANK_CAP {
                return CUTRANK_CAP as u32;
            }
        }
        rank as u32
    }
}

/// The highest set bit of `row`, or `None` if it is zero.
fn leading_bit(row: &[u64]) -> Option<usize> {
    row.iter()
        .rposition(|&word| word != 0)
        .map(|index| index * 64 + (63 - row[index].leading_zeros() as usize))
}

/// The per-node tables every quantity is read out of, built once per
/// (vtree, formula) pair.
pub(super) struct Tables {
    /// Inside context width: variables below a node that a clause crossing it
    /// also names.
    ctx_in: Vec<u32>,
    /// Outside context width: variables above a node sharing a clause with one
    /// below it.
    ctx_out: Vec<u32>,
    /// Clauses with a variable below the node and an LCA strictly above it.
    cross: Vec<u32>,
    /// Smallest of the three bounds above, with a leaf capped at one.
    tight: Vec<u32>,
    /// Clauses whose LCA is the node.
    clause_at: Vec<u32>,
    /// Clauses bucketed anywhere below the node, its own load included.
    subtree_clauses: Vec<u64>,
    subtree_leaves: Vec<u32>,
    subtree_height: Vec<u32>,
    depth: Vec<u32>,
    /// Variables in both children's outside sets.
    sibling_overlap: Vec<u32>,
    /// `tight(left) + tight(right)` less the part of the overlap both can
    /// account for.
    tight_unique_sum: Vec<u32>,
    /// The split and cut quantities, `None` when no column the model reads
    /// comes from either group.
    cut: Option<CutTables>,
}

impl Tables {
    pub(super) fn build(vtree: &Vtree, formula: &CnfFormula, split: bool, cut: bool) -> Tables {
        let clause_at = clause_lca_counts(vtree, formula);
        let high_lca = clause_high_lca(vtree, formula);
        let ctx_in = context_width_from_high_lca(vtree, &high_lca, None);
        let outside = outside_context_tables(vtree, formula);
        let cross = vtree_crossing_clauses_per_node(vtree, formula);
        let tight: Vec<u32> = (0..vtree.num_nodes())
            .map(|i| {
                let idx = VtreeIdx(i as u32);
                sorted_bounds(
                    ctx_in[i],
                    outside.widths[i],
                    cross[i],
                    vtree.node(idx).is_leaf(),
                )[0]
            })
            .collect();
        let subtree = subtree_tables(vtree, &clause_at);
        let boundaries =
            child_boundary_features(vtree, &tight, &outside.widths, &outside.sibling_overlap);
        let cut = (split || cut).then(|| {
            CutTables::build(
                vtree,
                formula,
                &subtree.clauses,
                &subtree.leaves,
                &clause_at,
                split,
                cut,
            )
        });
        Tables {
            ctx_in,
            ctx_out: outside.widths,
            cross,
            tight,
            clause_at,
            subtree_clauses: subtree.clauses,
            subtree_leaves: subtree.leaves,
            subtree_height: subtree.height,
            depth: node_depths(vtree),
            sibling_overlap: outside.sibling_overlap,
            tight_unique_sum: boundaries.tight_unique_sum,
            cut,
        }
    }

    /// The split and cut tables, which are built whenever a column reads one.
    fn cut(&self) -> &CutTables {
        self.cut
            .as_ref()
            .expect("the split and cut tables are built when a model reads one of their columns")
    }

    /// Whether the cut pass produced a row for `node`. The four cut columns
    /// have no value at a node it skipped — the root — as against a value of
    /// zero; see [`CutTables::has_cut`].
    pub(super) fn has_cut_row(&self, node: VtreeIdx) -> bool {
        self.cut.as_ref().is_some_and(|cut| cut.has_cut[node.idx()])
    }

    /// The five tables the structural cost is a reduction of, borrowed out of
    /// the ones already built here, so a caller wanting both the per-node
    /// quantities and the cost's own terms pays for one pass rather than two.
    pub(super) fn cost_tables(&self) -> super::UnifiedCostTables<'_> {
        super::UnifiedCostTables {
            clause_at: &self.clause_at,
            ctx_in: &self.ctx_in,
            ctx_out: &self.ctx_out,
            sibling_overlap: &self.sibling_overlap,
            cross: &self.cross,
        }
    }

    /// The per-node clause-LCA counts, which the load spread is measured from.
    pub(super) fn clause_at(&self) -> &[u32] {
        &self.clause_at
    }

    /// One quantity at one internal node.
    ///
    /// `child_tight_sum` is zero at a node no clause is bucketed at, which is
    /// how the tool that produced the fitting data defines it: it comes from a
    /// pass that only visits nodes carrying a clause. It is therefore NOT the
    /// same as `child_tight_overlap + child_tight_unique_sum`, and the model
    /// reads both.
    pub(super) fn value(
        &self,
        feature: Feature,
        node: VtreeIdx,
        left: VtreeIdx,
        right: VtreeIdx,
    ) -> f64 {
        let t = node.idx();
        let l = left.idx();
        let r = right.idx();
        let overlap = self.sibling_overlap[t];
        match feature {
            Feature::InsideWidth => f64::from(self.ctx_in[t]),
            Feature::OutsideWidth => f64::from(self.ctx_out[t]),
            Feature::TightWidth => f64::from(self.tight[t]),
            Feature::CrossingClauses => f64::from(self.cross[t]),
            Feature::LocalClauses => f64::from(self.clause_at[t]),
            Feature::LeftInsideWidth => f64::from(self.ctx_in[l]),
            Feature::RightInsideWidth => f64::from(self.ctx_in[r]),
            Feature::LeftOutsideWidth => f64::from(self.ctx_out[l]),
            Feature::RightOutsideWidth => f64::from(self.ctx_out[r]),
            Feature::LeftTightWidth => f64::from(self.tight[l]),
            Feature::RightTightWidth => f64::from(self.tight[r]),
            Feature::LeftCrossingClauses => f64::from(self.cross[l]),
            Feature::RightCrossingClauses => f64::from(self.cross[r]),
            Feature::SubtreeLeaves => f64::from(self.subtree_leaves[t]),
            Feature::SubtreeClauses => self.subtree_clauses[t] as f64,
            Feature::LeftSubtreeLeaves => f64::from(self.subtree_leaves[l]),
            Feature::RightSubtreeLeaves => f64::from(self.subtree_leaves[r]),
            Feature::LeftSubtreeClauses => self.subtree_clauses[l] as f64,
            Feature::RightSubtreeClauses => self.subtree_clauses[r] as f64,
            Feature::Depth => f64::from(self.depth[t]),
            Feature::SubtreeHeight => f64::from(self.subtree_height[t]),
            Feature::ChildTightSum => {
                if self.clause_at[t] == 0 {
                    0.0
                } else {
                    f64::from(self.tight[l]) + f64::from(self.tight[r])
                }
            }
            Feature::ChildTightProduct => f64::from(self.tight[l]) * f64::from(self.tight[r]),
            Feature::ChildTightOverlap => {
                f64::from(self.tight[l] + self.tight[r] - self.tight_unique_sum[t])
            }
            Feature::ChildTightImbalance => f64::from(self.tight[l].abs_diff(self.tight[r])),
            Feature::ChildTightUniqueSum => f64::from(self.tight_unique_sum[t]),
            Feature::ChildOutsideOverlap => f64::from(overlap),
            Feature::ChildOutsideUnion => f64::from(self.ctx_out[l] + self.ctx_out[r] - overlap),
            Feature::ChildOutsideSymmetricDifference => {
                f64::from(self.ctx_out[l] + self.ctx_out[r] - 2 * overlap)
            }
            Feature::LocalJoinDensitySubtree => self.cut().density_subtree[t],
            Feature::LocalJoinDensityTotal => self.cut().density_total[t],
            Feature::SignedSplitDistinct => f64::from(self.cut().signed_distinct[t]),
            Feature::UnsignedSplitDistinct => f64::from(self.cut().unsigned_distinct[t]),
            Feature::SignedSplitEntropyBits => self.cut().signed_entropy_bits[t],
            Feature::CutRank => f64::from(self.cut().cutrank[t]),
            Feature::TwinIn => f64::from(self.cut().twin_in[t]),
            Feature::TwinOut => f64::from(self.cut().twin_out[t]),
            Feature::Below => f64::from(self.cut().below[t]),
        }
    }
}

#[cfg(test)]
mod tests;
