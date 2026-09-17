//! Vitri's elimination-order names and CNF graph adapter.

use std::time::Duration;

use crate::cnf::CnfFormula;

use super::super::td_to_vtree::{ConversionRequest, convert_td};
use super::super::{GraphKind, TdConversion, TreeDecomposition};
use super::sat_score;

/// The ceiling on one goatd elimination pass. A ceiling rather than a spend: a
/// construction whose deadline falls sooner stops at the deadline, and one with
/// no deadline spends up to this.
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

/// The elimination orders a spec can name, without their view suffix.
pub(crate) fn elimination_spec_names() -> impl Iterator<Item = &'static str> {
    ELIMINATION_ORDERS.iter().map(|(name, _)| *name)
}

/// Whether `name` is an order whose ties can be broken by sampling, which is
/// what decides if a spec naming it may write `ties=jw-sample`.
pub(crate) fn elimination_order_samples(name: &str) -> bool {
    ELIMINATION_ORDERS
        .iter()
        .any(|&(candidate, samples)| candidate == name && samples)
}

/// The order and graph view a spec base names, or `None` when it names no
/// elimination construction. THE one reader of the view suffix, so every
/// elimination spec is split the same way.
pub(crate) fn elimination_spec(base: &str) -> Option<(&'static str, bool)> {
    let (order, incidence) = VIEW_SUFFIXES
        .iter()
        .find_map(|(suffix, incidence)| Some((base.strip_suffix(suffix)?, *incidence)))?;
    let name = elimination_spec_names().find(|name| *name == order)?;
    Some((name, incidence))
}

/// What one elimination pass may spend: [`GOATD_ELIMINATION_SOFT_MS`], or
/// whatever is left of the construction deadline when that is less. A deadline
/// already spent leaves [`crate::budget::LAST_ATTEMPT_MS`], the one attempt a
/// construction here gets rather than answering with no vtree.
fn elimination_budget(deadline: Option<std::time::Instant>) -> Duration {
    let left = crate::budget::clamp(Duration::from_millis(GOATD_ELIMINATION_SOFT_MS), deadline);
    if left.is_zero() {
        return Duration::from_millis(crate::budget::LAST_ATTEMPT_MS);
    }
    left
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

/// Eliminate `formula`'s graph view in the named order and convert the
/// decomposition that falls out of it.
///
/// One pass, unrefined and unscheduled: the goatd portfolio candidates run the
/// scheduled construction in `super::schedule` instead. `jw_sample` breaks ties
/// by the SAT-aware weights in `super::sat_score` rather than deterministically.
///
/// # Errors
///
/// The elimination's own message when goatd refuses the graph or the order, and
/// [`CONSTRUCTION_TIMED_OUT`](crate::decompose::CONSTRUCTION_TIMED_OUT) when the
/// request's deadline has already passed.
pub(crate) fn vtree_from_elimination(
    formula: &CnfFormula,
    name: &str,
    incidence: bool,
    jw_sample: bool,
    seed: u64,
    request: ConversionRequest<'_>,
) -> Result<TdConversion, String> {
    let budget = elimination_budget(request.deadline);
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
        Some(budget),
    )
    .map_err(|error| error.to_string())?;
    Ok(convert_td(formula, &td, request))
}

/// [`vtree_from_elimination`] at min-fill on the primal graph, deterministic:
/// the construction a caller that names no spec at all falls back to.
///
/// # Errors
///
/// As [`vtree_from_elimination`].
pub(crate) fn vtree_from_minfill(
    formula: &CnfFormula,
    seed: u64,
    request: ConversionRequest<'_>,
) -> Result<TdConversion, String> {
    vtree_from_elimination(formula, MINFILL_ORDER, false, false, seed, request)
}

/// A min-fill decomposition of a graph a caller already holds as edges, for
/// the constructions that decompose something other than a view of the whole
/// formula.
///
/// Unlike [`vtree_from_elimination`] this takes no deadline: it is called on
/// subproblems small enough that the elimination ceiling above is not reached,
/// and goatd cannot refuse a graph built here.
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

#[cfg(test)]
mod tests;
