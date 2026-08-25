//! Vtree construction backends — one function per decomposer family.
//!
//! Each takes the already-parsed spec: none of them reads the spec string
//! itself, so the grammar has exactly one reader ([`super::parse_vtree_spec`]).

use crate::cnf::CnfFormula;
use crate::decompose::{
    BuildLimits, ConversionRequest, GoatdKnobs, GraphKind, SelectionCtx, TdConversion,
};
use crate::error::{VitriError, from_construction};

use super::{ParsedSpec, VtreeArtifacts, VtreeBase};

/// What every decomposition family hands the ONE conversion: which dimensions
/// the spec left open, what the build may spend, and the spec to report under.
///
/// Assembled once per build, here, so no family can read its decomposition
/// under a rule of its own.
pub(super) fn conversion_request<'a>(
    parsed: &'a ParsedSpec<'a>,
    ctx: &SelectionCtx,
    limits: &BuildLimits,
    effort_scale: f64,
) -> ConversionRequest<'a> {
    ConversionRequest {
        spec: Some(parsed.base),
        reading: parsed.reading,
        effort_scale,
        deadline: limits.deadline,
        trace: ctx.conversion.trace,
    }
}

/// The single elimination-order specs: `minfill`, `mindegree` and
/// `nested-dissection`, each in both graph views, each taking a `seed`
/// (default 0) and a `ties` core.
///
/// Each builds from ONE elimination order — no schedule, no refinement, no
/// lex-min selection — so a sweep can sample an order individually with many
/// seeds; a schedule's min-width winner hides structurally different tree
/// decompositions. Examples: `minfill`, `minfill-sample-jw:seed=7`,
/// `mindegree-inc:seed=3`.
pub(super) fn build_vtree_elimination(
    formula: &CnfFormula,
    parsed: &ParsedSpec<'_>,
    name: &'static str,
    incidence: bool,
    request: ConversionRequest<'_>,
) -> Result<TdConversion, VitriError> {
    from_construction(
        crate::decompose::vtree_from_elimination(
            formula,
            name,
            incidence,
            parsed.param.jw_sample(),
            parsed.param.seed(),
            request,
        ),
        parsed,
    )
}

/// goatd TD specs: `goatd-primal` and `goatd-incidence`, one schedule run on
/// the graph view the base names.
///
/// `seed` picks the RNG seed for elimination tie-breaking and refinement
/// sampling (default 0), so a caller can race several seeds on one formula.
/// `refine=off` runs the unrefined single-slot schedule instead of the refined
/// one.
pub(super) fn build_vtree_goatd(
    formula: &CnfFormula,
    parsed: &ParsedSpec<'_>,
    incidence: bool,
    knobs: GoatdKnobs,
    request: ConversionRequest<'_>,
) -> Result<TdConversion, VitriError> {
    let seed = parsed.param.seed();
    let view = graph_kind(incidence);
    let built = if parsed.param.refine() {
        // This spec carries no construction budget — it runs to completion.
        // The schedule settings are the caller's, so this construction and the
        // portfolio's own goatd candidate are configured the same way.
        crate::decompose::vtree_from_goatd_refined(formula, view, seed, None, knobs, request)
    } else {
        crate::decompose::vtree_from_goatd(formula, view, seed, request)
    };
    from_construction(built, parsed)
}

/// FlowCutter TD specs: `flowcutter-{primal,incidence}[:params]`, in timed mode
/// (`budget=200ms`), step-budgeted mode (`budget=100000steps,iters=900`) or bare
/// (which the parse resolves to the timed defaults).
pub(super) fn build_vtree_flowcutter(
    formula: &CnfFormula,
    parsed: &ParsedSpec<'_>,
    request: ConversionRequest<'_>,
) -> Result<TdConversion, VitriError> {
    let kind = graph_kind(matches!(
        parsed.family,
        VtreeBase::Flowcutter { incidence: true }
    ));
    let budget = parsed.param.fc_budget(parsed.base)?;
    from_construction(
        crate::decompose::flowcutter_vtree(formula, kind, budget, request),
        parsed,
    )
}

/// The `guided-bisect` spec: recursive primal-graph bisection with a FlowCutter
/// incidence decomposition offered at every level.
///
/// The decomposition is built here and handed to the one construction the
/// portfolio candidate of the same name also reaches, so the two cannot drift.
pub(super) fn build_vtree_guided_bisect(
    formula: &CnfFormula,
    parsed: &ParsedSpec<'_>,
    request: ConversionRequest<'_>,
) -> Result<TdConversion, VitriError> {
    let budget = parsed.param.fc_budget(parsed.base)?;
    let built = crate::decompose::flowcutter_td(formula, GraphKind::Incidence, budget)
        .and_then(|td| crate::decompose::guided_bisect_from_incidence_td(formula, &td, request));
    from_construction(built, parsed)
}

/// The portfolio spec `portfolio`: several FlowCutter candidates plus goatd,
/// keeping the best-scoring one.
pub(super) fn build_vtree_portfolio(
    formula: &CnfFormula,
    parsed: &ParsedSpec<'_>,
    ctx: &SelectionCtx,
    limits: &BuildLimits,
) -> Result<VtreeArtifacts, VitriError> {
    // The portfolio's seed rides on `ctx` (`SelectionCtx::portfolio`, default 0)
    // — only the goatd candidate consumes it, FlowCutter seeds internally.
    crate::decompose::vtree_from_portfolio(
        formula,
        PORTFOLIO_STEPS,
        PORTFOLIO_ITERS,
        parsed.reading,
        ctx,
        limits,
    )
}

/// The graph view a base named, as the decomposers take it.
fn graph_kind(incidence: bool) -> GraphKind {
    if incidence {
        GraphKind::Incidence
    } else {
        GraphKind::Primal
    }
}

/// Computation-step budget handed to each portfolio candidate's FlowCutter run.
const PORTFOLIO_STEPS: i64 = 150_000;
/// FlowCutter iterations per portfolio candidate.
const PORTFOLIO_ITERS: i32 = 15;
