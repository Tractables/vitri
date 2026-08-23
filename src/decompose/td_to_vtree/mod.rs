//! Tree decomposition to vtree conversion.
//!
//! The construction that carries a decomposition's width over to the tree: each
//! bag becomes a subtree, so a narrow decomposition converts to a vtree with
//! narrow separators.

mod algo;
mod combiners;
mod config;
mod meta;
mod portfolio;

pub(crate) use algo::{ConversionInput, td_to_vtree_configured_traced};
pub use config::{BagAssignment, ItemOrdering, TdRootStrategy, TdToVtreeConfig, VarOrderInBag};
pub use meta::BagMetadata;
pub(crate) use portfolio::{TdConversionMeta, td_to_vtree_best, td_to_vtree_best_traced};

use super::TreeDecomposition;
use crate::cnf::CnfFormula;
use crate::vtree::Vtree;

/// Convert a tree decomposition into a vtree over variables `1..=num_vars`,
/// pushing each shared variable into its deepest bag.
///
/// Where a decomposition from any source — [`parse_pace_td`](crate::decompose::parse_pace_td),
/// a solver this crate does not wrap, one you built yourself — becomes a vtree.
/// The default assignment is the one that pays off: it exploits the running
/// intersection property to keep clause LCAs low in the tree. Use
/// [`td_to_vtree_with_assignment`] to say otherwise, or
/// [`td_to_vtree_configured`] to set every knob.
pub fn td_to_vtree(td: &TreeDecomposition, num_vars: u32) -> Vtree {
    td_to_vtree_with_assignment(td, num_vars, BagAssignment::Deepest)
}

/// [`td_to_vtree`] with the bag-assignment strategy named explicitly. See
/// [`BagAssignment`] for what the choice costs.
pub fn td_to_vtree_with_assignment(
    td: &TreeDecomposition,
    num_vars: u32,
    assignment: BagAssignment,
) -> Vtree {
    td_to_vtree_configured(
        td,
        num_vars,
        &TdToVtreeConfig::from_bag_assignment(assignment),
        None,
    )
}

/// Full-featured TD → vtree conversion with all configurable heuristics.
///
/// `formula` is what the clause-aware knobs read. Several of them —
/// [`ItemOrdering::ClauseSplit`], [`ItemOrdering::HypergraphBisect`],
/// [`ItemOrdering::TdEdgeAligned`], [`VarOrderInBag::ClauseAffinity`] — describe
/// an ordering in terms of which variables share clauses, so passing `None`
/// leaves them nothing to order by and they fall back to the plain balanced
/// combiner. Pass `Some(formula)` whenever the decomposition came from a CNF
/// that is still in hand: this is then the same conversion, with the same
/// knobs, that every construction in this crate reaches.
///
/// Runs at the baseline construction effort. The knobs that bisect scale how
/// hard they search with the wall-clock hint a whole run was given, which a
/// single conversion of a decomposition already in hand has no share of.
pub fn td_to_vtree_configured(
    td: &TreeDecomposition,
    num_vars: u32,
    config: &TdToVtreeConfig,
    formula: Option<&CnfFormula>,
) -> Vtree {
    td_to_vtree_configured_traced(
        ConversionInput {
            td,
            num_vars,
            formula,
            effort_scale: 1.0,
        },
        config,
    )
    .0
}

#[cfg(test)]
mod tests;
