//! Recursive-vtree adapter for goatd's graph partitioner.

use crate::cnf::CnfFormula;

use super::{BisectDials, Bisection, BisectionSolver};

pub(crate) struct PrimalBisectSolver<'a> {
    pub graph: &'a ::goatd::Graph,
    pub dials: BisectDials,
}

impl BisectionSolver for PrimalBisectSolver<'_> {
    fn partition(&mut self, vars: &[u32], _formula: &CnfFormula) -> Option<Bisection> {
        let local_graph = self.graph.induced_subgraph(vars).ok()?;
        let parts = multilevel_bisect(&local_graph, self.dials.imbalance, self.dials.base_seed)?;
        Bisection::from_side_bits(vars, &parts)
    }
}

pub(crate) fn vtree_from_primal_bisect(
    formula: &CnfFormula,
    dials: BisectDials,
) -> Result<std::sync::Arc<crate::vtree::Vtree>, String> {
    let graph = super::GraphKind::Primal.build(formula).as_goatd();
    let mut solver = PrimalBisectSolver {
        graph: &graph,
        dials,
    };
    super::run_bisection(formula, &mut solver)
}

pub(super) fn multilevel_bisect(
    graph: &::goatd::Graph,
    max_imbalance: f64,
    seed: u64,
) -> Option<Vec<u8>> {
    ::goatd::partition::multilevel_graph_bisect(
        graph,
        ::goatd::partition::GraphBisectionConfig::new(max_imbalance, seed),
    )
    .ok()
    .map(::goatd::partition::Bisection::into_parts)
}
