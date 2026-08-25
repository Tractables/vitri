//! Hybrid TD + bisection vtree construction.
//!
//! Combines tree decomposition (structural quality within clusters) with graph
//! partitioning (good top-level splits). At each recursion level, the pre-computed
//! TD is projected onto each partition half and compared against the bisection split.
//! Recursion stops when the projected TD alone produces a better vtree.
//!
//! The recursion itself is the shared one in [`super::bisect`]: this module
//! supplies the per-level partition and the projected-TD alternative, nothing
//! more.

use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::cnf::{Clause, CnfFormula, Literal};
use crate::score::{BUILT_FROM_THIS_FORMULA, vtree_cost};
use crate::vtree::{VarId, Vtree, VtreeArena, VtreeIdx, VtreeNode};

use super::best::select_first_min;
use super::multilevel_bisect::multilevel_bisect;
use super::td_ops::project_td;
use super::td_parse::restrict_to_subset;
use super::{
    BisectDials, Bisection, BisectionSolver, EMPTY_FORMULA, TdConversion, TreeDecomposition,
    build_primal_edges, local_index, run_bisection, td_to_vtree_best,
};

// ---------------------------------------------------------------------------
// Formula restriction
// ---------------------------------------------------------------------------

/// Build a formula restricted to a variable subset: keep only clauses whose
/// variables are ALL in the subset, and renumber variables to 0..k.
fn restrict_formula(
    formula: &CnfFormula,
    keep_vars: &FxHashSet<u32>,
    global_to_local: &FxHashMap<u32, u32>,
) -> CnfFormula {
    let num_local = global_to_local.len() as u32;
    // One scan of the WHOLE formula, taken once per recursion level: every
    // clause is tested for containment whether or not it survives. The `+ 1` per
    // clause prices the test itself, so a formula of short clauses is not
    // charged as though it were free.
    crate::decompose::meter::charge(
        formula
            .clauses
            .iter()
            .map(|c| c.literals.len() as u64 + 1)
            .sum(),
    );
    let mut clauses = Vec::new();
    for clause in &formula.clauses {
        if clause.literals.iter().all(|l| keep_vars.contains(&l.var.0)) {
            let lits: Vec<Literal> = clause
                .literals
                .iter()
                .map(|l| Literal {
                    var: VarId(global_to_local[&l.var.0]),
                    positive: l.positive,
                })
                .collect();
            clauses.push(Clause::new(lits));
        }
    }
    CnfFormula {
        num_vars: num_local,
        clauses,
    }
}

// ---------------------------------------------------------------------------
// Core recursive algorithm
// ---------------------------------------------------------------------------

/// At or below this many variables, one hybrid level skips bisection entirely
/// and builds the subtree from a local min-fill TD. Set higher than the generic
/// bisection fallback because a hybrid level also pays for a TD projection and a
/// candidate comparison on top of the partition, and neither earns its cost on a
/// subset this small.
const HYBRID_DIRECT_MINFILL_VARS: usize = 64;

/// The hybrid backend as the shared bisection framework sees it: a multilevel
/// bisection per level, plus the projected-TD alternative offered through the
/// framework's refinement hook.
struct HybridSolver<'a> {
    /// The decomposition each level projects onto its own variable subset.
    td: &'a TreeDecomposition,
    /// Primal edges of the whole formula, restricted per level.
    all_edges: &'a [(u32, u32)],
    dials: BisectDials,
}

impl BisectionSolver for HybridSolver<'_> {
    fn partition(&mut self, vars: &[u32], _formula: &CnfFormula) -> Option<Bisection> {
        let local_edges = restrict_to_subset(self.all_edges, vars);
        Bisection::from_side_bits(
            vars,
            &multilevel_bisect(
                vars.len(),
                &local_edges,
                self.dials.imbalance,
                self.dials.base_seed,
            ),
        )
    }

    fn minfill_cutoff(&self) -> usize {
        HYBRID_DIRECT_MINFILL_VARS
    }

    /// Score the subtree the bisection produced against the TD projected onto
    /// the same variables, and keep whichever is cheaper. This is what makes the
    /// construction hybrid: the partition proposes, the decomposition may
    /// override it, level by level.
    fn refine_subtree(
        &mut self,
        vars: &[u32],
        formula: &CnfFormula,
        nodes: &mut VtreeArena,
        checkpoint: usize,
        root: VtreeIdx,
    ) -> Option<VtreeIdx> {
        let keep: FxHashSet<u32> = vars.iter().copied().collect();
        let mut sorted_vars: Vec<u32> = vars.to_vec();
        sorted_vars.sort_unstable();
        let global_to_local = local_index(&sorted_vars);
        let local_formula = restrict_formula(formula, &keep, &global_to_local);

        // Projecting scans every bag of the WHOLE decomposition, once per
        // recursion level, filtering each bag's vertices against `keep`. The
        // `+ 1` per bag prices the per-bag work a bag with no surviving vertices
        // still costs.
        crate::decompose::meter::charge(
            self.td
                .bags
                .iter()
                .map(|b| b.vertices.len() as u64 + 1)
                .sum(),
        );
        let proj = project_td(self.td, &keep)?;
        let td_vtree = td_to_vtree_best(
            &proj.td,
            proj.td.num_vars,
            &local_formula,
            self.dials.effort_scale,
            None,
        );
        let td_score = vtree_cost(&td_vtree, &local_formula).expect(BUILT_FROM_THIS_FORMULA);

        // The bisected subtree, read back out of the shared arena in the same
        // local variable space the projection was scored in.
        let hybrid_nodes_local: Vec<VtreeNode> = nodes.nodes()[checkpoint..]
            .iter()
            .map(|node| match *node {
                VtreeNode::Leaf { var, parent } => VtreeNode::Leaf {
                    var: VarId(global_to_local[&var.0]),
                    parent,
                },
                VtreeNode::Internal {
                    left,
                    right,
                    parent,
                } => VtreeNode::Internal {
                    left: VtreeIdx(left.0 - checkpoint as u32),
                    right: VtreeIdx(right.0 - checkpoint as u32),
                    parent,
                },
            })
            .collect();
        let hybrid_local_root = VtreeIdx((root.0 as usize - checkpoint) as u32);
        let hybrid_vtree = Vtree::from_nodes(
            hybrid_nodes_local,
            hybrid_local_root,
            local_formula.num_vars,
        );
        let hybrid_score =
            vtree_cost(&hybrid_vtree, &local_formula).expect(BUILT_FROM_THIS_FORMULA);

        // The projection is offered first, so a tie keeps it: the
        // decomposition is the reason to run a hybrid level at all.
        let keep_projection =
            select_first_min([(true, td_score), (false, hybrid_score)], |&(_, score)| {
                score
            })
            .is_some_and(|(is_projection, _)| is_projection);

        keep_projection.then(|| {
            nodes.truncate(checkpoint);
            nodes.graft(&td_vtree, |local| {
                VarId(proj.local_to_global[local.0 as usize])
            })
        })
    }
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// The partition imbalance the `hybrid-flowcutter-incidence` construction runs
/// at. ONE value, read by the portfolio candidate of that name and by the
/// standalone spec of that name — they are the same construction and must not
/// be able to drift into two.
pub(super) const HYBRID_INCIDENCE_IMBALANCE: f64 = 0.40;

/// Entry point: hybrid TD + bisection construction, given a pre-computed tree
/// decomposition.
pub(super) fn vtree_from_hybrid(
    formula: &CnfFormula,
    td: &TreeDecomposition,
    dials: BisectDials,
) -> Result<Arc<Vtree>, String> {
    if formula.num_vars == 0 {
        return Err(EMPTY_FORMULA.to_string());
    }
    let edges = build_primal_edges(formula);
    let mut solver = HybridSolver {
        td,
        all_edges: &edges,
        dials,
    };
    run_bisection(formula, &mut solver)
}

// ---------------------------------------------------------------------------
// The combiner over a FlowCutter incidence decomposition
//
// Written ONCE here. `hybrid-flowcutter-incidence` is reached from two places —
// the portfolio candidate of that name, which hands in the incidence
// decomposition candidate 1 already built, and the `--vtree` spec of that name,
// which builds its own first. That reuse is the only difference between the two
// routes, so the standalone spec and the candidate cannot drift apart.
// ---------------------------------------------------------------------------

/// THE `hybrid-flowcutter-incidence` combiner: the hybrid TD + bisection
/// assembly over an already-built incidence decomposition, at the one imbalance
/// that construction runs at ([`HYBRID_INCIDENCE_IMBALANCE`]).
///
/// A recombination of several sub-decomposition conversions, so no one
/// conversion's bag assignment describes the result and it carries none.
pub(super) fn hybrid_from_incidence_td(
    formula: &CnfFormula,
    td: &TreeDecomposition,
    effort_scale: f64,
) -> Result<TdConversion, String> {
    let dials = BisectDials {
        imbalance: HYBRID_INCIDENCE_IMBALANCE,
        base_seed: 0,
        effort_scale,
    };
    vtree_from_hybrid(formula, td, dials).map(TdConversion::bare)
}

#[cfg(test)]
mod tests;
