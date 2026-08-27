//! Vitri's elimination-order names and CNF graph adapter.

use std::time::Duration;

use crate::cnf::CnfFormula;

use super::super::td_to_vtree::{ConversionRequest, convert_td};
use super::super::{GraphKind, TdConversion, TreeDecomposition};
use super::sat_score;

const GOATD_ELIMINATION_SOFT_MS: u64 = 10_000;
const MINFILL_ORDER: &str = "minfill";
const ELIMINATION_ORDERS: &[(&str, bool)] = &[
    (MINFILL_ORDER, true),
    ("mindegree", true),
    ("nested-dissection", false),
];

pub(crate) const MINFILL_SPEC: &str = "minfill-primal";
pub(crate) const VIEW_SUFFIXES: [(&str, bool); 2] = [("-primal", false), ("-incidence", true)];
pub(crate) const INTERNAL_ELIMINATION_SEED: u64 = 0;

pub(crate) fn elimination_spec_names() -> impl Iterator<Item = &'static str> {
    ELIMINATION_ORDERS.iter().map(|(name, _)| *name)
}

pub(crate) fn elimination_order_samples(name: &str) -> bool {
    ELIMINATION_ORDERS
        .iter()
        .any(|&(candidate, samples)| candidate == name && samples)
}

pub(crate) fn elimination_spec(base: &str) -> Option<(&'static str, bool)> {
    let (order, incidence) = VIEW_SUFFIXES
        .iter()
        .find_map(|(suffix, incidence)| Some((base.strip_suffix(suffix)?, *incidence)))?;
    let name = elimination_spec_names().find(|name| *name == order)?;
    Some((name, incidence))
}

fn order<'a>(
    name: &str,
    sampled: bool,
    weights: &'a [u32],
) -> Result<::goatd::elimination::Order<'a>, String> {
    match (name, sampled) {
        ("minfill", false) => Ok(::goatd::elimination::Order::MinFill),
        ("minfill", true) => Ok(::goatd::elimination::Order::MinFillSampled { weights }),
        ("mindegree", false) => Ok(::goatd::elimination::Order::MinDegree),
        ("mindegree", true) => Ok(::goatd::elimination::Order::MinDegreeSampled { weights }),
        ("nested-dissection", false) => Ok(::goatd::elimination::Order::NestedDissection),
        ("nested-dissection", true) => {
            Err("nested-dissection breaks ties deterministically only".into())
        }
        _ => Err(format!("unknown elimination-order construction: {name}")),
    }
}

pub(crate) fn vtree_from_elimination(
    formula: &CnfFormula,
    name: &str,
    incidence: bool,
    jw_sample: bool,
    seed: u64,
    request: ConversionRequest<'_>,
) -> Result<TdConversion, String> {
    let view = if incidence {
        GraphKind::Incidence
    } else {
        GraphKind::Primal
    };
    let pace = view.build(formula);
    let weights = sat_score::compute_weight(formula, pace.num_vertices());
    let td = ::goatd::elimination::decompose(
        pace.as_goatd(),
        order(name, jw_sample, &weights)?,
        seed,
        Some(Duration::from_millis(GOATD_ELIMINATION_SOFT_MS)),
    )
    .map_err(|error| error.to_string())?;
    Ok(convert_td(formula, &td, request))
}

pub(crate) fn vtree_from_minfill(
    formula: &CnfFormula,
    seed: u64,
    request: ConversionRequest<'_>,
) -> Result<TdConversion, String> {
    vtree_from_elimination(formula, MINFILL_ORDER, false, false, seed, request)
}

pub(crate) fn minfill_td_from_edges(
    num_vertices: u32,
    edges: &[(u32, u32)],
    seed: u64,
) -> TreeDecomposition {
    let graph = ::goatd::Graph::new(num_vertices, edges.iter().copied());
    ::goatd::elimination::decompose(
        &graph,
        ::goatd::elimination::Order::MinFill,
        seed,
        Some(Duration::from_millis(GOATD_ELIMINATION_SOFT_MS)),
    )
    .expect("trusted graph and fixed elimination budget are valid")
}
