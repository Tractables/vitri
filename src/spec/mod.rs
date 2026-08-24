//! Vtree construction dispatch: a parsed `--vtree` spec to the backend that
//! builds it.
//!
//! The spec **grammar** — what a spec string may say, and what it means — is
//! `parse`; the construction backends are `builders`, one per decomposer
//! family. This module is what joins them: it takes a spec string through the
//! one parse and hands the result to the one backend that family names. Every
//! consumer, the standalone tool and an embedding caller alike, comes through
//! here.
//!
//! What stays *outside* is the orchestration around a single construction:
//! splitting a formula into independent components and building one vtree each
//! ([`crate::component`]), settling `best=auto` against the whole formula's
//! variable count, and
//! structural-spec routing. Those callers call into this module.

use std::sync::Arc;

use crate::cnf::CnfFormula;
use crate::decompose::{BuildLimits, SelectionCtx};
use crate::error::{VitriError, from_construction};
use crate::vtree::Vtree;

mod builders;
mod parse;

use builders::{
    build_vtree_elimination, build_vtree_flowcutter, build_vtree_goatd, build_vtree_portfolio,
};
use parse::unknown_vtree_type;

pub(crate) use parse::{
    BALANCED_SPEC, ParsedSpec, SpecParam, VtreeBase, parse_vtree_spec, spec_has_candidates,
    spec_string,
};
pub use parse::{SpecParamDoc, spec_param_docs, validate_vtree_spec, vtree_spec_bases};
// Reached only from tests. Production code holds a parsed spec, which already
// carries the family `classify_base` would resolve and has already had its
// `best=auto` settled against the formula, so neither is re-derived downstream.
#[cfg(test)]
pub(crate) use parse::{BEST_AUTO_MAX_VARS, classify_base};

/// The default `--vtree` spec — the ONE literal for it in this crate.
///
/// `portfolio` builds several candidate vtrees for the same formula, scores
/// each against the CNF, and returns the best by this crate's cost model. It is
/// what both the standalone tool and an embedding caller get unless they say
/// otherwise, and the only construction that has a candidate set to retain
/// ([`crate::candidates`]).
pub const DEFAULT_VTREE_SPEC: &str = "portfolio";

/// Every single elimination order, in table order (`minfill`, `mindegree`, …).
///
/// These are ORDER names, not whole specs: a spec writes the order with the
/// graph view it runs on after it, and [`vtree_spec_bases`] is the list of
/// what a caller may type.
pub fn elimination_spec_names() -> impl Iterator<Item = &'static str> {
    crate::decompose::elimination_spec_names()
}

/// Every `--vtree` spec name that builds the tree from a decomposition — or a
/// partition — of a graph view of the CNF, in grammar order. These are the
/// constructions the portfolio chooses among, each also nameable on its own,
/// and each taking the parameters its family accepts.
///
/// `portfolio` itself, the force-directed embedding and the numbering-only
/// baselines are not among them; neither are the single elimination orders,
/// whose orders are [`elimination_spec_names`].
pub fn decomposition_spec_names() -> impl Iterator<Item = &'static str> {
    parse::decomposition_spec_names()
}

/// Every `--vtree` spec name that builds from the variable numbering alone,
/// consulting no clause, in grammar order.
pub fn baseline_spec_names() -> impl Iterator<Item = &'static str> {
    parse::baseline_spec_names()
}

/// Every `--vtree` spec name that stands on its own rather than inside a list:
/// [`DEFAULT_VTREE_SPEC`], the one construction with a candidate set, and the
/// force-directed embedding, which carries an axis grammar of its own
/// ([`spec_param_docs`]).
///
/// With [`decomposition_spec_names`] and [`baseline_spec_names`], this completes
/// the base vocabulary outside the single elimination orders — so a shell can
/// offer every name it will accept.
pub fn standalone_spec_names() -> impl Iterator<Item = &'static str> {
    parse::standalone_spec_names()
}

/// What one vtree construction reported about the tree it selected.
///
/// Describes the vtree it was returned alongside, and no other.
#[derive(Clone, Debug, Default)]
pub struct SelectionRecord {
    /// The `--vtree` spec that rebuilds this vtree. Under a portfolio spec it
    /// is the winning candidate, spelled with the parameter it was built at
    /// (`hypergraph-bisect:imbalance=0.40`, not the bare family, whose default
    /// imbalance
    /// is a different tree); `minfill` for a component small enough to skip the
    /// portfolio; and for any other spec, the base the caller asked for, who
    /// already holds the rest of what they typed.
    pub winning_spec: Option<String>,
    /// Tree-decomposition bag metadata of this vtree, when the conversion that
    /// produced it kept one. `None` for a vtree no TD conversion produced, and
    /// for one recombined from several conversions (no single bag assignment
    /// describes it). Variable ids are in the space of the formula the vtree was
    /// built for — component-LOCAL for a per-component entry — so a consumer must
    /// check [`crate::decompose::BagMetadata::num_vars`] before using it.
    pub td_meta: Option<Arc<crate::decompose::BagMetadata>>,
}

/// Everything one construction produced: the tree, what the selection behind it
/// reported about itself, and the candidate set it retained.
///
/// The candidate set and the selection record are by-products of the same
/// build — a spec that is not the portfolio leaves both at their default.
/// Returning them together lets `component` graft per-component vtrees and
/// their selection records in lockstep.
#[derive(Clone)]
pub(crate) struct VtreeArtifacts {
    /// The constructed vtree.
    pub vtree: Arc<Vtree>,
    /// Which candidate produced it and the TD metadata describing it.
    pub selection: SelectionRecord,
    /// The retained candidate set — empty unless the caller asked for one.
    pub candidate_set: crate::candidates::CandidateSet,
}

impl VtreeArtifacts {
    /// A construction with nothing to report beside the tree (a named simple
    /// vtree, a bisection, the force-directed embedding). The spec that ran is
    /// the only construction that ran, so it is the only honest answer to
    /// "which spec produced this".
    fn bare(vtree: Arc<Vtree>, spec: &ParsedSpec<'_>) -> Self {
        VtreeArtifacts {
            vtree,
            selection: SelectionRecord {
                winning_spec: Some(spec.to_string()),
                td_meta: None,
            },
            candidate_set: crate::candidates::CandidateSet::default(),
        }
    }

    /// A single-backend TD construction: no portfolio, so the spec that ran
    /// names it and there is no candidate set, but the conversion's own bag
    /// metadata travels with its tree.
    fn from_td(built: crate::decompose::TdConversion, spec: &ParsedSpec<'_>) -> Self {
        VtreeArtifacts {
            vtree: built.vtree,
            selection: SelectionRecord {
                winning_spec: Some(spec.to_string()),
                td_meta: built.td.meta,
            },
            candidate_set: crate::candidates::CandidateSet::default(),
        }
    }
}

/// One vtree construction, as the four things that describe it: the formula to
/// build over, the parsed spec naming the construction, what selection is to
/// optimize for, and what it may spend.
///
/// The four travel together from the entry point down to the dispatch below. A
/// component build re-points the formula, the selection context and the limits
/// at that component's own and carries the same spec — which is the whole of
/// what building a component differs in.
#[derive(Clone, Copy)]
pub(crate) struct BuildRequest<'a> {
    /// The formula to build over — a component's LOCAL formula on the
    /// per-component path, the whole one otherwise.
    pub formula: &'a CnfFormula,
    /// Which construction to run, already read through the grammar.
    pub spec: &'a ParsedSpec<'a>,
    /// What the construction is to optimize for.
    pub ctx: &'a SelectionCtx,
    /// What the construction may spend.
    pub limits: &'a BuildLimits,
}

/// Dispatch a parsed `--vtree` spec to the matching construction backend and
/// return everything it produced beside the tree.
///
/// Takes the parsed value rather than the string: the caller above holds one
/// parse for the whole build, so a formula split into components dispatches
/// each of them without re-reading the grammar.
///
/// # Errors
///
/// [`VitriError::Spec`] for a spec this crate cannot build, [`VitriError::Env`]
/// for a `VITRI_*` variable the construction reads, and
/// [`VitriError::Construction`] when the chosen construction ran and could not
/// produce a vtree.
pub(crate) fn build_one_vtree_artifacts(
    req: BuildRequest<'_>,
) -> Result<VtreeArtifacts, VitriError> {
    let BuildRequest {
        formula,
        spec: parsed,
        ctx,
        limits,
    } = req;
    let num_vars = formula.num_vars;
    // ONE effort multiplier for the whole build, from the budget hint the
    // limits carry.
    let effort_scale = crate::budget::vtree_effort_scale(limits.budget_ms);
    match parsed.family {
        // --- Named simple vtrees ---------------------------------------
        // The parse guarantees these carry no parameters.
        VtreeBase::Balanced => Ok(VtreeArtifacts::bare(
            Arc::new(Vtree::balanced(num_vars)),
            parsed,
        )),
        VtreeBase::Linear => Ok(VtreeArtifacts::bare(
            Arc::new(Vtree::linear(num_vars)),
            parsed,
        )),
        VtreeBase::ReverseLinear => Ok(VtreeArtifacts::bare(
            Arc::new(Vtree::reverse_linear(num_vars)),
            parsed,
        )),
        VtreeBase::Random => Ok(VtreeArtifacts::bare(
            Arc::new(Vtree::random(num_vars, 0)),
            parsed,
        )),

        // --- TD-based vtrees (the goatd family) ------------------------
        VtreeBase::Goatd { incidence } => {
            build_vtree_goatd(formula, parsed, incidence, ctx.goatd, effort_scale)
                .map(|b| VtreeArtifacts::from_td(b, parsed))
        }

        // --- FlowCutter vtrees (timed and step-budgeted) --------------
        VtreeBase::Flowcutter { .. } => build_vtree_flowcutter(formula, parsed, effort_scale)
            .map(|b| VtreeArtifacts::from_td(b, parsed)),

        // --- Portfolio vtrees -----------------------------------------
        VtreeBase::Portfolio => build_vtree_portfolio(formula, ctx, limits),

        // --- One elimination order (minfill, mindegree, …) -------------
        VtreeBase::Elimination { name, incidence } => {
            build_vtree_elimination(formula, parsed, name, incidence, effort_scale)
                .map(|b| VtreeArtifacts::from_td(b, parsed))
        }

        // --- Multilevel-hypergraph bisection --------------------------
        VtreeBase::HypergraphBisect => {
            let dials = crate::decompose::BisectDials {
                imbalance: parsed.param.imbalance(),
                base_seed: 0,
                effort_scale,
            };
            from_construction(
                crate::decompose::vtree_from_hg_bisect(formula, dials),
                parsed,
            )
            .map(|v| VtreeArtifacts::bare(v, parsed))
        }

        // --- Force-directed embedding ---------------------------------
        VtreeBase::Force => {
            let cfg = parsed.param.force();
            from_construction(crate::decompose::vtree_from_force(formula, cfg), parsed)
                .map(|v| VtreeArtifacts::bare(v, parsed))
        }

        VtreeBase::Unknown => Err(unknown_vtree_type(parsed.raw)),
    }
}
