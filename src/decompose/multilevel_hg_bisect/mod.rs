//! Recursive-vtree adapter for goatd's hypergraph partitioner.

#[cfg(test)]
mod tests;

use std::sync::Arc;

use crate::cnf::CnfFormula;
use crate::vtree::Vtree;

use super::{BisectDials, Bisection};
use super::{BisectionSolver, local_index};

/// The imbalance a bare `hypergraph-bisect` spec means.
pub(crate) const IMBALANCE_BALANCED: f64 = 0.03;

/// The imbalance the portfolio's `hypergraph-bisect` entry runs at, which its
/// `param` spells as `imbalance=0.40` so the winning spec rebuilds the tree.
/// `the_bisection_candidate_records_the_imbalance_it_builds_at` holds the two
/// to each other.
pub(crate) const IMBALANCE_PORTFOLIO_RELAXED: f64 = 0.40;

/// One cut of a hypergraph under goatd's multilevel partitioner: the side bit
/// of each vertex, with as few hyperedges as possible spanning both sides.
///
/// `effort_scale` is this run's construction-effort multiplier
/// ([`crate::budget::vtree_effort_scale`]); `1.0` is the calibrated baseline.
/// The hypergraph partitioner is the one bisection that spends it.
///
/// # Errors
///
/// The partitioner's own message when it refuses the hypergraph, and a vertex
/// count that does not fit goatd's `u32` vertex ids.
pub(crate) fn multilevel_hg_bisect(
    num_vertices: usize,
    hyperedges: &[Vec<u32>],
    hyperedge_weights: Option<&[u32]>,
    dials: BisectDials,
    effort_scale: f64,
) -> Result<Vec<u8>, String> {
    let num_vertices = u32::try_from(num_vertices)
        .map_err(|_| "hypergraph vertex count does not fit in u32".to_string())?;
    let hypergraph =
        ::goatd::partition::Hypergraph::new(num_vertices, hyperedges, hyperedge_weights)
            .map_err(|error| error.to_string())?;
    let config =
        ::goatd::partition::HypergraphBisectionConfig::new(dials.imbalance, dials.base_seed)
            .with_effort(effort_scale);
    ::goatd::partition::multilevel_hypergraph_bisect(&hypergraph, config)
        .map(::goatd::partition::Bisection::into_parts)
        .map_err(|error| error.to_string())
}

/// Bisects by clauses: each clause of the subproblem is a hyperedge, so a cut
/// is chosen to leave as few clauses as possible spanning both sides.
pub(crate) struct HypergraphBisectSolver {
    pub dials: BisectDials,
    /// See [`multilevel_hg_bisect`].
    pub effort_scale: f64,
}

impl BisectionSolver for HypergraphBisectSolver {
    fn partition(
        &mut self,
        vars: &[u32],
        formula: &CnfFormula,
    ) -> Result<Option<Bisection>, String> {
        let local_idx = local_index(vars);
        let mut hyperedges = Vec::new();
        for clause in &formula.clauses {
            let mut pins: Vec<u32> = clause
                .literals
                .iter()
                .filter_map(|literal| local_idx.get(&literal.var.0).copied())
                .collect();
            pins.sort_unstable();
            pins.dedup();
            if pins.len() >= 2 {
                hyperedges.push(pins);
            }
        }
        if hyperedges.is_empty() {
            return Ok(None);
        }
        let weights: Vec<u32> = hyperedges
            .iter()
            .map(|hyperedge| (hyperedge.len() - 1) as u32)
            .collect();
        let parts = multilevel_hg_bisect(
            vars.len(),
            &hyperedges,
            Some(&weights),
            self.dials,
            self.effort_scale,
        )?;
        Ok(Bisection::from_side_bits(vars, &parts))
    }

    fn deadline(&self) -> Option<std::time::Instant> {
        self.dials.deadline
    }
}

/// Build a vtree by bisecting the clause hypergraph recursively.
///
/// The `hypergraph-bisect` spec. Unlike the decomposition-derived families this
/// builds the tree top down and reads no [`Reading`](super::Reading).
///
/// # Errors
///
/// The partitioner's message, or the bisection's own once it gives up.
pub(crate) fn vtree_from_hg_bisect(
    formula: &CnfFormula,
    dials: BisectDials,
    effort_scale: f64,
) -> Result<Arc<Vtree>, String> {
    let mut solver = HypergraphBisectSolver {
        dials,
        effort_scale,
    };
    super::run_bisection(formula, &mut solver)
}
