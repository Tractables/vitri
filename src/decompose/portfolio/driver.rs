//! The portfolio driver: run the catalog, select a winner, publish the result.
//!
//! A construction budget that leaves nothing built is a hard error. A budget
//! already spent when the walk starts is not that case: the entry the walk
//! stops at gets one attempt under a fixed short wall, and only a budget under
//! which that attempt also produces nothing reaches
//! `Err(VitriError::construction(..))`.
//!
//! **Determinism:** what a portfolio build produces is a function of the
//! formula and of the budget it was given. The constructions themselves hold up
//! their end of that unconditionally — FlowCutter searches to a fixed step
//! budget under a fixed seed in the C++ backend, the multilevel bisections run
//! seeded RNGs over sorted edge accumulation, and selection is a numeric
//! comparison of scores. What the budget is decides how far it reaches, because
//! the budget is what every gate here is measured against.
//!
//! Under a
//! [`ConstructionBudget::Deterministic`](crate::config::ConstructionBudget)
//! budget the whole build is reproducible. The budget is a count of
//! construction work rather than a span of time, and every gate below reads the
//! clock that work drives ([`crate::decompose::meter`]) — which entries are
//! attempted, what each is scheduled, whether one overran, and whether the
//! projected large-component cap has been spent. None of those answers depends
//! on how fast the machine was or what else it was running, so the same formula
//! at the same unit budget considers the same candidates in the same order and
//! selects the same vtree on every machine.
//!
//! Under a wall-clock budget (`BuildLimits::deadline`) those same gates read
//! the wall, and a build that overruns skips what is left and tightens the
//! FlowCutter searches behind it — which is load-dependent. Every entry is
//! additionally bounded at the time left when it starts, but that bound is a
//! [`WallCapMode::BoundOnly`](crate::decompose::WallCapMode) one — the search it
//! runs is the unbounded one, and the wall only stops it once it has genuinely
//! passed — so a wall-budgeted build is still reproducible for every formula
//! that finishes construction inside its budget.

use crate::candidates::CandidateSet;
use crate::cnf::CnfFormula;
use crate::decompose::{BuildLimits, SelectionCtx, SelectionObjective, TraceLevel};
use crate::diagnostics::diag;
use crate::error::VitriError;
use crate::score::VtreeScores;
use crate::spec::{SelectionRecord, VtreeArtifacts};
use std::sync::Arc;

use super::catalog::{
    CatalogEntry, Derived, Gate, Incumbent, Inputs, PORTFOLIO_HEAVY_MAX_VARS, RunState,
    ScoredCandidate, TraceRow, build_fc_inc, build_fc_pri, build_goatd, build_guided_bisect,
    build_hypergraph_bisect, candidate_spec, gate_goatd, gate_guided_bisect,
    gate_hypergraph_bisect, outspent, work_ms_since,
};

/// The wall one catalog entry gets when the construction deadline is already
/// spent and nothing has been built.
///
/// It is a fixed number rather than a share of what is left, because what is
/// left is zero or less. Short enough that a build already over its budget does
/// not go far past it, and long enough for the first entry — an anytime cutter
/// under a timed budget — to return a decomposition.
const LAST_ATTEMPT_MS: i64 = 1_000;

/// One build's wall report: a build that left candidates unstarted is the
/// truncated one, and a build that walked the whole catalog is the complete
/// one. Stated here rather than at the call site so the rule can be asked
/// without a clock.
pub(super) fn limits_report(
    skipped: &[&'static str],
    spent: std::time::Duration,
) -> crate::decompose::BuildLimitsReport {
    crate::decompose::BuildLimitsReport {
        truncated_builds: u32::from(!skipped.is_empty()),
        complete_builds: u32::from(skipped.is_empty()),
        spent_ms: spent.as_millis() as u64,
        skipped: skipped.iter().map(|n| (*n).to_string()).collect(),
    }
}

/// Refuse a candidate name no catalog entry answers to, listing the ones that
/// do — a caller that mistyped one gets the alternatives rather than a build
/// that silently ignored the request.
fn check_candidate_name(name: &str) -> Result<(), VitriError> {
    let names = super::PortfolioKnobs::candidate_names();
    if catalog().iter().any(|c| c.name == name) || names.iter().any(|n| n == name) {
        return Ok(());
    }
    Err(VitriError::config(format!(
        "no portfolio candidate is named {name:?}; the catalog builds {}",
        names.join(", "),
    )))
}

/// Records the build's wall on every exit, including an unwind: a build that
/// died partway still measured what a build here costs.
struct MeasureBuild<'a> {
    started: std::time::Instant,
    history: &'a super::PortfolioBuildHistory,
}

impl<'a> MeasureBuild<'a> {
    fn new(history: &'a super::PortfolioBuildHistory) -> Self {
        Self {
            started: std::time::Instant::now(),
            history,
        }
    }
}

impl Drop for MeasureBuild<'_> {
    fn drop(&mut self) {
        self.history
            .record(self.started.elapsed().as_millis() as u64);
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
        },
        CatalogEntry {
            name: "flowcutter-primal",
            param: None,
            td_based: true,
            gate: Gate::Always,
            build: build_fc_pri,
        },
        CatalogEntry {
            name: "goatd-incidence",
            param: None,
            td_based: true,
            gate: Gate::FromInputs(gate_goatd),
            build: build_goatd,
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
        },
        // The same incidence decomposition as the first entry, guiding a
        // recursive bisection instead of being converted bag by bag.
        CatalogEntry {
            name: "guided-bisect",
            param: None,
            td_based: false,
            gate: Gate::FromDerived(gate_guided_bisect),
            build: build_guided_bisect,
        },
    ]
}

/// FlowCutter incidence + primal, goatd, plus the structure-gated
/// hypergraph-bisect and guided-bisect bisection candidates.
/// Selection picks the lowest combined cost in plain mode and uses the
/// projection-aware peak-width selector in projected mode.
///
/// A "separator" candidate was removed deliberately: every apparent win it
/// scored came with a much larger realized diagram. Do not restore it.
pub(crate) fn vtree_from_portfolio(
    formula: &CnfFormula,
    steps: i64,
    iters: i32,
    reading: crate::decompose::Reading,
    ctx: &SelectionCtx,
    limits: &BuildLimits,
) -> Result<VtreeArtifacts, VitriError> {
    let history = &ctx.portfolio.build_history;
    let _measured = MeasureBuild::new(history);
    let seed = ctx.portfolio.seed;
    let num_vars = formula.num_vars;
    if num_vars == 0 {
        return Err(VitriError::construction(
            "portfolio",
            crate::decompose::EMPTY_FORMULA,
        ));
    }
    // Checked before anything is built: a preference naming a candidate this
    // catalog does not have would otherwise spend the whole construction budget
    // and then quietly select on score, which is the failure the caller asked
    // for the preference to avoid.
    if let Some(prefer) = &ctx.portfolio.prefer {
        check_candidate_name(prefer.name())?;
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
    // Two readings of one moment. The real one is a MEASUREMENT, and backs the
    // wall this build reports when it is done. The construction clock's is what
    // the projected large-component cap is measured against, because that cap
    // decides which entries are attempted at all — and a decision about which
    // tree comes out has to be reproducible.
    let t_build_real = std::time::Instant::now();
    let t_build = crate::decompose::meter::now();

    // The metric this run ranks by, fixed before anything is built so deferred
    // selection and the exported candidate order agree. Without a show mask,
    // projected selection reads the all-variable peak.
    let rank_metric = match ctx.objective {
        SelectionObjective::ClauseBalance => crate::candidates::CandidateRankMetric::Cost,
        SelectionObjective::PeakWidthShow(_) => {
            crate::candidates::CandidateRankMetric::PeakContextWidthShow
        }
        SelectionObjective::PeakWidthAll => {
            crate::candidates::CandidateRankMetric::PeakContextWidthAll
        }
    };

    let inp = Inputs {
        formula,
        source_profile: ctx.source_profile,
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
        reading,
        conversion_trace: ctx.conversion.trace,
        prefer: ctx.portfolio.prefer.as_ref(),
    };

    let mut run = RunState::new(reduced_steps, iters);

    // The construction budget this call was admitted under, read once before
    // anything is built, from the same source the loop below reads. `None` = no
    // deadline. The wall report at the end compares the spent wall against it.
    let entry_budget_ms = inp.remaining_ms();

    let mut derived: Option<Derived> = None;

    let catalog = catalog();

    // A build with less room than the preceding one in this caller-owned
    // history enters
    // the capped regime at once, rather than discovering it one candidate too
    // late. The behind-schedule latch trips only after some candidate has
    // already overspent, so on a build that is short from the start it arms too
    // late to bound the candidate that spends the room.
    //
    // Both values are read once, here, and the message below prints those same
    // locals: a message that re-read the clock would report a state the
    // condition was never evaluated on.
    //
    // THE GATE STANDS DOWN under a deterministic construction budget, rather
    // than being converted to work units, for three reasons:
    //
    //  - What it consults is a wall measurement from a DIFFERENT build in the
    //    caller's explicitly shared cascade. It is reproducible in ownership
    //    and order, but it still cannot honestly be converted into work units.
    //  - The question it answers is already answered better. It exists because a
    //    build cannot otherwise tell how much room it has until a candidate has
    //    overspent; under a unit budget the room IS the budget and is known
    //    exactly at entry, and the fair-share and behind-schedule machinery
    //    below already reads it on the construction clock.
    //  - Standing down costs nothing it was buying: the entry it would have
    //    tightened is bounded by its fair share either way.
    let left_ms = inp.remaining_ms();
    let measured = history.last_build_ms();
    if !crate::decompose::meter::is_armed() && outspent(left_ms, measured) {
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
    // has spent its budget on earlier components — would skip the whole catalog
    // on the first iteration and fail the construction outright. A candidate
    // that could have been built is worth more than the deadline it misses, so
    // the entry the loop stopped at gets one attempt under a fixed short wall
    // when nothing has been built yet; the rest are skipped either way.
    let mut skipped: Vec<&'static str> = Vec::new();
    let mut last_attempt = false;
    for (i, c) in catalog.iter().enumerate() {
        if inp.out_of_time() {
            // Both, because which of the two a built candidate lands in depends
            // on the mode: plain selection adopts into `best`, projected
            // selection collects into `cands` and chooses at the end.
            if run.best.vtree.is_none() && run.cands.is_empty() {
                diag!(
                    "[portfolio] deadline spent with nothing built; {} gets {LAST_ATTEMPT_MS}ms",
                    c.name,
                );
                last_attempt = true;
            } else {
                skipped.extend(catalog[i..].iter().map(|c| c.name));
                break;
            }
        }
        if last_attempt {
            run.cand_cap_ms = Some(LAST_ATTEMPT_MS);
            run.cand_wall_ms = Some(LAST_ATTEMPT_MS);
            // The regime where finishing beats searching, which is what this
            // attempt is: the wall is the whole budget it has.
            run.behind_schedule = true;
        } else {
            run.cand_cap_ms = inp.fair_share_ms(catalog.len() - i);
            // The hard bound: whatever is still left of the whole construction
            // budget. `out_of_time` above has already ruled out a non-positive
            // one.
            run.cand_wall_ms = inp.remaining_ms().map(|r| r.max(1));
        }
        // Where this entry's slice starts, on the construction clock: what the
        // latch below decides — whether the entries behind this one search less
        // patiently — is a decision about which tree comes out, so it is
        // measured in the work the entry does rather than in the time it took.
        let slice_start = crate::decompose::meter::now();
        let open = match c.gate {
            Gate::Always => true,
            Gate::FromInputs(gate) => gate(&inp),
            Gate::FromDerived(gate) => gate(
                &inp,
                derived.get_or_insert_with(|| Derived::compute(&inp, &run)),
            ),
        };
        if open && let Some(built) = (c.build)(&inp, &mut run) {
            run.fold(&inp, c, built);
        }
        // One attempt is all a spent deadline buys, whether or not it produced
        // anything: the entries behind it are skipped.
        if last_attempt {
            skipped.extend(catalog[i + 1..].iter().map(|c| c.name));
            break;
        }
        if run
            .cand_cap_ms
            .is_some_and(|cap| (work_ms_since(slice_start) as i64) > cap)
        {
            run.behind_schedule = true;
        }
    }

    let RunState {
        mut best,
        mut trace_rows,
        cands,
        hypergraph_bisect_040_built,
        preferred,
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

    // Last, so it overrides both the greedy adoption and the projected band:
    // the preference is a decision about which tree to take away, and every
    // scoring rule above it has already had its say.
    if let Some(prefer) = &ctx.portfolio.prefer {
        match preferred {
            Some(c) => best.adopt(&c.stats, c.vtree, c.meta, c.name, c.param),
            None if prefer.is_required() => {
                return Err(VitriError::construction(
                    "portfolio",
                    format!(
                        "the required candidate {} did not build over this formula",
                        prefer.name()
                    ),
                ));
            }
            None => diag!(
                "[portfolio] preferred candidate {} did not build; selecting on score",
                prefer.name(),
            ),
        }
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
        wall = t_build_real.elapsed().as_millis(),
        budget = entry_budget_ms
            .map(|b| b.to_string())
            .unwrap_or_else(|| "-".to_string()),
        skip = if skipped.is_empty() {
            "-".to_string()
        } else {
            skipped.join(",")
        },
    );

    // The same two numbers the line above prints, as data: a caller reading a
    // result file rather than a console needs the truncation to be a field.
    let report = limits_report(&skipped, t_build_real.elapsed());

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
    let scores = best
        .scores
        .expect("a selected portfolio vtree has already been scored");
    history.record_winner(&winner, scores);
    // The winner is named, not the `portfolio` spec that ran it — which
    // construction won is what a consumer cannot otherwise recover.
    Ok(VtreeArtifacts {
        vtree,
        selection: SelectionRecord {
            winning_spec: Some(winner),
            scores: Some(scores),
            td_meta: best.meta,
        },
        candidate_set,
        limits: report,
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
    let sel_metric = if peak_mode { "peak-width" } else { "cost" };
    diag!(
        "[portfolio] selected: {winner} (metric={sel_metric}, stddev={stddev:.2}, cost={cost:.2})",
        stddev = best.stddev,
        cost = best.cost,
    );
    // `adopted` marks the final chain pick; hypergraph-bisect is param-agnostic
    // in the incumbent's name, so only the 0.40 representative can carry `adopted=1`.
    if trace {
        for row in trace_rows {
            let adopted = row.family == best.name;
            diag!(
                "[portfolio-trace] cand family={fam} param={param} stddev={sd:.4} mcl={mcl} peak={peak} cost={cost:.4} built={} adopted={}",
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
