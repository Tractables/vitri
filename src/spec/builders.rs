//! Vtree construction backends — one function per decomposer family.
//!
//! Each takes the already-parsed spec: none of them reads the spec string
//! itself, so the grammar has exactly one reader ([`super::parse_vtree_spec`]).

use crate::cnf::CnfFormula;
use crate::decompose::{
    BuildLimits, Conversion, GoatdKnobs, GraphKind, SelectionCtx, TdConversion, TdToVtreeConfig,
};
use crate::error::{VitriError, from_construction};

use super::{ParsedSpec, SpecParam, VtreeArtifacts, VtreeBase};

/// The `--vtree` name of a FlowCutter construction over `kind`.
///
/// A failure is reported under this name, so what a caller reads back is a spec
/// they could write themselves rather than an internal label.
fn flowcutter_spec_name(kind: GraphKind) -> &'static str {
    match kind {
        GraphKind::Incidence => "flowcutter-incidence",
        GraphKind::Primal => "flowcutter-primal",
    }
}

/// Which conversion a timed FlowCutter spec asks for.
///
/// The one place the grammar's FlowCutter parameters become a [`Conversion`];
/// [`crate::decompose::flowcutter_vtree`] runs whatever comes out.
fn fc_timed_conversion<'a>(kind: GraphKind, parsed: &'a ParsedSpec<'_>) -> Conversion<'a> {
    match (kind, parsed.use_best) {
        (_, true) => Conversion::Best,
        // Default: try two orderings from the same TD and keep the vtree with
        // the lower cost score. The second minimizes clause-spanning but is
        // sometimes worse, so this hedges by keeping both. A spec that named an
        // item ordering has already said what it wants, and gets it directly.
        (GraphKind::Incidence, false) => {
            if parsed.td_config != TdToVtreeConfig::default() {
                Conversion::Configured(&parsed.td_config)
            } else {
                Conversion::DualOrdering
            }
        }
        (GraphKind::Primal, false) => Conversion::Configured(&parsed.td_config),
    }
}

/// The single elimination-order specs: `minfill`, `mindegree`,
/// `nested-dissection` and their siblings, each also as `<name>-inc` on the
/// incidence graph and each taking a `seed` (default 0) that the `*-sample`
/// constructions consume.
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
    effort_scale: f64,
) -> Result<TdConversion, VitriError> {
    let seed = parsed.param.seed();
    from_construction(
        crate::decompose::vtree_from_elimination(formula, name, incidence, seed, effort_scale),
        name,
    )
}

/// goatd TD specs: `goatd` (incidence graph) and `goatd-primal`.
///
/// Both accept a `seed` that picks the RNG seed for elimination
/// tie-breaking and refinement sampling (default 0), so a caller can race
/// several seeds on one formula.
pub(super) fn build_vtree_goatd(
    formula: &CnfFormula,
    parsed: &ParsedSpec<'_>,
    incidence: bool,
    knobs: GoatdKnobs,
    effort_scale: f64,
) -> Result<TdConversion, VitriError> {
    let seed = parsed.param.seed();
    if incidence {
        from_construction(
            // This spec carries no construction budget — it runs to completion.
            // The schedule settings are the caller's, so this construction and
            // the portfolio's own goatd candidate are configured the same way.
            crate::decompose::vtree_from_goatd_incidence_refined_best(
                formula,
                seed,
                None,
                knobs,
                effort_scale,
            ),
            "goatd",
        )
    } else {
        from_construction(
            crate::decompose::vtree_from_goatd_best(formula, seed, effort_scale),
            "goatd-primal",
        )
    }
}

/// FlowCutter TD specs: `flowcutter-{primal,incidence}[:params]`, in timed mode
/// (`budget=200ms`), step-budgeted mode (`budget=100000steps,iters=900`) or bare
/// (which the parse resolves to the timed defaults).
pub(super) fn build_vtree_flowcutter(
    formula: &CnfFormula,
    parsed: &ParsedSpec<'_>,
    effort_scale: f64,
) -> Result<TdConversion, VitriError> {
    let kind = if matches!(parsed.family, VtreeBase::Flowcutter { incidence: true }) {
        GraphKind::Incidence
    } else {
        GraphKind::Primal
    };
    let budget = parsed.param.fc_budget(parsed.base)?;
    // Step-budgeted mode builds from the bag assignment alone; the parse rejects
    // an item ordering written alongside it.
    let step_config = matches!(parsed.param, SpecParam::FcSteps { .. })
        .then(|| TdToVtreeConfig::from_bag_assignment(parsed.td_config.bag_assignment));
    let conversion = match &step_config {
        Some(config) => Conversion::Configured(config),
        None => fc_timed_conversion(kind, parsed),
    };
    from_construction(
        crate::decompose::flowcutter_vtree(formula, kind, budget, conversion, effort_scale),
        flowcutter_spec_name(kind),
    )
}

/// The combiner spec over a FlowCutter incidence decomposition,
/// `hybrid-flowcutter-incidence`.
///
/// It builds the incidence decomposition at the effort the `:param` names —
/// the same token, with the same defaults, that `flowcutter-incidence` takes —
/// and then hands it to the ONE implementation of the combiner. The portfolio
/// candidate of the same name calls that same implementation on the
/// decomposition it already holds, so naming the portfolio's own effort
/// (`budget=150000steps,iters=15`) reproduces that
/// candidate exactly.
pub(super) fn build_vtree_flowcutter_combiner(
    formula: &CnfFormula,
    parsed: &ParsedSpec<'_>,
    effort_scale: f64,
) -> Result<TdConversion, VitriError> {
    // Step-budgeted mode is the deterministic one (no wall clock at all), which
    // is why it can reproduce the portfolio candidate; the timed default matches
    // a bare `flowcutter-incidence`.
    let budget = parsed.param.fc_budget(parsed.base)?;
    from_construction(
        crate::decompose::flowcutter_vtree(
            formula,
            GraphKind::Incidence,
            budget,
            Conversion::Hybrid,
            effort_scale,
        ),
        "hybrid-flowcutter-incidence",
    )
}

/// The portfolio spec `portfolio`: several FlowCutter candidates plus goatd,
/// keeping the best-scoring one.
pub(super) fn build_vtree_portfolio(
    formula: &CnfFormula,
    ctx: &SelectionCtx,
    limits: &BuildLimits,
) -> Result<VtreeArtifacts, VitriError> {
    // The portfolio's seed rides on `ctx` (`SelectionCtx::portfolio`, default 0)
    // — only the goatd candidate consumes it, FlowCutter seeds internally.
    crate::decompose::vtree_from_portfolio(formula, PORTFOLIO_STEPS, PORTFOLIO_ITERS, ctx, limits)
}

/// Computation-step budget handed to each portfolio candidate's FlowCutter run.
const PORTFOLIO_STEPS: i64 = 150_000;
/// FlowCutter iterations per portfolio candidate.
const PORTFOLIO_ITERS: i32 = 15;
