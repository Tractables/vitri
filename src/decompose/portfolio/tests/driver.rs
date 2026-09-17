//! Selection pins and construction-budget guarantees for the portfolio driver.

use crate::decompose::BuildLimits;
use crate::decompose::Place;
use crate::decompose::Reading;
use crate::decompose::SelectionCtx;
use crate::decompose::goatd::candidate_param;
use crate::decompose::portfolio::catalog::Inputs;
use crate::decompose::portfolio::catalog::RunState;
use crate::decompose::portfolio::catalog::ScoredCandidate;
use crate::decompose::portfolio::catalog::candidate_spec;
use crate::decompose::portfolio::driver::*;
use crate::score::VtreeScores;
use crate::score::agg::AggScore;
use crate::spec::{PORTFOLIO_ITERS, PORTFOLIO_STEPS};
use crate::vtree::Vtree;
use std::sync::Arc;

/// Pin projected ranking with explicit legacy refinement and every catalog
/// entry. Default polishing has a cooperative wall cap, so its offered trees
/// can differ with machine speed and load.
#[test]
fn peak_mode_selection_pin() {
    let formula = crate::tests::circuit_fixture::multiplier();
    let mut ctx = SelectionCtx::peak();
    ctx.goatd.polishing = crate::decompose::GoatdPolishing::legacy(true, true);
    ctx.portfolio.skip = Vec::new();
    // Same portfolio params as the `portfolio` spec builds with.
    let built = vtree_from_portfolio(
        &formula,
        PORTFOLIO_STEPS,
        PORTFOLIO_ITERS,
        Reading::default(),
        &ctx,
        &BuildLimits::default(),
    )
    .expect("portfolio");
    assert_eq!(
        built.selection.winning_spec.as_deref(),
        Some("goatd-incidence"),
        "peak-mode selection changed"
    );
    assert!(
        built.selection.scores.is_some(),
        "a portfolio winner must carry the scores used to select it",
    );
}

#[test]
fn build_history_is_shared_only_when_the_caller_clones_it() {
    let first = crate::decompose::PortfolioBuildHistory::default();
    let same_cascade = first.clone();
    let independent = crate::decompose::PortfolioBuildHistory::default();

    first.record(17);
    let scores = VtreeScores {
        clause_load_stddev: 1.0,
        max_clause_load: 2,
        peak_context_width_all: 3,
        peak_context_width_show: None,
        cost: 4.0,
    };
    first.record_winner("flowcutter-incidence", scores);

    assert_eq!(same_cascade.last_build_ms(), Some(17));
    assert_eq!(
        same_cascade.last_winning_spec().as_deref(),
        Some("flowcutter-incidence"),
    );
    assert_eq!(same_cascade.last_scores(), Some(scores));
    assert_eq!(independent.last_build_ms(), None);
    assert_eq!(independent.last_winning_spec(), None);
    assert_eq!(independent.last_scores(), None);
}

fn budget_fixture() -> crate::cnf::CnfFormula {
    crate::tests::circuit_fixture::multiplier()
}

/// A SPENT BUDGET STILL BUILDS ONE CANDIDATE: a deadline that has ALREADY
/// passed on entry leaves no share for anything, but a tree the caller can use
/// is worth more than the deadline it misses. The first catalog entry runs
/// under a fixed short wall and every entry behind it is reported as never
/// started, which is how a caller tells this tree from a complete one.
///
/// The spent deadline is constructed, not waited for: an `Instant` already in
/// the past is past on entry on any machine, so the case under test is reached
/// without timing anything.
#[test]
fn an_expired_deadline_still_builds_the_first_candidate() {
    use std::time::{Duration, Instant};
    let formula = budget_fixture();
    let limits = BuildLimits {
        deadline: Some(Instant::now() - Duration::from_secs(1)),
        ..BuildLimits::default()
    };
    let built = vtree_from_portfolio(
        &formula,
        PORTFOLIO_STEPS,
        PORTFOLIO_ITERS,
        Reading::default(),
        &SelectionCtx::plain(),
        &limits,
    )
    .expect("a spent deadline must still hand back a vtree");
    assert_eq!(
        built.vtree.num_leaves(),
        formula.num_vars,
        "the tree must cover the formula",
    );
    assert_eq!(
        built.limits.truncated_builds, 1,
        "a build that left catalog entries unstarted is the truncated one",
    );
    // The entries the build had: the catalog minus the default skip list.
    let behind_the_first: Vec<String> = catalog_with_knobs(&SelectionCtx::plain().portfolio.skip)
        .iter()
        .skip(1)
        .map(|c| c.name.into())
        .collect();
    assert_eq!(
        built.limits.skipped, behind_the_first,
        "one attempt is all a spent deadline buys: every entry behind it is never started",
    );
}

/// Goatd enables additional improvement stages when it has a deadline.
/// The fixed-schedule candidates still search identically under a generous cap.
#[test]
fn a_generous_deadline_preserves_the_fixed_schedule_candidates() {
    use std::time::{Duration, Instant};
    let formula = budget_fixture();
    let mut ctx = SelectionCtx::plain();
    ctx.portfolio.skip.push("goatd-incidence");
    let unbounded = vtree_from_portfolio(
        &formula,
        PORTFOLIO_STEPS,
        PORTFOLIO_ITERS,
        Reading::default(),
        &ctx,
        &BuildLimits::default(),
    )
    .expect("portfolio (no deadline)");
    let limits = BuildLimits {
        deadline: Some(Instant::now() + Duration::from_secs(3600)),
        ..BuildLimits::default()
    };
    let bounded = vtree_from_portfolio(
        &formula,
        PORTFOLIO_STEPS,
        PORTFOLIO_ITERS,
        Reading::default(),
        &ctx,
        &limits,
    )
    .expect("portfolio (generous deadline)");
    assert_eq!(
        bounded.selection.winning_spec, unbounded.selection.winning_spec,
        "a generous budget changed which candidate was selected",
    );
    assert_eq!(
        bounded.vtree.to_vtree_text(),
        unbounded.vtree.to_vtree_text(),
        "a generous budget changed the constructed vtree",
    );
    // The other side of the fallback above: with time left on entry the walk is
    // the ordinary fair-share one, so nothing is skipped and no entry is cut
    // down to the one-attempt wall.
    assert!(
        bounded.limits.skipped.is_empty(),
        "a budget with time left must walk the whole catalog",
    );
    assert_eq!(
        bounded.limits.complete_builds, 1,
        "a build that walked the whole catalog is the complete one",
    );
}

/// Tiny dummy ScoredCandidate (the vtree is never inspected by select_peak_band).
fn sc(sel_metric: f64, clause_load_stddev: f64, cost: f64, name: &'static str) -> ScoredCandidate {
    ScoredCandidate {
        sel_metric,
        stats: VtreeScores {
            clause_load_stddev,
            max_clause_load: 0,
            peak_context_width_all: sel_metric as u32,
            peak_context_width_show: None,
            cost,
        },
        agg: None,
        name,
        param: None,
        vtree: Arc::new(Vtree::balanced(2)),
        meta: None,
    }
}

/// The aggregate ranker decides the pick when it is on, and the run selects on
/// the cost when it is off. The two candidates are the d1 pair the ranker's own
/// tests score: their cost order and their aggregate order are opposite. A
/// margin narrower than the gap between the two costs takes the ranker's
/// favourite out of the field and leaves the cost pick standing.
#[test]
fn the_aggregate_ranker_picks_against_the_cost_and_only_when_it_is_on() {
    let mut cheap = sc(10.0, 1.0, 33.9617, "flowcutter-incidence");
    cheap.agg = Some(AggScore::Scalar(54.96));
    let mut wide = sc(10.0, 2.0, 35.2466, "flowcutter-primal");
    wide.agg = Some(AggScore::Scalar(51.25));
    let cands = vec![cheap, wide];
    assert_eq!(select_agg(&cands, None).name, "flowcutter-primal");
    // The two costs are 1.28 apart.
    assert_eq!(select_agg(&cands, Some(0.5)).name, "flowcutter-incidence");
    assert_eq!(select_agg(&cands, Some(2.0)).name, "flowcutter-primal");
    // Off, `fold`'s streaming greedy compares the cost and nothing else, and
    // the driver never reaches `select_agg` at all.
    assert_eq!(
        greedy_index(cands.iter().map(|c| c.stats.cost)),
        Some(0),
        "the cost pick is the first candidate",
    );
}

/// Band selection: among candidates within the peak band it picks minimum
/// stddev, and it never reaches a lower-stddev candidate that falls OUTSIDE the
/// band.
#[test]
fn select_peak_band_default_min_stddev_within_band() {
    // min_peak = 10.0, rel_tol = 0.10 → band = 11.0.
    let cands = vec![
        sc(10.0, 8.0, 100.0, "in_hi_stddev"), // in band, higher stddev
        sc(11.0, 4.0, 100.0, "in_lo_stddev"), // in band (11.0 <= 11.0), lower stddev → winner
        sc(20.0, 1.0, 100.0, "out_lowest"),   // out of band; lowest stddev but excluded
    ];
    let pick = select_peak_band(&cands, 0.10);
    assert_eq!(
        pick.name, "in_lo_stddev",
        "the band pick must be min-stddev within band"
    );
}

/// SINGLE SOURCE OF TRUTH: given the portfolio's own effort — its step budget
/// and iteration count, written out as `budget=150000steps,iters=15` — the
/// `guided-bisect` spec builds exactly the tree the portfolio's own code builds
/// from the FlowCutter incidence decomposition it holds. They are one
/// construction reached two ways, and a second implementation grown beside the
/// first would show up here as two different trees.
///
/// White-box on purpose: the comparison is against the candidate's build
/// function itself, run against the decomposition candidate 1 produces, so the
/// pin does not depend on which candidate selection would have picked.
#[test]
fn the_guided_bisect_spec_is_the_construction_the_portfolio_builds() {
    // Both sides scale their FlowCutter effort from the budget hint in the
    // build limits, and the default leaves it unset, so the two coincide
    // whatever the environment holds.
    let formula = crate::tests::circuit_fixture::multiplier();
    let ctx = SelectionCtx::plain();
    let limits = BuildLimits::default();
    let inp = super::inputs(&formula, &ctx, &limits, None);
    // Same effort the `portfolio` spec builds with, which is what lets a spec
    // naming that effort literally reproduce these trees.
    let mut run = RunState::new(PORTFOLIO_STEPS, PORTFOLIO_ITERS);
    let entry = |name: &str| {
        CATALOG
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("{name} is a catalog entry"))
    };
    assert!(
        !entry("flowcutter-incidence")
            .offer(&inp, &mut run)
            .is_empty(),
        "the flowcutter-incidence candidate must build"
    );
    let guided = entry("guided-bisect")
        .offer(&inp, &mut run)
        .pop()
        .expect("the guided-bisect candidate must build");

    let spec = "guided-bisect:budget=150000steps,iters=15";
    let parsed = crate::spec::parse_vtree_spec(spec).expect("the spec must parse");
    let standalone = crate::spec::build_one_vtree_artifacts(crate::spec::BuildRequest {
        formula: &formula,
        spec: &parsed,
        ctx: &SelectionCtx::plain(),
        limits: &BuildLimits::default(),
    })
    .unwrap_or_else(|e| panic!("{spec} must build: {e}"))
    .vtree;
    assert_eq!(
        standalone.to_vtree_text(),
        guided.vtree.to_vtree_text(),
        "{spec} must build exactly what the portfolio builds under that name",
    );
}

/// A candidate's name is what a run publishes as its winner, and the catalog
/// is the one place that vocabulary meets the `--vtree` grammar. A name the
/// grammar cannot build — or a param it would reject — is a dead end for the
/// caller who read the name out of a bundle and asked for that construction
/// back, so it fails here instead of in their hands.
#[test]
fn every_catalog_candidate_names_a_spec_that_rebuilds_it() {
    for c in CATALOG {
        assert_ne!(
            crate::spec::parse::classify_base(c.name),
            crate::spec::VtreeBase::Unknown,
            "catalog candidate '{}' names no buildable family",
            c.name,
        );
        assert!(
            c.offers == 1 || c.param.is_none(),
            "catalog candidate '{}' offers several trees, so a runner-up's spec \
             would drop the parameter '{:?}' the entry itself is built at",
            c.name,
            c.param,
        );
        // Every tree the entry can offer, not just its first: a runner-up is
        // published as a winner too, so its name has to rebuild it as well.
        for spec in c.published_specs() {
            crate::spec::validate_vtree_spec(&spec).unwrap_or_else(|e| {
                panic!(
                    "'{spec}' does not rebuild catalog candidate '{}': {e}",
                    c.name
                )
            });
        }
    }
}

/// The portfolio builds the bisection family at a RELAXED imbalance, while a
/// bare `hypergraph-bisect` spec means the balanced default — so the candidate
/// records the imbalance, and this pins that record to the constant its build
/// passes. Were the two to drift, the spec a reader assembles from the
/// published name would rebuild a different tree.
#[test]
fn the_bisection_candidate_records_the_imbalance_it_builds_at() {
    use crate::decompose::multilevel_hg_bisect::IMBALANCE_PORTFOLIO_RELAXED;

    let c = CATALOG
        .iter()
        .find(|c| c.name == "hypergraph-bisect")
        .expect("the bisection candidate is in the catalog");
    // The same string the plain-MC trace prints for a realized row as the
    // `=all` pass prints for a simulated one; that pass dedups on them agreeing.
    assert_eq!(
        c.param,
        Some(format!("imbalance={IMBALANCE_PORTFOLIO_RELAXED:.2}").as_str())
    );
    match crate::spec::parse_vtree_spec(&candidate_spec(c.name, c.param))
        .expect("a valid spec")
        .param
    {
        crate::spec::parse::SpecParam::Imbalance(v) => assert_eq!(v, IMBALANCE_PORTFOLIO_RELAXED),
        _ => panic!("the bisection spec's param is an imbalance"),
    }
}

/// The inputs the cap gates read, over the budget fixture: the plain context
/// and the default limits, with the cap itself left to the caller.
fn cap_gate_inputs<'a>(
    formula: &'a crate::cnf::CnfFormula,
    flowcutter_cap_ms: Option<i64>,
) -> Inputs<'a> {
    super::inputs(
        formula,
        &SelectionCtx::plain(),
        &BuildLimits::default(),
        flowcutter_cap_ms,
    )
}

#[test]
fn portfolio_td_candidates_preserve_open_or_explicit_placement() {
    let formula = budget_fixture();
    let mut inp = cap_gate_inputs(&formula, None);
    let conversion = inp.conversion("flowcutter-primal");
    assert_eq!(conversion.reading.place, None);

    inp.reading.place = Some(Place::Shallow);
    let explicit = inp.conversion("flowcutter-primal");
    assert_eq!(explicit.reading.place, Some(Place::Shallow));

    inp.reading.place = Some(Place::Deep);
    let explicit = inp.conversion("flowcutter-primal");
    assert_eq!(explicit.reading.place, Some(Place::Deep));
}

/// Under a deadline the first entry is already bounded, at the whole time left
/// rather than at its fair share.
///
/// The deadline itself is consulted only between entries, so before this the
/// first expensive entry ran with no wall at all — and that is the entry which
/// overruns the ceiling.
#[test]
fn the_first_entry_is_bounded_by_the_whole_time_left_not_by_its_share() {
    let formula = budget_fixture();
    let inp = cap_gate_inputs(&formula, None);
    let mut run = RunState::new(PORTFOLIO_STEPS, PORTFOLIO_ITERS);
    run.cand_wall_ms = Some(5_000);
    run.cand_cap_ms = Some(1_000);
    assert_eq!(run.fc_time_cap_ms(&inp), Some(5_000));
}

/// A wall the build is expected to finish inside leaves the search alone.
#[test]
fn a_wall_armed_on_a_healthy_build_is_bound_only() {
    let formula = budget_fixture();
    let inp = cap_gate_inputs(&formula, None);
    let mut run = RunState::new(PORTFOLIO_STEPS, PORTFOLIO_ITERS);
    run.cand_wall_ms = Some(5_000);
    assert_eq!(
        run.fc_cap_mode(&inp),
        crate::decompose::WallCapMode::BoundOnly
    );
}

/// Once an entry has overrun its share the remaining builds take both the fair
/// share as their cap and the tight search with it.
#[test]
fn a_build_behind_schedule_is_capped_at_its_share_and_searches_tight() {
    let formula = budget_fixture();
    let inp = cap_gate_inputs(&formula, None);
    let mut run = RunState::new(PORTFOLIO_STEPS, PORTFOLIO_ITERS);
    run.cand_wall_ms = Some(5_000);
    run.cand_cap_ms = Some(1_000);
    run.behind_schedule = true;
    assert_eq!(run.fc_time_cap_ms(&inp), Some(1_000));
    assert_eq!(run.fc_cap_mode(&inp), crate::decompose::WallCapMode::Tight);
}

/// The projected large-component cap composes with the rest and, like the
/// behind-schedule share, means the wall is expected to bite.
#[test]
fn the_projected_component_cap_tightens_the_search_it_bounds() {
    let formula = budget_fixture();
    let inp = cap_gate_inputs(&formula, Some(200));
    let mut run = RunState::new(PORTFOLIO_STEPS, PORTFOLIO_ITERS);
    run.cand_wall_ms = Some(5_000);
    assert_eq!(run.fc_time_cap_ms(&inp), Some(200));
    assert_eq!(run.fc_cap_mode(&inp), crate::decompose::WallCapMode::Tight);
}

/// With no deadline and no cap there is no wall, which is the deterministic
/// step-budgeted search.
#[test]
fn a_build_with_no_deadline_and_no_cap_gets_no_wall() {
    let formula = budget_fixture();
    let inp = cap_gate_inputs(&formula, None);
    let run = RunState::new(PORTFOLIO_STEPS, PORTFOLIO_ITERS);
    assert_eq!(run.fc_time_cap_ms(&inp), None);
}

/// A build entered with less room than the last one in its shared history took is
/// gated on that measurement.
///
/// The behind-schedule latch cannot reach this case: it trips only after some
/// candidate has already overspent, so on a build that is short from the start
/// it arms too late to bound the candidate that spends the room.
#[test]
fn a_build_with_less_room_than_the_last_one_measured_is_gated() {
    use crate::decompose::portfolio::catalog::outspent;
    let was = Some(226_751);
    assert!(outspent(Some(150_000), was));
    // The boundary is "more room than", so equal room is not more room.
    assert!(outspent(Some(226_751), was));
    assert!(!outspent(Some(226_752), was));
}

/// A build with more room than the measurement is left alone, which is what
/// keeps a run whose builds fit the room left unchanged.
#[test]
fn a_build_with_more_room_than_the_last_one_measured_is_not_gated() {
    use crate::decompose::portfolio::catalog::outspent;
    assert!(!outspent(Some(3_500_000), Some(226_751)));
}

/// Without a measurement or without a deadline there is nothing to gate on.
#[test]
fn a_build_with_no_measurement_or_no_deadline_is_not_gated() {
    use crate::decompose::portfolio::catalog::outspent;
    assert!(!outspent(Some(150_000), None));
    assert!(!outspent(None, Some(226_751)));
    assert!(!outspent(Some(0), Some(226_751)));
    assert!(!outspent(Some(-5), Some(226_751)));
}

/// The truncation flag is the skip list, said the other way round: a build that
/// left a candidate unstarted is the truncated one. Asked of the rule rather
/// than of a build, so nothing here depends on how fast the machine is.
#[test]
fn a_build_that_left_a_candidate_unstarted_is_the_truncated_one() {
    use crate::decompose::portfolio::driver::limits_report;
    use std::time::Duration;

    let complete = limits_report(&[], Duration::from_millis(120));
    assert_eq!(complete.complete_builds, 1);
    assert_eq!(complete.truncated_builds, 0);
    assert_eq!(complete.spent_ms, 120);
    assert!(complete.skipped.is_empty());

    let truncated = limits_report(&["goatd-incidence", "hypergraph-bisect"], Duration::ZERO);
    assert_eq!(truncated.complete_builds, 0);
    assert_eq!(truncated.truncated_builds, 1);
    assert_eq!(
        truncated.skipped,
        vec![
            "goatd-incidence".to_string(),
            "hypergraph-bisect".to_string()
        ],
        "the candidates are named, in the order the catalog would have built them",
    );
}

/// A goatd entry asked for several trees publishes each runner-up as
/// `candidate=n` of its spec, and that spec rebuilds the same tree, so a
/// reader who took the name out of a bundle gets the tree the run ranked.
#[test]
fn a_goatd_runner_up_is_rebuilt_by_the_spec_it_publishes() {
    let formula = crate::tests::circuit_fixture::multiplier();
    let mut ctx = SelectionCtx::plain();
    ctx.goatd.candidates = 3;
    let limits = BuildLimits::default();
    let inp = super::inputs(&formula, &ctx, &limits, None);
    let mut run = RunState::new(PORTFOLIO_STEPS, PORTFOLIO_ITERS);
    let offered = CATALOG
        .iter()
        .find(|c| c.name == "goatd-incidence")
        .expect("goatd-incidence is a catalog entry")
        .offer(&inp, &mut run);
    assert!(
        offered.len() > 1,
        "the schedule offers a runner-up on this formula"
    );
    assert!(offered.len() <= 3, "no more trees than were asked for");
    for (index, built) in offered.iter().enumerate() {
        let spec = candidate_spec("goatd-incidence", candidate_param(index));
        let parsed = crate::spec::parse_vtree_spec(&spec).expect("the spec must parse");
        let standalone = crate::spec::build_one_vtree_artifacts(crate::spec::BuildRequest {
            formula: &formula,
            spec: &parsed,
            ctx: &SelectionCtx::plain(),
            limits: &BuildLimits::default(),
        })
        .unwrap_or_else(|e| panic!("{spec} must build: {e}"))
        .vtree;
        assert_eq!(
            standalone.to_vtree_text(),
            built.vtree.to_vtree_text(),
            "{spec} must rebuild the tree offered at index {index}"
        );
    }
}

#[test]
fn budgeted_goatd_keeps_time_to_convert_its_runner_ups() {
    use crate::decompose::goatd::{GoatdKnobs, vtrees_from_goatd_refined};
    use crate::decompose::td_to_vtree::ConversionRequest;
    use crate::decompose::{GraphKind, Reading, meter};

    let formula = crate::tests::circuit_fixture::multiplier();
    let _clock = meter::arm(std::time::Instant::now());
    let trees = vtrees_from_goatd_refined(
        &formula,
        GraphKind::Incidence,
        0,
        Some(200),
        GoatdKnobs::default(),
        false,
        ConversionRequest::open(Reading::default(), None),
    )
    .expect("budgeted construction");
    assert!(
        trees.len() > 1,
        "search consumed the runner-ups' conversion budget"
    );
}

#[test]
fn goatd_stops_runner_ups_at_the_outer_deadline() {
    use crate::decompose::goatd::{GoatdKnobs, vtrees_from_goatd_refined};
    use crate::decompose::td_to_vtree::ConversionRequest;
    use crate::decompose::{GraphKind, Reading, meter};

    let formula = crate::tests::circuit_fixture::multiplier();
    let epoch = std::time::Instant::now();
    let _clock = meter::arm(epoch);
    let trees = vtrees_from_goatd_refined(
        &formula,
        GraphKind::Incidence,
        0,
        Some(200),
        GoatdKnobs::default(),
        false,
        ConversionRequest {
            deadline: Some(epoch),
            ..ConversionRequest::open(Reading::default(), None)
        },
    )
    .expect("construction must return its first tree");
    assert_eq!(
        trees.len(),
        1,
        "expired construction cannot start runner-ups"
    );
}

#[test]
fn goatd_search_respects_an_outer_deadline_with_a_larger_override() {
    use crate::decompose::goatd::{GoatdKnobs, vtrees_from_goatd_refined};
    use crate::decompose::td_to_vtree::ConversionRequest;
    use crate::decompose::{GraphKind, Reading, meter};

    let formula = crate::tests::circuit_fixture::multiplier();
    let build = |budget| {
        let epoch = std::time::Instant::now();
        let _clock = meter::arm(epoch);
        let trees = vtrees_from_goatd_refined(
            &formula,
            GraphKind::Incidence,
            0,
            None,
            GoatdKnobs {
                refine_budget_ms: Some(budget),
                candidates: 1,
                ..GoatdKnobs::default()
            },
            false,
            ConversionRequest {
                deadline: Some(epoch + std::time::Duration::from_millis(20)),
                ..ConversionRequest::open(Reading::default(), None)
            },
        )
        .expect("construction must return its first tree");
        (
            trees[0].vtree.to_vtree_text(),
            meter::now().duration_since(epoch),
        )
    };
    assert_eq!(
        build(20),
        build(200),
        "the outer deadline bounds both allocations"
    );
}

#[test]
fn adaptive_goatd_preserves_its_converted_baseline_score() {
    use crate::decompose::goatd::{GoatdKnobs, GoatdPolishing, vtrees_from_goatd_refined};
    use crate::decompose::td_to_vtree::ConversionRequest;
    use crate::decompose::{GraphKind, Reading, meter};
    let formula = crate::tests::circuit_fixture::multiplier();
    let build = |policy| {
        let epoch = std::time::Instant::now();
        let _clock = meter::arm(epoch);
        vtrees_from_goatd_refined(
            &formula,
            GraphKind::Incidence,
            0,
            Some(200),
            GoatdKnobs {
                candidates: 1,
                polishing: policy,
                ..GoatdKnobs::default()
            },
            false,
            ConversionRequest::open(Reading::default(), None),
        )
        .unwrap()
        .remove(0)
    };
    let baseline = build(GoatdPolishing::adaptive(0, 0));
    let refined = build(GoatdPolishing::adaptive(8, 100));
    assert!(
        crate::score::vtree_cost(&refined.vtree, &formula).unwrap()
            <= crate::score::vtree_cost(&baseline.vtree, &formula).unwrap()
    );
    let legacy_off = build(GoatdPolishing::legacy(false, false));
    assert_eq!(
        baseline.vtree.to_vtree_text(),
        legacy_off.vtree.to_vtree_text()
    );
}

#[test]
fn a_distant_deadline_keeps_a_positive_construction_budget() {
    use crate::decompose::meter;
    let _meter = meter::arm(std::time::Instant::now());
    let formula = budget_fixture();
    let mut inputs = cap_gate_inputs(&formula, None);
    inputs.deadline = Some(meter::now() + std::time::Duration::from_millis(i64::MAX as u64 + 1));
    assert_eq!(inputs.remaining_ms(), Some(i64::MAX));
    assert!(!inputs.out_of_time());
    assert_eq!(inputs.fair_share_ms(4), Some(i64::MAX / 4));
}
