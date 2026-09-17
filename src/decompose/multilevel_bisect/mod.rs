//! Recursive-vtree adapter for goatd's graph partitioner.

#[cfg(test)]
mod tests;

use crate::cnf::CnfFormula;

use super::{BisectDials, Bisection, BisectionSolver};

/// Bisects by co-occurrence: the cut is taken in the primal graph, so it
/// separates variables rather than clauses, and the formula is not consulted.
pub(crate) struct PrimalBisectSolver<'a> {
    pub graph: &'a ::goatd::Graph,
    pub dials: BisectDials,
}

impl BisectionSolver for PrimalBisectSolver<'_> {
    fn partition(
        &mut self,
        vars: &[u32],
        _formula: &CnfFormula,
    ) -> Result<Option<Bisection>, String> {
        let local_graph = self
            .graph
            .induced_subgraph(vars)
            .map_err(|error| error.to_string())?;
        let parts = multilevel_bisect(&local_graph, self.dials.imbalance, self.dials.base_seed)?;
        Ok(Bisection::from_side_bits(vars, &parts))
    }

    fn deadline(&self) -> Option<std::time::Instant> {
        self.dials.deadline
    }
}

/// Build a vtree by bisecting the primal graph recursively.
///
/// The `primal-bisect` spec, the graph-partitioner twin of
/// [`vtree_from_hg_bisect`](super::multilevel_hg_bisect::vtree_from_hg_bisect).
///
/// # Errors
///
/// The partitioner's message, or the bisection's own once it gives up.
pub(crate) fn vtree_from_primal_bisect(
    formula: &CnfFormula,
    dials: BisectDials,
) -> Result<std::sync::Arc<crate::vtree::Vtree>, String> {
    let pace = super::GraphKind::Primal.build(formula);
    let mut solver = PrimalBisectSolver {
        graph: pace.as_goatd(),
        dials,
    };
    super::run_bisection(formula, &mut solver)
}

/// One cut of a graph under goatd's multilevel partitioner: the side bit of
/// each vertex, with as few edges as possible crossing.
///
/// # Errors
///
/// The partitioner's own message when it refuses the graph.
pub(super) fn multilevel_bisect(
    graph: &::goatd::Graph,
    max_imbalance: f64,
    seed: u64,
) -> Result<Vec<u8>, String> {
    ::goatd::partition::multilevel_graph_bisect(
        graph,
        ::goatd::partition::GraphBisectionConfig::new(max_imbalance, seed),
    )
    .map(::goatd::partition::Bisection::into_parts)
    .map_err(|error| error.to_string())
}
