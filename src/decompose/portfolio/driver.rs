//! The portfolio driver: run the catalog, select a winner, publish the result.
//!
//! A construction budget that leaves nothing built is a hard error: no
//! fallback stands between an exhausted budget and
//! `Err(VitriError::construction(..))`.
//!
//! **Determinism:** [`vtree_from_portfolio`] is fully deterministic for the
//! same input formula. FlowCutter uses a fixed step budget with a fixed RNG
//! seed in the C++ backend, the multilevel bisection uses a seeded RNG and
//! sorted (BTreeMap) edge accumulation, and the cost-score comparison is a
//! pure numeric comparison.
//!
//! The one exception is the wall-clock safety net (`BuildLimits::deadline`): a
//! build that overruns its budget skips what is left and tightens the FlowCutter
//! searches behind it, which is load-dependent. Under a deadline every entry is
//! also bounded at the time left when it starts, but that bound is a
//! [`WallCapMode::BoundOnly`](crate::decompose::WallCapMode) one — the search it
//! runs is the unbounded one, and the wall only stops it once it has genuinely
//! passed — so determinism holds for every instance that finishes construction
//! within its budget.

use crate::candidates::CandidateSet;
use crate::cnf::CnfFormula;
use crate::decompose::{BuildLimits, SelectionCtx, SelectionObjective, TraceLevel};
use crate::diagnostics::diag;
use crate::error::VitriError;
use crate::score::VtreeScores;
use crate::spec::{SelectionRecord, VtreeArtifacts};
use std::sync::Arc;

use super::catalog::{
    AdoptRule, CatalogEntry, Derived, Gate, Incumbent, Inputs, PORTFOLIO_HEAVY_MAX_VARS, RunState,
    ScoredCandidate, TraceRow, build_fc_inc, build_fc_pri, build_goatd, build_hybrid,
    build_hypergraph_bisect, candidate_spec, gate_goatd, gate_hybrid, gate_hypergraph_bisect,
    outspent,
};

/// What a portfolio build in this process last cost, in ms; `0` until one has
/// finished.
///
/// Read only through [`last_build_ms`], and only by the entry gate below, which
/// compares it against the time left. Without a construction deadline there is
/// no time left to compare against, so a run that passes no budget never
/// consults it.
static LAST_BUILD_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Records the build's wall on every exit, including an unwind: a build that
/// died partway still measured what a build here costs.
struct MeasureBuild(std::time::Instant);

impl Drop for MeasureBuild {
    fn drop(&mut self) {
        LAST_BUILD_MS.store(
            self.0.elapsed().as_millis() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

/// The last measured build wall, or `None` before the first build of the
/// process finishes.
fn last_build_ms() -> Option<u64> {
    match LAST_BUILD_MS.load(std::sync::atomic::Ordering::Relaxed) {
        0 => None,
        ms => Some(ms),
    }
}

/// Blended projected selection over the collected candidates: peak ∃-forget-
/// frontier width is the primary cost, but candidates whose peak is within a
/// relative tolerance band of the minimum are a frontier tie, decided by
/// clause-load stddev.
pub(super) fn select_peak_band(cands: &[ScoredCandidate], rel_tol: f64) -> &ScoredCandidate {
    let min_peak = cands.iter().map(|c| c.sel_metric).fold(f64::MAX, f64::min);
    let band = min_peak * (1.0 + rel_tol);
    cands
        .iter()
        .filter(|c| c.sel_metric <= band)
        .min_by(|a, b| {
            a.stats
                .clause_load_stddev
                .partial_cmp(&b.stats.clause_load_stddev)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .expect("min_peak member is always within band")
}

/// The ordered catalog every portfolio build walks.
///
/// THE ORDER DECIDES TIES: peak-mode selection does a stable `min_by` over the
/// retained candidates, so this order breaks them.
///
/// Each entry spells the `--vtree` spec that builds it alone — the name a
/// win publishes plus the parameter it was built with — so every name this
/// crate can publish as a winner is one the same crate can be asked for.
/// `every_catalog_candidate_names_a_spec_that_rebuilds_it` holds that.
pub(super) fn catalog() -> Vec<CatalogEntry> {
    vec![
        CatalogEntry {
            name: "flowcutter-incidence",
            param: None,
            td_based: true,
            gate: Gate::Always,
            build: build_fc_inc,
            adopt: AdoptRule::MinStddev,
        },
        CatalogEntry {
            name: "flowcutter-primal",
            param: None,
            td_based: true,
            gate: Gate::Always,
            build: build_fc_pri,
            adopt: AdoptRule::MinStddev,
        },
        CatalogEntry {
            name: "goatd-incidence",
            param: None,
            td_based: true,
            gate: Gate::FromInputs(gate_goatd),
            build: build_goatd,
            adopt: AdoptRule::MinStddev,
        },
        // The imbalance is spelled out: this family is built at a relaxed
        // imbalance, not at the balanced default a bare `hypergraph-bisect`
        // spec means, so the name alone would not reproduce the tree that won.
        CatalogEntry {
            name: "hypergraph-bisect",
            param: Some("imbalance=0.40"),
            td_based: false,
            gate: Gate::FromDerived(gate_hypergraph_bisect),
            build: build_hypergraph_bisect,
            adopt: AdoptRule::ColoringGated,
        },
        // The assembly rule is spelled out: this entry reads the same incidence
        // decomposition as the first, and assembles it the other way.
        CatalogEntry {
            name: "flowcutter-incidence",
            param: Some("assembly=hybrid"),
            td_based: false,
            gate: Gate::FromDerived(gate_hybrid),
            build: build_hybrid,
            adopt: AdoptRule::JointStddevCost,
        },
    ]
}

/// FlowCutter incidence + primal, goatd, plus the structure-gated
/// hypergraph-bisect and hybrid-flowcutter-incidence bisection candidates.
/// Selection picks the best candidate by clause-load stddev (plain MC) or by
/// peak context width (projected mode).
///
/// A "separator" candidate was removed deliberately: every apparent win it
/// scored came with a much larger realized diagram. Do not restore it.
pub(crate) fn vtree_from_portfolio(
    formula: &CnfFormula,
    steps: i64,
    iters: i32,
    ctx: &SelectionCtx,
    limits: &BuildLimits,
) -> Result<VtreeArtifacts, VitriError> {
    let _measured = MeasureBuild(std::time::Instant::now());
    let seed = ctx.portfolio.seed;
    let num_vars = formula.num_vars;
    if num_vars == 0 {
        return Err(VitriError::construction(
            "portfolio",
            crate::decompose::EMPTY_FORMULA,
        ));
    }

    // FlowCutter effort scales with the timeout: steps and iters each scale by
    // √eff so total work grows linearly with eff.
    let effort_scale = crate::budget::vtree_effort_scale(limits.budget_ms);
    let fc_steps_eff = effort_scale.sqrt();
    let fc_iters_eff = effort_scale.sqrt();
    let reduced_steps = (if num_vars <= 2000 {
        steps.min(200_000)
    } else {
        steps.min(50_000)
    } as f64
        * fc_steps_eff) as i64;
    let iters = ((iters as f64) * fc_iters_eff).round().max(1.0) as i32;

    let peak_mode = ctx.objective.is_peak();
    let trace = ctx.portfolio.trace != TraceLevel::Off;
    // `all`: additionally build+score the hypergraph-bisect family at every
    // imbalance point, purely for the trace; never touches `best_*`.
    let trace_all = ctx.portfolio.trace == TraceLevel::All;

    // Bounds `flowcutter-primal` (see `RunState::fc_time_cap_ms`) only under
    // projected selection on large components (`num_vars` over 2000, where
    // `reduced_steps` already jumps to 50k); uncapped everywhere else.
    let flowcutter_cap_ms = if peak_mode && num_vars > 2000 {
        ctx.portfolio.flowcutter_cap_ms
    } else {
        None
    };
    let t_build = std::time::Instant::now();

    // The metric THIS run ranks by, fixed before anything is built so that
    // deferred selection and the exported candidate order are the same
    // preference and cannot come apart. Without a show mask the show peak does
    // not exist, so the peak metric reads the all-variable one.
    let rank_metric = match ctx.objective {
        SelectionObjective::ClauseBalance => {
            crate::candidates::CandidateRankMetric::ClauseLoadStddev
        }
        SelectionObjective::PeakWidthShow(_) => {
            crate::candidates::CandidateRankMetric::PeakContextWidthShow
        }
        SelectionObjective::PeakWidthAll => {
            crate::candidates::CandidateRankMetric::PeakContextWidthAll
        }
    };

    let inp = Inputs {
        formula,
        seed,
        peak_mode,
        show_mask: ctx.objective.show_mask(),
        trace,
        flowcutter_cap_ms,
        t_build,
        deadline: limits.deadline,
        candidate_capacity: limits.candidates,
        peak_tolerance: ctx.portfolio.peak_tolerance,
        goatd: ctx.goatd,
        effort_scale,
        rank_metric,
    };

    let mut run = RunState::new(reduced_steps, iters);

    // The construction budget this call was admitted under, read once before
    // anything is built, from the same source the loop below reads. `None` = no
    // deadline. The wall report at the end compares the spent wall against it.
    let entry_budget_ms = inp.remaining_ms();

    let mut derived: Option<Derived> = None;

    let catalog = catalog();

    // A build with less room than the last one in this process measured enters
    // the capped regime at once, rather than discovering it one candidate too
    // late. The behind-schedule latch trips only after some candidate has
    // already overspent, so on a build that is short from the start it arms too
    // late to bound the candidate that spends the room.
    //
    // Both values are read once, here, and the message below prints those same
    // locals: a message that re-read the clock would report a state the
    // condition was never evaluated on.
    let left_ms = inp.remaining_ms();
    let measured = last_build_ms();
    if outspent(left_ms, measured) {
        // What the uncapped policy would have spent is kept beside the two
        // numbers that decided: capping is a choice about the tree's quality,
        // and the counterfactual is what a reader needs to weigh it.
        diag!(
            "[portfolio] capped by the last build here ({measured}ms measured, {left}ms left),              uncapped policy wanted {wanted}ms",
            measured = measured.unwrap_or(0),
            left = left_ms.unwrap_or(0),
            wanted = crate::budget::vtree_budget_ms(left_ms.unwrap_or(0).max(0) as u64),
        );
        run.behind_schedule = true;
    }

    // A deadline already passed on entry — common once a multi-component build
    // has spent its budget on earlier components — skips the whole catalog on
    // the first iteration, so construction fails outright.
    let mut skipped: Vec<&'static str> = Vec::new();
    for (i, c) in catalog.iter().enumerate() {
        if inp.out_of_time() {
            skipped.extend(catalog[i..].iter().map(|c| c.name));
            break;
        }
        run.cand_cap_ms = inp.fair_share_ms(catalog.len() - i);
        // The hard bound: whatever is still left of the whole construction
        // budget. `out_of_time` above has already ruled out a non-positive one.
        run.cand_wall_ms = inp.remaining_ms().map(|r| r.max(1));
        let slice_start = std::time::Instant::now();
        let open = match c.gate {
            Gate::Always => true,
            Gate::FromInputs(gate) => gate(&inp),
            Gate::FromDerived(gate) => gate(
                &inp,
                derived.get_or_insert_with(|| Derived::compute(&inp, &run)),
            ),
        };
        if open && let Some(built) = (c.build)(&inp, &mut run) {
            run.fold(&inp, derived.as_ref(), c, built);
        }
        if run
            .cand_cap_ms
            .is_some_and(|cap| (slice_start.elapsed().as_millis() as i64) > cap)
        {
            run.behind_schedule = true;
        }
    }

    let RunState {
        mut best,
        mut trace_rows,
        cands,
        hypergraph_bisect_040_built,
        ..
    } = run;
    let candidate_capacity = inp.candidate_capacity;

    if trace_all && num_vars <= PORTFOLIO_HEAVY_MAX_VARS {
        trace_rows.extend(trace_hg_bisect_family(
            formula,
            effort_scale,
            hypergraph_bisect_040_built,
        )?);
    }

    if peak_mode && !cands.is_empty() {
        let rel_tol = inp.peak_tolerance;
        let pick = select_peak_band(&cands, rel_tol);
        best.adopt(
            &pick.stats,
            Arc::clone(&pick.vtree),
            pick.meta.clone(),
            pick.name,
            pick.param,
        );
    }

    let mut candidate_set = CandidateSet::default();
    // Every number here was already computed by `fold`, so this is one dedup
    // pass over vtrees the run already holds — nothing is rebuilt or rescored.
    if candidate_capacity > 1
        && let Some(winner) = best.vtree.as_ref()
    {
        let scored: Vec<crate::candidates::ScoredVtree> = cands
            .iter()
            .map(|c| crate::candidates::ScoredVtree {
                built_by: candidate_spec(c.name, c.param),
                vtree: Arc::clone(&c.vtree),
                scores: c.stats,
            })
            .collect();
        candidate_set =
            crate::candidates::from_scored(scored, winner, rank_metric, candidate_capacity);
    }

    // One line per build, whatever happened. It used to be emitted only when the
    // budget had already forced entries to be dropped, so the builds that
    // finished inside their budget — the majority — reported no construction
    // time at all, and a reader of the two lines together saw a distribution
    // with the cheap builds filtered out. The skip list is a field of the
    // report now, not the condition for making one.
    diag!(
        "[portfolio] wall_ms={wall} vars={num_vars} budget_ms={budget} skip={skip}",
        wall = inp.t_build.elapsed().as_millis(),
        budget = entry_budget_ms
            .map(|b| b.to_string())
            .unwrap_or_else(|| "-".to_string()),
        skip = if skipped.is_empty() {
            "-".to_string()
        } else {
            skipped.join(",")
        },
    );

    // Assembled once: what the run announces as its winner is the same string
    // it publishes, so a reader of either can ask for that construction back.
    let winner = candidate_spec(best.name, best.param);
    report_selection(
        &best,
        &winner,
        peak_mode,
        trace,
        &trace_rows,
        derived.as_ref(),
        num_vars,
    );
    let vtree = best
        .vtree
        .ok_or_else(|| VitriError::construction("portfolio", "every candidate failed"))?;
    // The winner is named, not the `portfolio` spec that ran it — which
    // construction won is what a consumer cannot otherwise recover.
    Ok(VtreeArtifacts {
        vtree,
        selection: SelectionRecord {
            winning_spec: Some(winner),
            td_meta: best.meta,
        },
        candidate_set,
    })
}

/// Build and score the hypergraph-bisect family at every imbalance point, for
/// the trace alone: these rows report what the other imbalances would have
/// scored, and nothing here can reach the selection. `already_built` skips the
/// one point the chain itself covered.
fn trace_hg_bisect_family(
    formula: &CnfFormula,
    effort_scale: f64,
    already_built: bool,
) -> Result<Vec<TraceRow>, VitriError> {
    let mut rows = Vec::new();
    for &imb in &[0.03_f64, 0.10, 0.20, 0.30, 0.40] {
        if (imb - crate::decompose::multilevel_hg_bisect::IMBALANCE_PORTFOLIO_RELAXED).abs() < 1e-9
            && already_built
        {
            continue;
        }
        let dials = crate::decompose::BisectDials {
            imbalance: imb,
            base_seed: 0,
            effort_scale,
        };
        if let Ok(v) = crate::decompose::multilevel_hg_bisect::vtree_from_hg_bisect(formula, dials)
        {
            // Scored exactly as a realized candidate is, through the one owner:
            // a trace row that disagreed with the selector would be reporting a
            // different run than the one that happened. No show mask — no trace
            // column reads the show peak.
            let scores = VtreeScores::compute(&v, formula, None)?;
            rows.push(TraceRow::from_scores(
                "hypergraph-bisect",
                format!("{imb:.2}"),
                &scores,
                false,
            ));
        }
    }
    Ok(rows)
}

/// Announce the pick, and under a trace every candidate row and the structure
/// signals the pick was made under.
fn report_selection(
    best: &Incumbent,
    winner: &str,
    peak_mode: bool,
    trace: bool,
    trace_rows: &[TraceRow],
    derived: Option<&Derived>,
    num_vars: u32,
) {
    let sel_metric = if peak_mode { "peak-width" } else { "stddev" };
    diag!(
        "[portfolio] selected: {winner} (metric={sel_metric}, stddev={stddev:.2}, cost={cost})",
        stddev = best.stddev,
        cost = best.cost,
    );
    // `adopted` marks the final chain pick; hypergraph-bisect is param-agnostic
    // in the incumbent's name, so only the 0.40 representative can carry `adopted=1`.
    if trace {
        for row in trace_rows {
            let adopted = row.family == best.name;
            diag!(
                "[portfolio-trace] cand family={fam} param={param} stddev={sd:.4} mcl={mcl} peak={peak} cost={cost} built={} adopted={}",
                row.built as u8,
                adopted as u8,
                fam = row.family,
                param = row.param,
                sd = row.stddev,
                mcl = row.mcl,
                peak = row.peak_context_width_all,
                cost = row.cost,
            );
        }
        diag!(
            "[portfolio-trace] pick name={name} stddev={stddev:.4} coloring_like={} gen_gate={} num_vars={num_vars}",
            derived.is_some_and(|d| d.coloring_like) as u8,
            derived.is_some_and(|d| d.hypergraph_bisect_gen_gate) as u8,
            name = best.name,
            stddev = best.stddev,
        );
    }
}
