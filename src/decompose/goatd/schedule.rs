//! Vitri's CNF weights and vtree objective around goatd's portfolios.

use std::time::{Duration, Instant};

use crate::cnf::CnfFormula;
use crate::score::{BUILT_FROM_THIS_FORMULA, vtree_cost};

use super::super::best::select_first_min;
use super::super::td_to_vtree::{ConversionRequest, convert_td};
use super::super::{GraphKind, TdConversion};
use super::sat_score;

const FC_SLOT_CAP_MS: u64 = 2_000;

/// Vitri-side controls for the refined goatd portfolio candidate.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GoatdKnobs {
    /// Explicit budget in milliseconds for the refined portfolio, overriding
    /// the share of the construction budget it would otherwise receive.
    pub refine_budget_ms: Option<u64>,
}

impl GoatdKnobs {
    pub(in crate::decompose) fn with_env_defaults(self) -> Result<Self, crate::error::VitriError> {
        Ok(Self {
            refine_budget_ms: refine_budget_ms(
                crate::env::env_raw("VITRI_GOATD_REFINE_BUDGET_MS", REFINE_BUDGET_FORM)?.as_deref(),
            )?
            .or(self.refine_budget_ms),
        })
    }
}

pub(crate) fn vtree_from_goatd(
    formula: &CnfFormula,
    view: GraphKind,
    seed: u64,
    request: ConversionRequest<'_>,
) -> Result<TdConversion, String> {
    let pace = view.build(formula);
    let graph = pace.as_goatd();
    let weights = sat_score::compute_weight(formula, pace.num_vertices());
    let mut config = ::goatd::portfolio::PortfolioConfig::sampled_min_fill();
    if view == GraphKind::Primal {
        config = config.with_flowcutter(Duration::from_millis(FC_SLOT_CAP_MS));
    }
    let candidates = ::goatd::portfolio::sampled_min_fill_candidates(graph, &weights, seed, config)
        .map_err(|error| error.to_string())?;

    let best = select_first_min(
        candidates.into_iter().map(|td| {
            let width = td.treewidth();
            let total_bag_size = td.total_bag_size();
            let built = convert_td(formula, &td, request);
            let cost = vtree_cost(&built.vtree, formula).expect(BUILT_FROM_THIS_FORMULA);
            (built, (u64::from(width), cost, total_bag_size as u64))
        }),
        |(_, key)| *key,
    )
    .map(|(built, _)| built);
    Ok(best.expect("goatd's first portfolio candidate always produces a decomposition"))
}

pub(crate) fn vtree_from_goatd_refined(
    formula: &CnfFormula,
    view: GraphKind,
    seed: u64,
    caller_budget_ms: Option<u64>,
    knobs: GoatdKnobs,
    request: ConversionRequest<'_>,
) -> Result<TdConversion, String> {
    let pace = view.build(formula);
    let graph = pace.as_goatd();
    let weights = sat_score::compute_weight(formula, pace.num_vertices());
    let budget_ms = knobs.refine_budget_ms.or(caller_budget_ms);
    let started = crate::decompose::meter::now();
    let deadline = budget_ms.map(|milliseconds| started + Duration::from_millis(milliseconds));
    let mut config = ::goatd::portfolio::PortfolioConfig::standard();
    if let Some(milliseconds) = budget_ms {
        config = config.with_soft_budget(Duration::from_millis(milliseconds));
    }
    let td = ::goatd::portfolio::decompose(graph, &weights, seed, config)
        .map_err(|error| error.to_string())?;
    let remaining =
        deadline.map(|limit| limit.saturating_duration_since(crate::decompose::meter::now()));
    let td = ::goatd::decomposition::refine_with_flowcutter(td, graph, remaining)
        .map_err(|error| error.to_string())?;
    Ok(convert_td(
        formula,
        &td,
        ConversionRequest {
            deadline: earliest(request.deadline, deadline),
            ..request
        },
    ))
}

fn earliest(left: Option<Instant>, right: Option<Instant>) -> Option<Instant> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (only, None) | (None, only) => only,
    }
}

const REFINE_BUDGET_FORM: &str = "milliseconds of budget for the goatd refine \
     schedule (0 = take the caller's share instead)";

fn refine_budget_ms(value: Option<&str>) -> Result<Option<u64>, crate::error::VitriError> {
    let milliseconds = crate::env::parse_value(
        "VITRI_GOATD_REFINE_BUDGET_MS",
        value,
        0u64,
        REFINE_BUDGET_FORM,
    )?;
    Ok((milliseconds > 0).then_some(milliseconds))
}
