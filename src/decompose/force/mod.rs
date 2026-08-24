//! Force-directed embedding of the variables, tree-ified into a vtree.
//!
//! Generalizes classic 1D FORCE (Aloul, Markov and Sakallah, "FORCE: a fast
//! and easy-to-implement variable-ordering heuristic", GLSVLSI 2003) to `d`
//! dimensions (`d ∈ {2,3,4}`, default 2), then turns the point cloud into a
//! binary vtree via a Euclidean-MST single-linkage hierarchy (`mst`) or a
//! recursive principal-axis median cut (`cut`).
//!
//! Fully deterministic: seeded from the fixed [`SEED`], no threads or wall
//! clock — one formula and one configuration always produce the same vtree.

use std::sync::Arc;

use rustc_hash::FxHashMap;

use super::EMPTY_FORMULA;
use super::best::BestBy;
use crate::cnf::CnfFormula;
use crate::score::{clause_lca_nodes, load_stats};
use crate::vtree::{VarId, Vtree, VtreeArena, VtreeIdx};

mod geometry;
mod layout;
mod tree;
pub(crate) use geometry::*;
use layout::*;
use tree::*;

/// Base RNG seed for the deterministic layout. Restart `i` of the `seeds` axis uses
/// `SEED + i`, so `seeds = 1` reruns exactly this seed.
const SEED: u64 = 42;

/// Fixed 1D-FORCE pre-pass round budget for `init = force1d`. A handful of rounds
/// converge the ranking on typical instances; the loop early-exits when the rank
/// order stops changing. Fixed rather than time-based, for determinism.
const FORCE1D_ROUNDS: usize = 30;

/// Zero-variance / degenerate-axis guard.
const EPS: f64 = 1e-9;

/// Prim is exact and O(n) memory; above this the layout switches to the
/// grid-bucketed k-NN candidate graph so MST construction stays sub-quadratic.
pub(super) const PRIM_LIMIT: usize = 20_000;

/// k-nearest candidates gathered per point in the grid-kNN MST path.
const KNN_K: usize = 8;

/// Clause-size cap for co-occurrence pair enumeration (`w=co`): clauses wider than
/// this are skipped for pair enumeration — near-uninformative and quadratic.
///
/// This weights MST candidate edges over the layout's own incidence lists. The
/// co-occurrence GRAPH the tree-decomposition constructions read is a separate
/// object with a separate cap, `COOC_CLAUSE_LEN_CAP` (`decompose::td_parse`),
/// which decides which pairs that graph has at all. Nothing in the tree records
/// why the two values differ.
const CO_CLAUSE_CAP: usize = 64;

/// Maximum embedding dimension (`d` axis). Kept small so the d×d Jacobi
/// eigensolver stays a handful of flops; 8×8 is still tiny for cyclic Jacobi and
/// converges inside [`JACOBI_SWEEPS`] with a wide margin.
///
/// The ceiling: the `dim=` spec grammar reads it from here, so the range a vtree
/// spec accepts and the range the layout supports cannot come apart.
pub(crate) const MAX_DIM: usize = 8;

/// Cyclic-Jacobi sweep cap for the `d > 2` symmetric eigensolver. For `d ≤ 4` a
/// handful of sweeps drive the off-diagonal to machine zero; the loop also
/// early-exits once the off-diagonal mass is negligible. Fixed, not time-based, so
/// the layout stays deterministic.
const JACOBI_SWEEPS: usize = 30;

/// Which tree-ifier turns the layout into a vtree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ForceMode {
    /// Euclidean MST → single-linkage merge hierarchy (top-down longest-edge split).
    Mst,
    /// Recursive principal-axis median cut.
    Cut,
}

/// How the MST is rooted into a vtree (MST mode only). All three split the SAME
/// spanning tree top-down; only the edge-picking rule differs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RootRule {
    /// Split at the longest edge, reproducing the single-linkage merge hierarchy.
    Merge,
    /// Split at the edge minimizing `max(|A|, |B|)` (tie → longer edge, then smaller
    /// endpoint variable).
    Balance,
    /// Among edges at least half the component's longest, minimize `max(|A|, |B|)`
    /// (same tie rule).
    Hybrid,
}

/// Left/right rule at each internal node (MST mode only).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum OrientRule {
    /// Smaller-centroid-x subtree LEFT (tie → smaller minimum variable index).
    X,
    /// Smaller-variable-count subtree LEFT (same tie rule).
    Small,
    /// Larger-variable-count subtree LEFT (same tie rule).
    Big,
}

/// MST edge-weight rule (MST mode only).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WeightRule {
    Euclid,
    /// Co-occurrence-discounted: `euclid(u, v) / (1 + #clauses containing both)`.
    Co,
}

/// Clause weighting in the layout iteration (both tree-ifiers).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ClauseWeight {
    /// Unweighted mean of clause centres of gravity.
    Uniform,
    /// Each variable's update weights clause `c` by `1/max(1, |c| − 1)`, so short
    /// clauses pull harder — the standard FORCE refinement.
    Short,
}

/// Layout initialization rule (`init` axis, both tree-ifiers).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum InitMode {
    /// Every dimension seeded uniform random.
    Rand,
    /// Dimension 0 seeded from a 1D FORCE pre-pass rank, scaled to unit variance;
    /// dimensions 1.. keep the seeded random values.
    Force1d,
}

/// A complete `force` configuration. `root`, `orient` and `weight` are
/// MST-mode-only; `clause_weight`, `dim`, `fb`, `seeds` and `init` apply to the
/// shared layout. [`ForceConfig::new`] gives the defaults (`merge` / `x` /
/// `euclid` / `uniform`, `dim = 2`, `fb = 0`, `seeds = 1`, `init = rand`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ForceConfig {
    /// Which tree-ifier turns the point cloud into a vtree.
    pub mode: ForceMode,
    /// Which MST edge each internal node splits at.
    pub root: RootRule,
    /// Which side of a split becomes the left child.
    pub orient: OrientRule,
    /// How MST edge lengths are measured.
    pub weight: WeightRule,
    /// How clauses are weighted in the layout iteration.
    pub clause_weight: ClauseWeight,
    /// Embedding dimension (`d` axis): 2 (default), 3, or 4. Higher dimensions add
    /// spectral axes through the Jacobi solver.
    pub dim: usize,
    /// Metric-feedback reweighting rounds (`fb` axis): 0 (default) = no feedback.
    pub fb: u8,
    /// Random-restart count (`seeds` axis): 1 (default) = a single build from the
    /// base [`SEED`]. `k > 1` reruns the whole pipeline from `SEED + i` and keeps
    /// the vtree with the lowest max clause-LCA load. Valid for both tree-ifiers.
    pub seeds: u8,
    /// Layout initialization (`init` axis).
    pub init: InitMode,
}

impl ForceConfig {
    /// Default configuration for `mode` (every other axis at its default value).
    pub(crate) fn new(mode: ForceMode) -> Self {
        ForceConfig {
            mode,
            root: RootRule::Merge,
            orient: OrientRule::X,
            weight: WeightRule::Euclid,
            clause_weight: ClauseWeight::Uniform,
            dim: 2,
            fb: 0,
            seeds: 1,
            init: InitMode::Rand,
        }
    }
}

// ---------------------------------------------------------------------------
// Clause-LCA load — the metric-feedback objective
// ---------------------------------------------------------------------------

/// Max clause-LCA load over INTERNAL vtree nodes, or 0 if none carries a clause.
/// Unit clauses land on leaves and are excluded: the feedback objective is the
/// internal-node bottleneck.
pub(super) fn max_internal_load(vtree: &Vtree, loads: &[u32]) -> u32 {
    let mut m = 0;
    for (idx, &load) in loads.iter().enumerate() {
        if load > 0 && !vtree.node(VtreeIdx(idx as u32)).is_leaf() {
            m = m.max(load);
        }
    }
    m
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Build a vtree from a `d`-dimensional FORCE embedding of the formula's variables.
/// `cfg` selects the tree-ifier, its per-axis rules, the embedding dimension and the
/// metric-feedback rounds; [`ForceConfig::new`] gives the defaults. Deterministic
/// (fixed [`SEED`]); the only error is an empty formula.
///
/// Metric feedback (`fb > 0`): after building the vtree, clauses whose LCA is an
/// OVERLOADED internal node — load above mean + 1 standard deviation over the
/// loaded internal nodes — get their layout weight multiplied by
/// `min(node_load / mean_load, 4)`, accumulated across rounds, i.e. an iteratively
/// reweighted layout. The reweighted layout warm-starts from the previous round's
/// positions, is re-tree-ified, and has its loads recomputed. This runs `fb` times
/// and the vtree with the LOWEST max clause-LCA load is kept, ties going to the
/// earliest round — so round 0 (the `fb = 0` result) is always a candidate and the
/// kept load can only improve.
///
/// Random restarts (`seeds > 1`): the whole per-seed pipeline above reruns from
/// `SEED + i` for restart `i`, and the lowest-max-clause-LCA-load vtree is kept
/// across restarts, ties going to the lowest seed. Restart 0 is the base [`SEED`],
/// so the kept load can only improve over `seeds = 1`.
pub(crate) fn vtree_from_force(
    formula: &CnfFormula,
    cfg: ForceConfig,
) -> Result<Arc<Vtree>, String> {
    let n = formula.num_vars as usize;
    if n == 0 {
        return Err(EMPTY_FORMULA.to_string());
    }
    debug_assert!(
        (2..=MAX_DIM).contains(&cfg.dim),
        "force dim out of range: {}",
        cfg.dim
    );
    let d = cfg.dim;
    let inc = build_incidence(formula);

    // Tree-ify one layout into (nodes, root) per the configured mode.
    let build_tree = |layout: &[Vec<f64>]| -> (VtreeArena, VtreeIdx) {
        let mut nodes = VtreeArena::with_capacity(2 * n - 1);
        let root = match cfg.mode {
            ForceMode::Mst => mst_tree(layout, &cfg, &inc, &mut nodes),
            ForceMode::Cut => {
                let all: Vec<u32> = (0..n as u32).collect();
                cut_tree(layout, d, &all, &mut nodes)
            }
        };
        (nodes, root)
    };

    // Build the best vtree for ONE seed: round 0 plus the `fb` metric-feedback
    // rounds, keeping the lowest-max-internal-load vtree. Returns that vtree and its
    // max clause-LCA load, which is the `seeds` restart objective. Round 0 is always
    // a candidate, so the kept load can only improve within a seed.
    let build_for_seed = |seed: u64| -> (Arc<Vtree>, u32) {
        let layout0 = force_layout(n, &inc, seed, &cfg, None, None);
        let (nodes0, root0) = build_tree(&layout0);
        let vtree0 = Arc::new(Vtree::from_nodes(
            nodes0.into_nodes(),
            root0,
            formula.num_vars,
        ));
        let (mut prev_lca, mut prev_loads) = clause_lca_nodes(&vtree0, formula);
        let mut best: BestBy<Arc<Vtree>, u32> = BestBy::new();
        best.offer(vtree0.clone(), max_internal_load(&vtree0, &prev_loads));
        if cfg.fb == 0 {
            return best.into_best().expect("the round-zero layout was offered");
        }
        let mut prev_vtree = vtree0;
        let mut prev_layout = layout0;
        let mut extra_w = vec![1.0f64; inc.nc];
        for _ in 0..cfg.fb {
            // Reweight from the PREVIOUS round's overloaded internal LCA nodes.
            // Over internal nodes only: unit clauses land on leaves, and the
            // feedback objective is the internal-node bottleneck.
            let stats = load_stats(&prev_loads, |t| !prev_vtree.node(t).is_leaf());
            if stats.count > 0 && stats.mean > EPS {
                let thresh = stats.mean + stats.stddev;
                for (c, w) in extra_w.iter_mut().enumerate() {
                    let node = prev_lca[c];
                    if !prev_vtree.node(node).is_leaf() && (prev_loads[node.idx()] as f64) > thresh
                    {
                        let factor = ((prev_loads[node.idx()] as f64) / stats.mean).min(4.0);
                        *w *= factor;
                    }
                }
            }
            let layout = force_layout(n, &inc, seed, &cfg, Some(&extra_w), Some(&prev_layout));
            let (nodes, root) = build_tree(&layout);
            let vtree = Arc::new(Vtree::from_nodes(
                nodes.into_nodes(),
                root,
                formula.num_vars,
            ));
            let (lca, loads) = clause_lca_nodes(&vtree, formula);
            best.offer(vtree.clone(), max_internal_load(&vtree, &loads));
            prev_lca = lca;
            prev_loads = loads;
            prev_vtree = vtree;
            prev_layout = layout;
        }
        best.into_best().expect("the round-zero layout was offered")
    };

    // `seeds` random restarts: restart 0 uses the base SEED, restart i uses
    // `SEED + i`. Keep the lowest-max-load vtree; a tie (load not strictly smaller)
    // goes to the earliest restart, i.e. the lowest seed.
    let mut best: BestBy<Arc<Vtree>, u32> = BestBy::new();
    for i in 0..(cfg.seeds as u64).max(1) {
        let (vtree, load) = build_for_seed(SEED + i);
        best.offer(vtree, load);
    }
    Ok(best
        .into_best()
        .map(|(vtree, _)| vtree)
        .expect("at least one restart"))
}

#[cfg(test)]
mod tests;
