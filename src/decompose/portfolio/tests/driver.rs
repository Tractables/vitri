//! Selection pins and construction-budget guarantees for the portfolio driver.

use crate::decompose::BuildLimits;
use crate::decompose::Reading;
use crate::decompose::SelectionCtx;
use crate::decompose::portfolio::catalog::Inputs;
use crate::decompose::portfolio::catalog::RunState;
use crate::decompose::portfolio::catalog::ScoredCandidate;
use crate::decompose::portfolio::catalog::build_fc_inc;
use crate::decompose::portfolio::catalog::build_guided_bisect;
use crate::decompose::portfolio::catalog::candidate_spec;
use crate::decompose::portfolio::driver::*;
use crate::score::VtreeScores;
use crate::vtree::Vtree;
use std::sync::Arc;

/// Peak-mode (projected) selection pin. The golden-trace tests pin the
/// PLAIN-MC selection path, but `peak_mode` (the blended peak-context-width
/// band selection) is unreachable through the CLI —
/// the projected driver decomposes and builds each component in plain mode. So
/// the projected selection path is pinned HERE by passing a peak-active
/// `SelectionCtx` directly and asserting the winner.
/// Candidate metrics drift run-to-run, but on this small fixture the
/// DECISION is stable; if this ever flakes, the winner set is tiny
/// (flowcutter-incidence/flowcutter-primal/goatd/hypergraph-bisect/
/// guided-bisect) — investigate, do not just relax it.
///
/// The expected winner is `hypergraph-bisect` at the portfolio's relaxed
/// imbalance on the generated multiplier fixture. It is a property of the
/// fixture, not a target: regenerating the fixture at a different width means
/// re-observing this, never editing it to match a one-off run. Peak-mode ranks
/// by context width while the conversion searches on cost, so a decomposition
/// candidate's peak width moves when the reading it settles on moves.
#[test]
fn peak_mode_selection_pin() {
    let formula = crate::tests::circuit_fixture::multiplier();
    // Same portfolio params as the `portfolio` spec builds with (150_000/15/0).
    let built = vtree_from_portfolio(
        &formula,
        150_000,
        15,
        Reading::default(),
        &SelectionCtx::peak(),
        &BuildLimits::default(),
    )
    .expect("portfolio");
    assert_eq!(
        built.selection.winning_spec.as_deref(),
        Some("hypergraph-bisect:imbalance=0.40"),
        "peak-mode selection changed"
    );
}

/// Scoring purity pin: `VtreeScores::compute` is a pure function of a fixed
/// `(vtree, formula, show_mask)`, so computing it twice on the SAME realized
/// vtree yields identical fields. This isolates scoring determinism from the
/// decomposition-search nondeterminism that `peak_mode_selection_pin`
/// tolerates: here the vtree is fixed, so all five metrics must be
/// bit-for-bit reproducible.
#[test]
fn realized_stats_compute_twice_equal() {
    let formula = crate::tests::circuit_fixture::multiplier();
    // One deterministic realized vtree, converted straight off a
    // flowcutter-incidence decomposition.
    let td = crate::decompose::flowcutter::flowcutter_td(
        &formula,
        crate::decompose::GraphKind::Incidence,
        crate::decompose::FcBudget::Steps {
            steps: 150_000,
            iters: 15,
        },
    )
    .expect("flowcutter-incidence TD");
    let vtree = crate::decompose::td_to_vtree_reading(
        &td,
        formula.num_vars,
        Reading::default(),
        Some(&formula),
        None,
    );
    let a = VtreeScores::compute(&vtree, &formula, None).expect("vtree covers the formula");
    let b = VtreeScores::compute(&vtree, &formula, None).expect("vtree covers the formula");
    assert_eq!(
        a.clause_load_stddev, b.clause_load_stddev,
        "stddev not reproducible"
    );
    assert_eq!(
        a.max_clause_load, b.max_clause_load,
        "max_clause_load not reproducible"
    );
    assert_eq!(
        a.peak_context_width_all, b.peak_context_width_all,
        "peak_context_width_all not reproducible"
    );
    assert_eq!(
        a.peak_context_width_show, b.peak_context_width_show,
        "peak_context_width_show not reproducible"
    );
    assert_eq!(a.cost, b.cost, "cost not reproducible");
}

fn budget_fixture() -> crate::cnf::CnfFormula {
    crate::tests::circuit_fixture::multiplier()
}

/// SPENT BUDGET IS A HARD ERROR: a deadline that has ALREADY passed on entry
/// skips every catalog candidate, and the build fails outright rather than
/// handing back a degraded vtree.
#[test]
fn expired_deadline_is_a_construction_error() {
    use std::time::{Duration, Instant};
    let formula = budget_fixture();
    let limits = BuildLimits {
        deadline: Some(Instant::now() - Duration::from_secs(1)),
        ..BuildLimits::default()
    };
    // Matched by hand rather than `.expect_err()`: `VtreeArtifacts` (the `Ok`
    // side) carries an `Arc<Vtree>` and does not derive `Debug`, which
    // `.expect_err()`'s bound would otherwise require adding just for this test.
    match vtree_from_portfolio(
        &formula,
        150_000,
        15,
        Reading::default(),
        &SelectionCtx::plain(),
        &limits,
    ) {
        Ok(_) => panic!("an already-spent deadline must fail construction, not build a vtree"),
        Err(e) => assert!(
            matches!(e, crate::error::VitriError::Construction { .. }),
            "expected a construction error, got {e:?}",
        ),
    }
}

/// NO BEHAVIOR DRIFT: a deadline far beyond what construction needs must
/// produce the SAME vtree as no deadline at all — compared structurally
/// (`to_vtree_text`), not just by winner name.
///
/// Under a deadline every entry is now bounded at the time left when it starts,
/// so this fixture runs the bound-only FlowCutter path end to end rather than
/// the untimed one. The equality is what says that path searches identically;
/// `a_bound_only_wall_the_build_never_reaches_decomposes_exactly_as_no_wall_does`
/// pins the same property at the FlowCutter layer, on a component large enough
/// for the tight gates to matter.
#[test]
fn generous_deadline_matches_no_deadline() {
    use std::time::{Duration, Instant};
    let formula = budget_fixture();
    let unbounded = vtree_from_portfolio(
        &formula,
        150_000,
        15,
        Reading::default(),
        &SelectionCtx::plain(),
        &BuildLimits::default(),
    )
    .expect("portfolio (no deadline)");
    let limits = BuildLimits {
        deadline: Some(Instant::now() + Duration::from_secs(3600)),
        ..BuildLimits::default()
    };
    let bounded = vtree_from_portfolio(
        &formula,
        150_000,
        15,
        Reading::default(),
        &SelectionCtx::plain(),
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
}

/// Tiny dummy ScoredCandidate (the vtree is never inspected by select_peak_band).
fn sc(sel_metric: f64, clause_load_stddev: f64, cost: u64, name: &'static str) -> ScoredCandidate {
    ScoredCandidate {
        sel_metric,
        stats: VtreeScores {
            clause_load_stddev,
            max_clause_load: 0,
            peak_context_width_all: sel_metric as u32,
            peak_context_width_show: None,
            cost,
        },
        name,
        param: None,
        vtree: Arc::new(Vtree::balanced(2)),
        meta: None,
    }
}

/// Band selection: among candidates within the peak band it picks minimum
/// stddev, and it never reaches a lower-stddev candidate that falls OUTSIDE the
/// band.
#[test]
fn select_peak_band_default_min_stddev_within_band() {
    // min_peak = 10.0, rel_tol = 0.10 → band = 11.0.
    let cands = vec![
        sc(10.0, 8.0, 100, "in_hi_stddev"), // in band, higher stddev
        sc(11.0, 4.0, 100, "in_lo_stddev"), // in band (11.0 <= 11.0), lower stddev → winner
        sc(20.0, 1.0, 100, "out_lowest"),   // out of band; lowest stddev but excluded
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
    use std::time::Instant;

    // Both sides scale their FlowCutter effort from the budget hint in the
    // build limits, and the default leaves it unset, so the two coincide
    // whatever the environment holds.
    let formula = crate::tests::circuit_fixture::multiplier();
    let ctx = SelectionCtx::plain();
    let limits = BuildLimits::default();
    let inp = Inputs {
        formula: &formula,
        seed: ctx.portfolio.seed,
        peak_mode: false,
        show_mask: None,
        trace: false,
        flowcutter_cap_ms: None,
        t_build: Instant::now(),
        deadline: None,
        candidate_capacity: limits.candidates,
        peak_tolerance: ctx.portfolio.peak_tolerance,
        goatd: ctx.goatd,
        rank_metric: crate::candidates::CandidateRankMetric::ClauseLoadStddev,
        effort_scale: crate::budget::vtree_effort_scale(limits.budget_ms),
        reading: Reading::default(),
        conversion_trace: false,
    };
    // Same effort the `portfolio` spec builds with, which is what lets a spec
    // naming that effort literally reproduce these trees.
    let mut run = RunState::new(150_000, 15);
    build_fc_inc(&inp, &mut run).expect("the flowcutter-incidence candidate must build");
    let guided =
        build_guided_bisect(&inp, &mut run).expect("the guided-bisect candidate must build");

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
    for c in catalog() {
        assert_ne!(
            crate::spec::classify_base(c.name),
            crate::spec::VtreeBase::Unknown,
            "catalog candidate '{}' names no buildable family",
            c.name,
        );
        let spec = candidate_spec(c.name, c.param);
        crate::spec::validate_vtree_spec(&spec).unwrap_or_else(|e| {
            panic!(
                "'{spec}' does not rebuild catalog candidate '{}': {e}",
                c.name
            )
        });
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

    let c = catalog()
        .into_iter()
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
        crate::spec::SpecParam::Imbalance(v) => assert_eq!(v, IMBALANCE_PORTFOLIO_RELAXED),
        _ => panic!("the bisection spec's param is an imbalance"),
    }
}

/// A minimal `Inputs` over the budget fixture, with the two fields the cap
/// gates read left to the caller.
fn cap_gate_inputs<'a>(
    formula: &'a crate::cnf::CnfFormula,
    flowcutter_cap_ms: Option<i64>,
) -> Inputs<'a> {
    use std::time::Instant;
    let ctx = SelectionCtx::plain();
    let limits = BuildLimits::default();
    Inputs {
        formula,
        seed: ctx.portfolio.seed,
        peak_mode: false,
        show_mask: None,
        trace: false,
        flowcutter_cap_ms,
        t_build: Instant::now(),
        deadline: None,
        candidate_capacity: limits.candidates,
        peak_tolerance: ctx.portfolio.peak_tolerance,
        goatd: ctx.goatd,
        rank_metric: crate::candidates::CandidateRankMetric::ClauseLoadStddev,
        effort_scale: crate::budget::vtree_effort_scale(limits.budget_ms),
        reading: Reading::default(),
        conversion_trace: false,
    }
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
    let mut run = RunState::new(150_000, 15);
    run.cand_wall_ms = Some(5_000);
    run.cand_cap_ms = Some(1_000);
    assert_eq!(run.fc_time_cap_ms(&inp), Some(5_000));
}

/// A wall the build is expected to finish inside leaves the search alone.
#[test]
fn a_wall_armed_on_a_healthy_build_is_bound_only() {
    let formula = budget_fixture();
    let inp = cap_gate_inputs(&formula, None);
    let mut run = RunState::new(150_000, 15);
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
    let mut run = RunState::new(150_000, 15);
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
    let mut run = RunState::new(150_000, 15);
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
    let run = RunState::new(150_000, 15);
    assert_eq!(run.fc_time_cap_ms(&inp), None);
}

/// A build entered with less room than the last one in this process took is
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
