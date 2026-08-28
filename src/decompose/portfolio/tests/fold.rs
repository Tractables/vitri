//! The one fold: what a freshly scored candidate has to beat, and what
//! changes when it does.

use crate::candidates::CandidateRankMetric;
use crate::cnf::CnfFormula;
use crate::decompose::portfolio::catalog::{
    CatalogEntry, Gate, Incumbent, Inputs, RunState, coloring_like_for_selection,
};
use crate::decompose::{
    ConversionRequest, Reading, SelectionCtx, TdConversion, TreeDecomposition, convert_td,
};
use crate::score::VtreeScores;
use crate::tests::common::{clause_dimacs, make_td};
use std::sync::Arc;

fn structure_profile(occurrence_cv: f64, width_cv: f64) -> crate::score::StructureProfile {
    crate::score::StructureProfile::from_coefficients(width_cv, occurrence_cv)
}

#[test]
fn missing_source_profile_preserves_the_built_formula_gate() {
    let reduced_ok = structure_profile(0.2, 0.2);
    let reduced_wide = structure_profile(0.2, 0.6);

    assert_eq!(
        coloring_like_for_selection(reduced_ok, None),
        reduced_ok.coloring_like,
        "without a source profile the existing reduced-formula gate is unchanged",
    );
    assert_eq!(
        coloring_like_for_selection(reduced_wide, None),
        reduced_wide.coloring_like,
        "a missing source profile cannot relax the reduced-formula gate",
    );
}

#[test]
fn source_clause_width_can_enable_the_structure_gate() {
    let reduced_wide = structure_profile(0.2, 0.6);
    let source_narrow = structure_profile(0.1, 0.2);

    assert!(
        coloring_like_for_selection(reduced_wide, Some(source_narrow)),
        "a source formula's narrow clause-width dispersion may supply the width signal",
    );
}

#[test]
fn source_occurrence_cannot_enable_the_structure_gate() {
    let reduced_occurrence_skewed = structure_profile(0.8, 0.6);
    let source_narrow = structure_profile(0.1, 0.2);

    assert!(
        !coloring_like_for_selection(reduced_occurrence_skewed, Some(source_narrow)),
        "the reduced formula's occurrence dispersion remains authoritative",
    );
}

/// Six variables tied together unevenly, so the trees below score apart
/// instead of landing on one number.
fn formula() -> CnfFormula {
    CnfFormula {
        num_vars: 6,
        clauses: vec![
            clause_dimacs(&[1, 2, 3]),
            clause_dimacs(&[2, -3]),
            clause_dimacs(&[3, 4]),
            clause_dimacs(&[4, 5, 6]),
            clause_dimacs(&[-5, 6]),
            clause_dimacs(&[1, -6]),
        ],
    }
}

/// A second decomposition of the same six variables, so the incumbent below
/// holds a tree and metadata of its own rather than starting empty.
fn wide_td() -> TreeDecomposition {
    make_td(vec![vec![0, 1, 2, 3], vec![3, 4, 5]], vec![(0, 1)], 6)
}

/// One decomposition converted the way a candidate's builder converts it:
/// the whole search, unbounded, reporting nothing.
fn convert(formula: &CnfFormula, td: &TreeDecomposition) -> TdConversion {
    convert_td(
        formula,
        td,
        ConversionRequest::open(Reading::default(), None),
    )
}

fn inputs(formula: &CnfFormula) -> Inputs<'_> {
    Inputs {
        formula,
        source_profile: None,
        seed: 0,
        peak_mode: false,
        show_mask: None,
        trace: false,
        flowcutter_cap_ms: None,
        t_build: std::time::Instant::now(),
        deadline: None,
        candidate_capacity: 0,
        peak_tolerance: 0.1,
        goatd: SelectionCtx::plain().goatd,
        rank_metric: CandidateRankMetric::Cost,
        effort_scale: 1.0,
        reading: Reading::default(),
        conversion_trace: false,
        prefer: None,
    }
}

/// The fold is handed what was built, so an entry's builder is never reached
/// from here.
fn builder_not_reached(_: &Inputs, _: &mut RunState) -> Option<TdConversion> {
    unreachable!("fold folds a candidate that is already built")
}

fn entry(td_based: bool) -> CatalogEntry {
    CatalogEntry {
        name: "challenger",
        param: Some("challenger-param"),
        td_based,
        gate: Gate::Always,
        build: builder_not_reached,
    }
}

/// The unified selector does not substitute clause-load spread for cost: a
/// challenger with tighter spread but higher cost remains the loser.
#[test]
fn a_costlier_challenger_is_not_adopted_for_its_spread() {
    let formula = formula();
    let td = crate::tests::td_fixture::make_test_td();
    let scores = {
        let built = convert(&formula, &td);
        VtreeScores::compute(&built.vtree, &formula, None).expect("the tree covers the formula")
    };

    // An incumbent the challenger out-spreads and undercuts on nothing else.
    let incumbent_stddev = scores.clause_load_stddev + 1.0;
    let incumbent_cost = scores.cost - 1.0;
    assert!(scores.cost > incumbent_cost, "the challenger is costlier");
    assert!(
        scores.clause_load_stddev < incumbent_stddev,
        "the challenger has tighter spread",
    );

    let mut run = RunState::new(150_000, 15);
    run.best = Incumbent {
        scores: None,
        stddev: incumbent_stddev,
        cost: incumbent_cost,
        vtree: Some(convert(&formula, &wide_td()).vtree),
        meta: None,
        name: "incumbent",
        param: None,
    };
    run.fold(&inputs(&formula), &entry(true), convert(&formula, &td));
    assert_eq!(run.best.name, "incumbent");
}

/// Adoption swaps the whole incumbent: scores, tree, metadata, name and
/// parameter all become the challenger's. A field left behind would describe
/// the tree that lost — bag metadata for a tree nobody holds any more, or a
/// name that no longer spells the spec rebuilding what was selected.
#[test]
fn an_adopted_incumbent_replaces_every_field_at_once() {
    let formula = formula();
    let td = crate::tests::td_fixture::make_test_td();
    let built = convert(&formula, &td);
    let challenger_vtree = Arc::clone(&built.vtree);
    let challenger_meta = built
        .td
        .meta
        .clone()
        .expect("a decomposition conversion carries the bag metadata of its tree");
    let scores = VtreeScores::compute(&challenger_vtree, &formula, None)
        .expect("the tree covers the formula");

    let loser = convert(&formula, &wide_td());
    let loser_vtree = Arc::clone(&loser.vtree);
    let loser_meta = loser.td.meta.clone().expect("the same, for the other tree");

    let mut run = RunState::new(150_000, 15);
    run.best = Incumbent {
        scores: None,
        stddev: scores.clause_load_stddev + 1.0,
        cost: scores.cost + 1.0,
        vtree: Some(Arc::clone(&loser_vtree)),
        meta: Some(Arc::clone(&loser_meta)),
        name: "incumbent",
        param: Some("incumbent-param"),
    };
    let inp = inputs(&formula);
    run.fold(&inp, &entry(true), built);

    assert_eq!(run.best.name, "challenger", "the name is the challenger's");
    assert_eq!(
        run.best.param,
        Some("challenger-param"),
        "the parameter is the challenger's",
    );
    assert_eq!(
        run.best.stddev, scores.clause_load_stddev,
        "the spread is the challenger's",
    );
    assert_eq!(run.best.cost, scores.cost, "the cost is the challenger's");
    assert_eq!(
        run.best.scores,
        Some(scores),
        "all scores are the challenger's"
    );
    assert!(
        Arc::ptr_eq(
            run.best.vtree.as_ref().expect("a tree was adopted"),
            &challenger_vtree,
        ),
        "the tree is the challenger's",
    );
    assert!(
        Arc::ptr_eq(
            run.best.meta.as_ref().expect("metadata was adopted"),
            &challenger_meta,
        ),
        "the metadata is the challenger's",
    );
}

/// A family that does not hand back a decomposition describing the tree it
/// built adopts without metadata, and the incumbent's metadata goes with the
/// incumbent. Keeping it would leave bag metadata of one tree attached to
/// another.
#[test]
fn adopting_a_candidate_no_decomposition_describes_clears_the_bag_metadata() {
    let formula = formula();
    let td = crate::tests::td_fixture::make_test_td();
    let built = convert(&formula, &td);
    let scores =
        VtreeScores::compute(&built.vtree, &formula, None).expect("the tree covers the formula");
    let loser = convert(&formula, &wide_td());

    let mut run = RunState::new(150_000, 15);
    run.best = Incumbent {
        scores: None,
        stddev: scores.clause_load_stddev + 1.0,
        cost: scores.cost + 1.0,
        vtree: Some(Arc::clone(&loser.vtree)),
        meta: Some(loser.td.meta.clone().expect("the losing tree has metadata")),
        name: "incumbent",
        param: None,
    };
    let inp = inputs(&formula);
    run.fold(&inp, &entry(false), built);

    assert_eq!(run.best.name, "challenger", "the challenger was adopted");
    assert!(
        run.best.meta.is_none(),
        "nothing describes the adopted tree's bags, so the incumbent carries none",
    );
}
