//! Beside the module: these reach the portfolio's private run state and its
//! candidate catalog.

mod driver;
mod fold;
mod ranker;
mod skip;

use crate::candidates::CandidateRankMetric;
use crate::cnf::CnfFormula;
use crate::decompose::portfolio::catalog::Inputs;
use crate::decompose::{BuildLimits, Reading, SelectionCtx};

/// What one portfolio build is handed: the formula, the knobs the run was
/// configured with, and the limits it runs under. `flowcutter_cap_ms` is the
/// one field neither of those carries, so a test gating on the cap passes it.
///
/// The driver assembles the same struct from the same three sources, so a test
/// that built its own would be pinning a build no run performs.
fn inputs<'a>(
    formula: &'a CnfFormula,
    ctx: &SelectionCtx,
    limits: &BuildLimits,
    flowcutter_cap_ms: Option<i64>,
) -> Inputs<'a> {
    Inputs {
        formula,
        source_profile: None,
        seed: ctx.portfolio.seed,
        peak_mode: false,
        show_mask: None,
        trace: false,
        flowcutter_cap_ms,
        t_build: std::time::Instant::now(),
        deadline: None,
        candidate_capacity: limits.candidates,
        peak_tolerance: ctx.portfolio.peak_tolerance,
        goatd: ctx.goatd,
        rank_metric: CandidateRankMetric::Cost,
        effort_scale: crate::budget::vtree_effort_scale(limits.budget_ms),
        reading: Reading::default(),
        conversion_trace: false,
        prefer: None,
        score_agg: None,
    }
}
