//! Vitri's CNF weights and vtree objective around goatd's portfolios.

use std::time::{Duration, Instant};

use ::goatd::portfolio::{
    CandidateOrigin, CandidateOutcome, CandidateTrace, Pass, PortfolioConfig, SamplingPatience,
};

use crate::cnf::CnfFormula;
use crate::diagnostics::diag;
use crate::score::{BUILT_FROM_THIS_FORMULA, vtree_cost};

use super::super::best::select_first_min;
use super::super::td_to_vtree::{ConversionRequest, convert_td};
use super::super::{GraphKind, TdConversion};
use super::polishing::{GoatdLift, GoatdPolishing};
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
    /// The caller's construction deadline still bounds this allocation.
    /// A budget also enables goatd's deadline-dependent improvement stages,
    /// so a generous budget can produce different trees from an unbounded run.
    /// Search and refinement share the first half; the remaining half is
    /// reserved for converting the retained decompositions into vtrees.
    ///
    /// [`SelectionCtx::with_env_defaults`](crate::decompose::SelectionCtx::with_env_defaults)
    /// preserves this value when the environment variable is unset. An explicit
    /// `VITRI_GOATD_REFINE_BUDGET_MS=0` clears it to use the caller's allocation.
    pub refine_budget_ms: Option<u64>,
    /// Enable final vertex reinsertion and FlowCutter refinement of the winner.
    /// Enabled by default; disabling it retains the standard candidate
    /// generators and initial triangulation refinement. Both settings use the
    /// same construction allocation and reserve time for vtree conversion.
    /// `VITRI_GOATD_FINAL_POLISHING` overrides this through
    /// [`SelectionCtx::with_env_defaults`](crate::decompose::SelectionCtx::with_env_defaults).
    pub final_polishing: bool,
    /// Optional detailed final-polishing policy. Requires `final_polishing`.
    /// `None` uses the existing pair of final passes.
    pub polishing: Option<GoatdPolishing>,
    /// Enable projection-and-lift for bipartite graph views. `None` keeps the
    /// standard schedule's setting (disabled).
    pub bipartite_lift: Option<GoatdLift>,
    /// How many of the schedule's decompositions the refined construction
    /// converts and offers (`VITRI_GOATD_CANDIDATES`), in goatd's order of
    /// width and then total bag size. With `final_polishing` enabled, the
    /// winner receives additional FlowCutter refinement; the runners-up do not.
    /// Each is converted while the budget holds, and the caller ranks them
    /// against every other tree it has. The default
    /// is 4. Accepted counts are 1 through 8; other values return a configuration
    /// error. Vtree ranking is independent of goatd's decomposition ordering.
    pub candidates: u32,
}

impl Default for GoatdKnobs {
    /// No explicit budget, the winner and its first three runners-up.
    fn default() -> Self {
        Self {
            refine_budget_ms: None,
            final_polishing: true,
            polishing: None,
            bipartite_lift: None,
            candidates: 4,
        }
    }
}

impl GoatdKnobs {
    pub(crate) fn validate(self) -> Result<(), crate::error::VitriError> {
        if !self.final_polishing && self.polishing.is_some() {
            return Err(crate::error::VitriError::config(
                "goatd.polishing requires goatd.final_polishing",
            ));
        }
        if let Some(polishing) = self.polishing {
            polishing.validate()?;
        }
        validate_candidate_count(self.candidates).map_err(|reason| {
            crate::error::VitriError::config(format!("goatd.candidates {reason}"))
        })
    }

    pub(in crate::decompose) fn with_env_defaults(self) -> Result<Self, crate::error::VitriError> {
        Ok(Self {
            polishing: self.polishing,
            bipartite_lift: self.bipartite_lift,
            final_polishing: crate::env::env_flag_or(
                "VITRI_GOATD_FINAL_POLISHING",
                self.final_polishing,
            )?,
            refine_budget_ms: refine_budget_ms(
                crate::env::env_raw("VITRI_GOATD_REFINE_BUDGET_MS", REFINE_BUDGET_FORM)?.as_deref(),
                self.refine_budget_ms,
            )?,
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
    drop(weights);
    drop(pace);

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

/// Stop sampled restarts after a stall while preserving a minimum search.
/// The remaining search allocation is available to goatd's final improvements.
const SAMPLING_PATIENCE: SamplingPatience = SamplingPatience::Halving { min_restarts: 200 };

/// Stop launching candidates halfway through the search allocation. Its hard
/// bound also covers final reinsertion; refinement gets any time left before
/// that bound. Unbudgeted search runs to the schedule's end, with stall stops.
fn portfolio_config(budget: Option<Duration>, final_polishing: bool) -> PortfolioConfig {
    let mut config = PortfolioConfig::standard().with_sampling_patience(SAMPLING_PATIENCE);
    if !final_polishing {
        config = config.without_vertex_reinsertion();
    }
    match budget {
        Some(budget) => config.with_soft_budget(budget / 2).with_hard_budget(budget),
        None => config,
    }
}

/// The trace line for a stop goatd's schedule reported: where the restarts
/// gave up, or how the trailing FlowCutter candidate was bounded. Every other
/// record is a candidate, which the `[goatd-cand]` lines already list.
fn stop_line(spec: &str, record: &CandidateTrace) -> Option<String> {
    match record.outcome {
        CandidateOutcome::SamplingStopped {
            restarts,
            last_improvement,
            left,
        } => Some(format!(
            "[goatd-stop] {spec} restarts={restarts} last_improvement={} left_ms={} at_ms={}",
            last_improvement.map_or_else(|| "none".to_string(), |index| index.to_string()),
            left.map_or_else(|| "none".to_string(), |left| left.as_millis().to_string()),
            record.elapsed.as_millis(),
        )),
        CandidateOutcome::TailBounded {
            window,
            patience,
            spent,
        } => Some(format!(
            "[goatd-tail] {spec} window_ms={} patience_ms={} spent_ms={} at_ms={}",
            window.as_millis(),
            patience.as_millis(),
            spent.as_millis(),
            record.elapsed.as_millis(),
        )),
        _ => None,
    }
}

/// The refined construction's trees, best first: goatd's winner, optionally
/// polished by FlowCutter, then up to `knobs.candidates - 1` further
/// decompositions of the same run in goatd's order. Never empty. The tree at index `i`
/// is what the spec with [`candidate_param`]`(i)` rebuilds.
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
    knobs.validate().map_err(|error| error.to_string())?;
    let started = crate::decompose::meter::now();
    let budget_ms = knobs.refine_budget_ms.or(caller_budget_ms);
    let deadline = earliest(
        request.deadline,
        budget_ms.and_then(|milliseconds| started.checked_add(Duration::from_millis(milliseconds))),
    );
    let pace = view.build(formula);
    let graph = pace.as_goatd();
    let weights = sat_score::compute_weight(formula, pace.num_vertices());
    let search_started = crate::decompose::meter::now();
    let search_budget = deadline.map(|limit| limit.saturating_duration_since(search_started) / 2);
    let search_deadline = search_budget.map(|budget| search_started + budget);
    let polishing = knobs.polishing.unwrap_or(GoatdPolishing::legacy(
        knobs.final_polishing,
        knobs.final_polishing,
    ));
    let mut config = portfolio_config(search_budget, polishing.reinsertion());
    if let Some(lift) = knobs.bipartite_lift {
        config = lift.apply(config);
    }
    let spec = request.spec.unwrap_or("goatd");
    let real = Instant::now();
    // goatd measures each candidate's shape for a traced run only, a pass over
    // its bags apiece. A conversion with no trace has nowhere to report that or
    // the origins, so it asks for the plain list.
    let (mut candidates, mut origins): (Vec<_>, Vec<CandidateOrigin>) = if trace {
        let mut on_record = |record: CandidateTrace| {
            if let Some(line) = stop_line(spec, &record) {
                diag!("{line}");
            }
        };
        ::goatd::portfolio::candidates_traced(graph, &weights, seed, config, &mut on_record)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|candidate| (candidate.decomposition, candidate.origin))
            .unzip()
    } else {
        let trees = ::goatd::portfolio::candidates(graph, &weights, seed, config)
            .map_err(|error| error.to_string())?;
        (trees, Vec::new())
    };
    let decompose_ms = real.elapsed().as_millis();
    drop(weights);
    let found = candidates.len();
    let keep = knobs.candidates.min(MAX_GOATD_CANDIDATES) as usize;
    candidates.truncate(keep);
    origins.truncate(keep);
    // Empty unless the run is traced, so this reports what was kept or nothing.
    for (index, (origin, decomposition)) in origins.iter().zip(&candidates).enumerate() {
        diag!(
            "[goatd-cand] {spec} index={index} stage={} seed={} pass={} width={} bags={} \
             total_bag_size={}",
            origin.stage,
            origin.seed,
            pass_name(origin.pass),
            decomposition.treewidth(),
            decomposition.bags().len(),
            decomposition.total_bag_size(),
        );
    }
    let mut candidates = candidates.into_iter();
    let first = candidates
        .next()
        .expect("goatd's first portfolio candidate always produces a decomposition");
    let raw = (
        first.treewidth(),
        first.bags().len(),
        first.total_bag_size(),
    );
    let remaining = search_deadline
        .map(|limit| limit.saturating_duration_since(crate::decompose::meter::now()));
    let first = if polishing.separator() {
        ::goatd::decomposition::refine_with_flowcutter(first, graph, remaining)
            .map_err(|error| error.to_string())?
    } else {
        first
    };
    if trace {
        diag!(
            "[goatd] {spec} vertices={} budget_ms={} decompose_ms={decompose_ms} refine_ms={} \
             width={} bags={} total_bag_size={} refined_width={} refined_bags={} \
             refined_total_bag_size={} found={found} final_polishing={}",
            pace.num_vertices(),
            budget_ms.map_or_else(|| "-".to_string(), |b| b.to_string()),
            real.elapsed().as_millis() - decompose_ms,
            raw.0,
            raw.1,
            raw.2,
            first.treewidth(),
            first.bags().len(),
            first.total_bag_size(),
            knobs.final_polishing,
        );
    }
    let request = ConversionRequest {
        deadline,
        ..request
    };
    let baseline = convert_td(formula, &first, request);
    let first = if polishing.is_adaptive() {
        polishing.refine(graph, first, baseline, formula, request, trace)?
    } else {
        baseline
    };
    drop(pace);
    let mut built = vec![first];
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

fn refine_budget_ms(
    value: Option<&str>,
    default: Option<u64>,
) -> Result<Option<u64>, crate::error::VitriError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let milliseconds = crate::env::parse_value(
        "VITRI_GOATD_REFINE_BUDGET_MS",
        Some(value),
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
    validate_candidate_count(count).map_err(|reason| match value {
        Some(_) => crate::error::VitriError::env("VITRI_GOATD_CANDIDATES", reason),
        None => crate::error::VitriError::config(format!("goatd.candidates {reason}")),
    })?;
    Ok(count)
}

fn validate_candidate_count(count: u32) -> Result<(), String> {
    if !(1..=MAX_GOATD_CANDIDATES).contains(&count) {
        return Err(format!(
            "must be from 1 to {MAX_GOATD_CANDIDATES}; got {count}; to build no goatd \
             tree at all, name the entry in VITRI_PORTFOLIO_SKIP"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
