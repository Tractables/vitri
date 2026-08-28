//! Structural scores for a (vtree, formula) pair — what candidate selection
//! ranks on, read off the tree's shape without compiling anything.
//!
//! Two tables underlie all of them: the clause-LCA counts (each clause bucketed
//! at the single node where its variables first meet) and, per variable, the
//! shallowest clause-LCA it appears under, which fixes the segment of the tree
//! that variable crosses. Clause load, its spread, peak context width and the
//! composite [`vtree_cost`] are reductions of those two; [`VtreeScores`] fuses
//! the five the portfolio reads over one shared scan of each.
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
use crate::vtree::{VarId, Vtree, VtreeIdx};

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
fn for_each_clause_lca(vtree: &Vtree, formula: &CnfFormula, mut f: impl FnMut(VtreeIdx)) {
    for clause in &formula.clauses {
        if clause.literals.is_empty() {
            continue;
        }
        let lca = clause
            .literals
            .iter()
            .map(|lit| vtree.leaf_of(lit.var))
            .reduce(|a, b| vtree.lca(a, b))
            .unwrap();
        f(lca);
    }
}

/// For each non-empty clause, increment the count at the vtree node where the
/// clause's variables meet. Returns a vector of length `vtree.num_nodes()` with
/// clause counts per node.
fn clause_lca_counts(vtree: &Vtree, formula: &CnfFormula) -> Vec<u32> {
    let mut clause_at = vec![0u32; vtree.num_nodes()];
    for_each_clause_lca(vtree, formula, |lca| clause_at[lca.idx()] += 1);
    clause_at
}

/// The clause-LCA counts, together with the node each non-empty clause landed
/// on, in the order [`for_each_clause_lca`] reports them.
///
/// For a caller that has to go back from an overloaded node to the clauses
/// sitting on it, which the counts alone cannot answer.
pub(crate) fn clause_lca_nodes(vtree: &Vtree, formula: &CnfFormula) -> (Vec<VtreeIdx>, Vec<u32>) {
    let mut per_clause = Vec::with_capacity(formula.clauses.len());
    let mut clause_at = vec![0u32; vtree.num_nodes()];
    for_each_clause_lca(vtree, formula, |lca| {
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

/// Composite vtree cost score: lower is better.
///
/// Combines three cost signals:
/// - `max_load³`: the maximum number of clauses assigned to any single vtree node
///   (the LCA of their variables).
/// - `sum_of_products`: Σ subtree_clauses(left) × subtree_clauses(right) over all
///   internal nodes.
/// - `scope_cost`: Σ clause_at(t) × log₂(leaves_below(t)).
///
/// Public: a caller comparing its own vtree against one this crate produced
/// scores both through this one entry rather than reimplementing the metric.
///
/// # Errors
///
/// [`VitriError::Mismatch`] if `formula` names a variable `vtree` has no leaf
/// for, which is the one way the two arguments can fail to be about the same
/// formula.
pub fn vtree_cost(vtree: &Vtree, formula: &CnfFormula) -> Result<u64, VitriError> {
    covered_by(vtree, formula)?;
    Ok(cost_from_counts(vtree, &clause_lca_counts(vtree, formula)))
}

/// Core of [`vtree_cost`] over a precomputed clause-LCA count table. Split
/// out so `VtreeScores::compute` can share the one `clause_lca_counts` scan
/// across stddev / max-load / cost.
fn cost_from_counts(vtree: &Vtree, clause_at: &[u32]) -> u64 {
    let nn = vtree.num_nodes();

    let mut subtree_clauses = vec![0u32; nn];
    let mut subtree_leaves = vec![0u32; nn];
    let mut max_load: u32 = 0;
    for (t, _var) in vtree.leaf_bottomup() {
        subtree_clauses[t.idx()] = clause_at[t.idx()];
        subtree_leaves[t.idx()] = 1;
        max_load = max_load.max(clause_at[t.idx()]);
    }

    let mut sum_of_products: u64 = 0;
    let mut scope_cost: u64 = 0;
    for (t, left, right) in vtree.internal_bottomup() {
        subtree_clauses[t.idx()] =
            clause_at[t.idx()] + subtree_clauses[left.idx()] + subtree_clauses[right.idx()];
        subtree_leaves[t.idx()] = subtree_leaves[left.idx()] + subtree_leaves[right.idx()];
        max_load = max_load.max(clause_at[t.idx()]);

        let product = subtree_clauses[left.idx()] as u64 * subtree_clauses[right.idx()] as u64;
        sum_of_products = sum_of_products.saturating_add(product);

        let leaves = subtree_leaves[t.idx()];
        // ⌊log₂(leaves)⌋: position of the highest set bit in a u32.
        let log_leaves = if leaves <= 1 {
            0u32
        } else {
            31 - leaves.leading_zeros()
        };
        scope_cost += clause_at[t.idx()] as u64 * log_leaves as u64;
    }

    let ml = max_load as u64;
    // Saturate: the cubic term wraps u64 once max_load exceeds ~2.64M clauses on
    // one LCA node, and release builds have no overflow-checks — a wrapped score
    // would silently misrank candidate vtrees (worst case: pick a slower vtree →
    // timeout). Never affects the count (any vtree yields the same exact count);
    // saturating keeps the ranking monotone at the top end.
    ml.saturating_mul(ml)
        .saturating_mul(ml)
        .saturating_add(sum_of_products)
        .saturating_add(scope_cost)
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
/// whose LCA is a strict ancestor of `t`) — the separator / cut size at `t`.
/// `2^ctx[t]` is a theoretical *upper bound* on how many distinct
/// sub-functions can be materialised at `t` during compilation. Unlike the
/// `clause_load_*` metrics (which only count clauses bucketed at their LCA),
/// this measures how many variables leak across each split. NB: the bound is
/// loose — `max_ctx` predicts the realized cost well on some instances and
/// poorly on others, so it is not a standalone selection signal.
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
        let lca = clause
            .literals
            .iter()
            .map(|lit| vtree.leaf_of(lit.var))
            .reduce(|a, b| vtree.lca(a, b))
            .unwrap();
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

/// All five structural selection metrics for one realized vtree, computed ONCE
/// at realization (each pure `(vtree, formula)` free function called exactly
/// once). Every candidate scoring site reads these cached fields instead of
/// recomputing a metric ad-hoc. All five fields are computed unconditionally:
/// the traversals are part of the fixed-cost vtree-construction phase
/// (negligible vs the FlowCutter/goatd/bisection builds), and making any field
/// lazy would reintroduce the recompute-scatter this removes.
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
/// | [`peak_context_width_all`](Self::peak_context_width_all) | the widest CUT: the largest separator over all nodes — variables in a subtree that also occur in a clause crossing out of it. `2^peak_context_width_all` bounds the largest intermediate diagram. A loose bound, hence not a standalone signal. | variables |
/// | [`peak_context_width_show`](Self::peak_context_width_show) | the same peak counting only SHOW (projected-kept) variables, or `None` for a non-projected instance. | variables |
/// | [`cost`](Self::cost) | a composite realized-size proxy: `max_clause_load³ + Σ (left subtree clauses × right subtree clauses) + Σ clauses_at(t)·log₂(leaves below t)`. Saturating, so it stays monotone at the top end. | dimensionless |
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
    /// Composite realized-size proxy. Lower is better.
    pub cost: u64,
}

impl VtreeScores {
    /// Compute all five fields for `vtree` against `formula` in one pass over
    /// the shared clause-LCA tables. `show_mask` is `Some` only for projected
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
        // Build the two shared tables ONCE, then derive every field from them —
        // the five free functions internally re-derive `clause_lca_counts` 3× and
        // `clause_high_lca` 2×; fusing here does each scan a single time.
        let clause_at = clause_lca_counts(vtree, formula);
        let high_lca = clause_high_lca(vtree, formula);
        Ok(Self {
            clause_load_stddev: stddev_from_counts(&clause_at),
            max_clause_load: max_from_counts(&clause_at),
            peak_context_width_all: context_width_from_high_lca(vtree, &high_lca, None)
                .into_iter()
                .max()
                .unwrap_or(0),
            peak_context_width_show: show_mask.map(|m| {
                context_width_from_high_lca(vtree, &high_lca, Some(m))
                    .into_iter()
                    .max()
                    .unwrap_or(0)
            }),
            cost: cost_from_counts(vtree, &clause_at),
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
