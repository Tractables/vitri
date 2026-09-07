//! Vitri's CNF weights and vtree objective around goatd's portfolios.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ::goatd::portfolio::{CandidateOrigin, Pass, Stage};

use crate::cnf::CnfFormula;
use crate::diagnostics::diag;
use crate::score::{BUILT_FROM_THIS_FORMULA, vtree_cost};

use super::super::best::select_first_min;
use super::super::td_to_vtree::{ConversionRequest, convert_td};
use super::super::{GraphKind, TdConversion};
use super::sat_score;

const FC_SLOT_CAP_MS: u64 = 2_000;

/// The most decompositions the refined schedule offers from one run, so that a
/// caller asking for every candidate holds a bounded number of trees.
pub(crate) const MAX_GOATD_CANDIDATES: u32 = CANDIDATE_PARAMS.len() as u32 + 1;

/// The `--vtree` parameter that rebuilds the candidate at each index past the
/// first, which the bare spec rebuilds.
const CANDIDATE_PARAMS: [&str; 7] = [
    "candidate=1",
    "candidate=2",
    "candidate=3",
    "candidate=4",
    "candidate=5",
    "candidate=6",
    "candidate=7",
];

/// The spec parameter naming the candidate at `index` in the order the
/// schedule offers them: `None` for the first, which the bare spec names.
pub(crate) fn candidate_param(index: usize) -> Option<&'static str> {
    index.checked_sub(1).map(|at| {
        *CANDIDATE_PARAMS
            .get(at)
            .expect("a construction offers at most MAX_GOATD_CANDIDATES trees")
    })
}

/// Vitri-side controls for the refined goatd portfolio candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GoatdKnobs {
    /// Explicit budget in milliseconds for the refined portfolio, overriding
    /// the share of the construction budget it would otherwise receive.
    pub refine_budget_ms: Option<u64>,
    /// How many of the schedule's decompositions the refined construction
    /// converts and offers (`VITRI_GOATD_CANDIDATES`). 1 offers the winner
    /// alone, refined; above 1 the rest follow it unrefined, drawn in turn from
    /// each kind of candidate the run produced (a stage, on the caller's weights
    /// or a hedge's, each kind in goatd's order), each
    /// converted while the budget holds, and the caller ranks them against
    /// every other tree it has.
    pub candidates: u32,
}

impl Default for GoatdKnobs {
    /// No explicit budget, the winner alone.
    fn default() -> Self {
        Self {
            refine_budget_ms: None,
            candidates: 1,
        }
    }
}

impl GoatdKnobs {
    pub(in crate::decompose) fn with_env_defaults(self) -> Result<Self, crate::error::VitriError> {
        Ok(Self {
            refine_budget_ms: refine_budget_ms(
                crate::env::env_raw("VITRI_GOATD_REFINE_BUDGET_MS", REFINE_BUDGET_FORM)?.as_deref(),
            )?
            .or(self.refine_budget_ms),
            candidates: candidate_count(
                crate::env::env_raw("VITRI_GOATD_CANDIDATES", CANDIDATES_FORM)?.as_deref(),
                self.candidates,
            )?,
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

/// goatd's standard schedule under the share of the budget this construction
/// was given: no new candidate past half the share, everything stopped at the
/// share, and the rest of the share for the refinement pass. Without a share
/// the schedule runs to its own end.
fn portfolio_config(budget: Option<Duration>) -> ::goatd::portfolio::PortfolioConfig {
    let config = ::goatd::portfolio::PortfolioConfig::standard();
    match budget {
        Some(budget) => config.with_soft_budget(budget / 2).with_hard_budget(budget),
        None => config,
    }
}

/// The refined construction's trees, best first: goatd's winner refined by
/// FlowCutter, then up to `knobs.candidates - 1` further decompositions of the
/// same run, unrefined, in the order [`varied_prefix`] gives them. Never
/// empty. The tree at index `i` is what the spec with [`candidate_param`]`(i)`
/// rebuilds.
///
/// The budget is a wall on the whole construction: conversion stops once it is
/// spent, so a short share yields fewer trees than were asked for.
pub(crate) fn vtrees_from_goatd_refined(
    formula: &CnfFormula,
    view: GraphKind,
    seed: u64,
    caller_budget_ms: Option<u64>,
    knobs: GoatdKnobs,
    trace: bool,
    request: ConversionRequest<'_>,
) -> Result<Vec<TdConversion>, String> {
    let pace = view.build(formula);
    let graph = pace.as_goatd();
    let weights = sat_score::compute_weight(formula, pace.num_vertices());
    let budget_ms = knobs.refine_budget_ms.or(caller_budget_ms);
    let started = crate::decompose::meter::now();
    let deadline = budget_ms.map(|milliseconds| started + Duration::from_millis(milliseconds));
    let config = portfolio_config(budget_ms.map(Duration::from_millis));
    let real = Instant::now();
    let candidates =
        ::goatd::portfolio::candidates_traced(graph, &weights, seed, config, &mut |_| {})
            .map_err(|error| error.to_string())?;
    let decompose_ms = real.elapsed().as_millis();
    let found = candidates.len();
    let candidates = varied_prefix(
        candidates,
        knobs.candidates.min(MAX_GOATD_CANDIDATES) as usize,
        |candidate| CandidateKind::of(candidate.origin),
    );
    let spec = request.spec.unwrap_or("goatd");
    if trace {
        for (index, candidate) in candidates.iter().enumerate() {
            let origin = candidate.origin;
            diag!(
                "[goatd-cand] {spec} index={index} stage={} seed={} pass={} width={} bags={} \
                 total_bag_size={}",
                origin.stage,
                origin.seed,
                pass_name(origin.pass),
                candidate.decomposition.treewidth(),
                candidate.decomposition.bags().len(),
                candidate.decomposition.total_bag_size(),
            );
        }
    }
    let mut candidates = candidates
        .into_iter()
        .map(|candidate| candidate.decomposition);
    let first = candidates
        .next()
        .expect("goatd's first portfolio candidate always produces a decomposition");
    let raw = (
        first.treewidth(),
        first.bags().len(),
        first.total_bag_size(),
    );
    let remaining =
        deadline.map(|limit| limit.saturating_duration_since(crate::decompose::meter::now()));
    let first = ::goatd::decomposition::refine_with_flowcutter(first, graph, remaining)
        .map_err(|error| error.to_string())?;
    if trace {
        diag!(
            "[goatd] {spec} vertices={} budget_ms={} decompose_ms={decompose_ms} refine_ms={} \
             width={} bags={} total_bag_size={} refined_width={} refined_bags={} \
             refined_total_bag_size={} found={found}",
            pace.num_vertices(),
            budget_ms.map_or_else(|| "-".to_string(), |b| b.to_string()),
            real.elapsed().as_millis() - decompose_ms,
            raw.0,
            raw.1,
            raw.2,
            first.treewidth(),
            first.bags().len(),
            first.total_bag_size(),
        );
    }
    let request = ConversionRequest {
        deadline: earliest(request.deadline, deadline),
        ..request
    };
    let mut built = vec![convert_td(formula, &first, request)];
    for (index, td) in candidates.enumerate() {
        // One tree is in hand; the rest are converted only while there is room.
        if deadline.is_some_and(|limit| crate::decompose::meter::now() >= limit) {
            break;
        }
        let label = crate::spec::spec_string(spec, candidate_param(index + 1));
        let request = ConversionRequest {
            spec: request.spec.map(|_| label.as_str()),
            ..request
        };
        built.push(convert_td(formula, &td, request));
    }
    Ok(built)
}

/// What a candidate of goatd's schedule is, for the purpose of offering
/// different kinds: its stage, and whether it ran on the weights of one of the
/// hedge's stages rather than the caller's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CandidateKind {
    stage: Stage,
    hedged: bool,
}

impl CandidateKind {
    fn of(origin: CandidateOrigin) -> Self {
        Self {
            stage: origin.stage,
            hedged: matches!(origin.pass, Pass::Modified { .. }),
        }
    }
}

/// At most `count` of goatd's candidates, which come sorted by width and then
/// total bag size: the first, goatd's own pick, then the rest drawn in turn
/// from each kind of candidate in the run, each kind in goatd's order and the
/// kinds in the order they first appear. A run whose hedge made eight passes
/// of one stage at the pick's width would otherwise fill every slot with them
/// and leave out its sampled restarts, which is where the trees that compile
/// when the pick does not come from. Fewer than `count` are returned as they
/// came.
fn varied_prefix<T>(
    candidates: Vec<T>,
    count: usize,
    kind: impl Fn(&T) -> CandidateKind,
) -> Vec<T> {
    if candidates.len() <= count {
        return candidates;
    }
    let mut candidates = candidates.into_iter();
    let mut taken: Vec<T> = candidates.next().into_iter().collect();
    let mut kinds: Vec<(CandidateKind, VecDeque<T>)> = Vec::new();
    for candidate in candidates {
        let of = kind(&candidate);
        match kinds.iter_mut().find(|(known, _)| *known == of) {
            Some((_, members)) => members.push_back(candidate),
            None => kinds.push((of, VecDeque::from([candidate]))),
        }
    }
    while taken.len() < count {
        let mut drew = false;
        for (_, members) in &mut kinds {
            if taken.len() == count {
                break;
            }
            if let Some(candidate) = members.pop_front() {
                taken.push(candidate);
                drew = true;
            }
        }
        if !drew {
            break;
        }
    }
    taken
}

/// One token per pass of goatd's hedged schedule, for the trace line.
fn pass_name(pass: Pass) -> String {
    match pass {
        Pass::Only => "only".to_string(),
        Pass::Plain => "plain".to_string(),
        Pass::Modified { index } => format!("modified:{index}"),
    }
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

const CANDIDATES_FORM: &str = "how many of goatd's decompositions to convert, \
     from 1 to 8";

/// `VITRI_GOATD_CANDIDATES`: at least 1, since a construction that converts
/// nothing has no tree to offer, and at most [`MAX_GOATD_CANDIDATES`].
fn candidate_count(value: Option<&str>, default: u32) -> Result<u32, crate::error::VitriError> {
    let count = crate::env::parse_value("VITRI_GOATD_CANDIDATES", value, default, CANDIDATES_FORM)?;
    if !(1..=MAX_GOATD_CANDIDATES).contains(&count) {
        let got = value.unwrap_or_default();
        return Err(crate::error::VitriError::env(
            "VITRI_GOATD_CANDIDATES",
            format!(
                "must be from 1 to {MAX_GOATD_CANDIDATES}; got {got:?}; to build no goatd \
                 tree at all, name the entry in VITRI_PORTFOLIO_SKIP"
            ),
        ));
    }
    Ok(count)
}

#[cfg(test)]
mod tests;
