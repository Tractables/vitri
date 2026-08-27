//! Recursive-vtree adapter for goatd's hypergraph partitioner.

use std::sync::Arc;

use crate::cnf::CnfFormula;
use crate::vtree::Vtree;

use super::{BisectDials, Bisection};
use super::{BisectionSolver, local_index};

pub(crate) const IMBALANCE_BALANCED: f64 = 0.03;
pub(crate) const IMBALANCE_PORTFOLIO_RELAXED: f64 = 0.40;

pub(crate) fn multilevel_hg_bisect(
    num_vertices: usize,
    hyperedges: &[Vec<u32>],
    hyperedge_weights: Option<&[u32]>,
    dials: BisectDials,
) -> Option<Vec<u8>> {
    let num_vertices = u32::try_from(num_vertices).ok()?;
    let hypergraph =
        ::goatd::partition::Hypergraph::new(num_vertices, hyperedges, hyperedge_weights).ok()?;
    let config =
        ::goatd::partition::HypergraphBisectionConfig::new(dials.imbalance, dials.base_seed)
            .with_effort(dials.effort_scale);
    ::goatd::partition::multilevel_hypergraph_bisect(&hypergraph, config)
        .ok()
        .map(::goatd::partition::Bisection::into_parts)
}

pub(crate) struct HypergraphBisectSolver {
    pub dials: BisectDials,
}

impl BisectionSolver for HypergraphBisectSolver {
    fn partition(&mut self, vars: &[u32], formula: &CnfFormula) -> Option<Bisection> {
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
            return None;
        }
        let weights: Vec<u32> = hyperedges
            .iter()
            .map(|hyperedge| (hyperedge.len() - 1) as u32)
            .collect();
        let parts = multilevel_hg_bisect(vars.len(), &hyperedges, Some(&weights), self.dials)?;
        Bisection::from_side_bits(vars, &parts)
    }
}

pub(crate) fn vtree_from_hg_bisect(
    formula: &CnfFormula,
    dials: BisectDials,
) -> Result<Arc<Vtree>, String> {
    let mut solver = HypergraphBisectSolver { dials };
    super::run_bisection(formula, &mut solver)
}
