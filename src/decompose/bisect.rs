//! Recursive-bisection vtree construction: the framework every bisecting
//! backend plugs into.
//!
//! A backend supplies one two-way split of a variable subset
//! ([`BisectionSolver::partition`]); [`bisect_recursive_generic`] owns the
//! recursion, the internal-node construction, the deadline check, and what to
//! do when a split comes back degenerate — a local min-fill elimination order
//! on a small subset, a midpoint split above that. [`minfill_subtree`] is that
//! same fallback, and is the base case for subsets at or below the solver's
//! cutoff. A backend that can cover the same variables another way offers its
//! alternative through [`BisectionSolver::refine_subtree`], which the recursion
//! consults once per assembled internal node.
//!
//! Subtrees are appended to a caller-owned [`VtreeArena`] with every
//! `parent` left `None`, and the returned [`VtreeIdx`] names the subtree's
//! root; parent links are wired once at the end by `Vtree::from_nodes`.
//! Variable ids are formula variable indices throughout — `minfill_subtree`
//! renumbers to a local `0..n` space for the elimination and maps back before
//! pushing leaves.

use std::time::Instant;

use crate::budget::expired;
use crate::cnf::CnfFormula;
use crate::vtree::{VarId, Vtree, VtreeArena, VtreeIdx};

/// The three dials a recursive bisection runs at, fixed for the whole of one
/// construction and passed down every level of it.
///
/// One value rather than three arguments: two of them are `f64`s that a call
/// site can transpose without a word from the compiler, and they mean opposite
/// things — one loosens the split, the other buys more work per split.
#[derive(Clone, Copy)]
pub(crate) struct BisectDials {
    /// How far a split may deviate from an even half/half; see
    /// [`IMBALANCE_BALANCED`](super::IMBALANCE_BALANCED).
    pub(crate) imbalance: f64,
    /// The RNG stream every bisection below draws from. `0` for an entry with
    /// no schedule seed of its own, which freezes that construction onto one
    /// fixed stream.
    pub(crate) base_seed: u64,
    /// This run's construction-effort multiplier
    /// ([`crate::budget::vtree_effort_scale`]), applied to every bisection
    /// below; `1.0` is the calibrated baseline.
    pub(crate) effort_scale: f64,
}

/// Default for [`BisectionSolver::minfill_cutoff`]: at or below this many
/// variables, the generic recursion stops calling the solver's `partition` and
/// builds the subtree from a local min-fill elimination order instead. Tuned
/// against partitioner call overhead — min-fill runs in microseconds on 32
/// variables, so paying for a multilevel bisection there is wasted work. A
/// solver that handles small subsets better overrides the method.
pub(crate) const SOLVER_FALLBACK_VARS: usize = 32;

/// Build a vtree subtree for a small variable subset via minfill elimination
/// order.
pub(crate) fn minfill_subtree(
    vars: &[u32],
    formula: &CnfFormula,
    nodes: &mut VtreeArena,
) -> VtreeIdx {
    let edges = super::td_parse::primal_edges_on_subset(formula, vars);
    minfill_subtree_from_local_edges(vars, vars.len() as u32, &edges, nodes)
}

/// Core: run minfill pipeline on already-local edges and transplant into shared nodes vec.
fn minfill_subtree_from_local_edges(
    vars: &[u32],
    n: u32,
    local_edges: &[(u32, u32)],
    nodes: &mut VtreeArena,
) -> VtreeIdx {
    // The graph is already built, so this enters the shared min-fill through the
    // prebuilt-graph seam rather than the formula one — same preprocessing, same
    // elimination core, at a fixed seed for determinism.
    let td = super::goatd::minfill_td_from_edges(n, local_edges, super::INTERNAL_ELIMINATION_SEED);
    let sub_vtree = super::td_to_vtree::td_to_vtree(&td, n);

    nodes.graft(&sub_vtree, |local| VarId(vars[local.0 as usize]))
}

// ---------------------------------------------------------------------------
// Generic recursive bisection framework
// ---------------------------------------------------------------------------

/// One half of a subset of variables and the other, each non-empty.
///
/// The only way to make one is [`Bisection::from_side_bits`], so a split with
/// an empty side does not exist and the recursion that consumes one can descend
/// into both without a check.
pub(crate) struct Bisection {
    /// The variables on side 0, in the order the subset listed them.
    pub(crate) left: Vec<u32>,
    /// The variables on side 1, in the order the subset listed them.
    pub(crate) right: Vec<u32>,
}

impl Bisection {
    /// The split `bits` describes over `vars`: entry `0` puts a variable left,
    /// anything else puts it right, position by position.
    ///
    /// `None` when what came back is not a split of `vars` at all — no bits, a
    /// side for every variable but all on one side, or a bit vector whose
    /// length disagrees with the subset. That is the answer a partitioner gives
    /// when nothing in the subset says how it should divide, and it is what the
    /// framework falls back from; testing for it here is what keeps every
    /// implementor from testing for it itself.
    pub(crate) fn from_side_bits(vars: &[u32], bits: &[u8]) -> Option<Bisection> {
        // A short bit vector says nothing about the variables past its end, and
        // pairing off what it does cover would silently drop them from the
        // vtree the split is built into. It is a partitioner that answered a
        // different question, so it is no answer to this one.
        if bits.len() != vars.len() {
            return None;
        }
        let mut left = Vec::new();
        let mut right = Vec::new();
        for (&var, &side) in vars.iter().zip(bits) {
            if side == 0 {
                left.push(var);
            } else {
                right.push(var);
            }
        }
        (!left.is_empty() && !right.is_empty()).then_some(Bisection { left, right })
    }
}

/// Contract for a backend that bisects a set of variables into two groups.
///
/// The framework (see [`run_bisection`] and [`bisect_recursive_generic`]) drives
/// recursive vtree construction; implementors only provide the per-step partition.
///
/// # `partition` contract
/// - **Input:** `vars` is a non-empty subset of variable indices (always
///   `vars.len() > minfill_cutoff()` — the framework handles smaller subsets
///   itself, so impls never see them).
/// - **Output:** the split, or `None` for "nothing in this subset says how it
///   divides", which the framework recovers from with a minfill or midpoint
///   split.
///
/// # State
/// `&mut self` may carry scratch buffers (e.g. dense `var_in_set` flags,
/// counters, tmpdirs) reused across calls. The framework invokes `partition`
/// recursively on shrinking subsets, so any per-call mutable state must be
/// restored before returning. External resources (tmpdirs, file handles)
/// should be tied to the solver's lifetime via `Drop`.
///
/// # Deadlines
/// Override [`deadline`] to return an absolute time after which construction
/// should abort. The framework checks it once per recursion node, before
/// invoking `partition`. Per-call timeout handling inside `partition` itself
/// is the implementor's responsibility.
pub(crate) trait BisectionSolver {
    fn partition(&mut self, vars: &[u32], formula: &CnfFormula) -> Option<Bisection>;

    /// Absolute deadline; checked by the framework before each `partition` call.
    /// Default: no deadline.
    fn deadline(&self) -> Option<Instant> {
        None
    }

    /// Subtree size at which the framework switches to minfill fallback instead
    /// of calling `partition`. `0` disables the fallback (solver handles all sizes).
    fn minfill_cutoff(&self) -> usize {
        SOLVER_FALLBACK_VARS
    }

    /// Second look at the subtree the framework just assembled from a real
    /// partition, for a solver that can cover the same variables another way
    /// and wants whichever candidate scores better.
    ///
    /// Called once per such node, with the subtree occupying
    /// `nodes[checkpoint..]` and rooted at `root`. Returning `Some` replaces it,
    /// and an implementor that does so truncates the arena back to `checkpoint`
    /// before pushing its replacement, so no unreachable nodes are left behind.
    /// Default: `None`, keeping what the recursion built.
    fn refine_subtree(
        &mut self,
        vars: &[u32],
        formula: &CnfFormula,
        nodes: &mut VtreeArena,
        checkpoint: usize,
        root: VtreeIdx,
    ) -> Option<VtreeIdx> {
        let _ = (vars, formula, nodes, checkpoint, root);
        None
    }
}

/// End-to-end driver for any [`BisectionSolver`]: builds the variable list,
/// runs recursive bisection, and packages the result into an `Arc<Vtree>`.
///
/// Every backend's top-level `vtree_from_*` should delegate here after
/// constructing its solver, instead of hand-rolling the same wrapper.
pub(crate) fn run_bisection<S: BisectionSolver>(
    formula: &CnfFormula,
    solver: &mut S,
) -> Result<std::sync::Arc<Vtree>, String> {
    let num_vars = formula.num_vars;
    let all_vars: Vec<u32> = (0..num_vars).collect();
    let mut nodes = VtreeArena::new();
    let root = bisect_recursive_generic(&all_vars, formula, &mut nodes, solver)?;
    Ok(std::sync::Arc::new(Vtree::from_nodes(
        nodes.into_nodes(),
        root,
        num_vars,
    )))
}

/// Generic recursive bisection algorithm for building vtrees. All
/// bisection-based vtree backends delegate to this function (via
/// [`run_bisection`]).
pub(crate) fn bisect_recursive_generic<S: BisectionSolver>(
    vars: &[u32],
    formula: &CnfFormula,
    nodes: &mut VtreeArena,
    solver: &mut S,
) -> Result<VtreeIdx, String> {
    if expired(solver.deadline()) {
        return Err("bisection vtree construction timed out".to_string());
    }

    if vars.len() == 1 {
        let idx = nodes.leaf(VarId(vars[0]));
        return Ok(idx);
    }
    if vars.len() == 2 {
        let l_idx = nodes.leaf(VarId(vars[0]));
        let r_idx = nodes.leaf(VarId(vars[1]));
        let idx = nodes.internal(l_idx, r_idx);
        return Ok(idx);
    }

    // Small-subtree fallback: minfill is structure-aware and runs in microseconds.
    let cutoff = solver.minfill_cutoff();
    if cutoff > 0 && vars.len() <= cutoff {
        return Ok(minfill_subtree(vars, formula, nodes));
    }

    let Some(split) = solver.partition(vars, formula) else {
        // No useful partition: use minfill if the subset is small enough
        // (minfill is O(n²) to O(n³) on dense graphs). For larger subsets,
        // fall back to a balanced midpoint split.
        if vars.len() <= 256 {
            return Ok(minfill_subtree(vars, formula, nodes));
        }
        let mid = vars.len() / 2;
        let l = bisect_recursive_generic(&vars[..mid], formula, nodes, solver)?;
        let r = bisect_recursive_generic(&vars[mid..], formula, nodes, solver)?;
        let idx = nodes.internal(l, r);
        return Ok(idx);
    };

    let checkpoint = nodes.len();
    let l = bisect_recursive_generic(&split.left, formula, nodes, solver)?;
    let r = bisect_recursive_generic(&split.right, formula, nodes, solver)?;
    let idx = nodes.internal(l, r);
    Ok(solver
        .refine_subtree(vars, formula, nodes, checkpoint, idx)
        .unwrap_or(idx))
}
