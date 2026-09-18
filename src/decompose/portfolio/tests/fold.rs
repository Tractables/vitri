//! The one fold: what a freshly scored candidate has to beat, and what
//! changes when it does.

use crate::cnf::CnfFormula;
use crate::decompose::portfolio::catalog::inputs::coloring_like_for_selection;
use crate::decompose::portfolio::catalog::{
    Build, CatalogEntry, Gate, Incumbent, Inputs, RunState, ScoredCandidate,
};
use crate::decompose::{
    ConversionRequest, Reading, SelectionCtx, TdConversion, TreeDecomposition, convert_td,
};
use crate::score::VtreeScores;
use crate::spec::builders::{PORTFOLIO_ITERS, PORTFOLIO_STEPS};
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
    CnfFormula::from_parts(
        6,
        vec![
            clause_dimacs(&[1, 2, 3]),
            clause_dimacs(&[2, -3]),
            clause_dimacs(&[3, 4]),
            clause_dimacs(&[4, 5, 6]),
            clause_dimacs(&[-5, 6]),
            clause_dimacs(&[1, -6]),
        ],
    )
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

/// Nothing here builds, so the fold is folded into the inputs of a plain run
/// under the default limits.
fn inputs(formula: &CnfFormula) -> Inputs<'_> {
    super::inputs(
        formula,
        &SelectionCtx::plain(),
        &crate::decompose::BuildLimits::default(),
        None,
    )
}

/// An incumbent already holding a tree, at the cost and spread the test wants
/// it to defend. Its other scores are the tree's own.
fn incumbent(
    vtree: Arc<crate::vtree::Vtree>,
    meta: Option<Arc<crate::decompose::BagMetadata>>,
    stats: VtreeScores,
    cost: f64,
    stddev: f64,
    param: Option<&'static str>,
) -> Incumbent {
    let mut best = Incumbent::default();
    best.adopt(ScoredCandidate {
        sel_metric: cost,
        stats: VtreeScores {
            cost,
            clause_load_stddev: stddev,
            ..stats
        },
        agg: None,
        name: "incumbent",
        param,
        vtree,
        meta,
    });
    best
}

/// The fold is handed what was built, so an entry's builder is never reached
/// from here.
fn builder_not_reached(_: &Inputs, _: &mut RunState, _: &CatalogEntry) -> Vec<TdConversion> {
    unreachable!("fold folds a candidate that is already built")
}

fn entry(td_based: bool) -> CatalogEntry {
    CatalogEntry {
        name: "challenger",
        param: Some("challenger-param"),
        td_based,
        offers: 1,
        gate: Gate::Always,
        build: Build::Own(builder_not_reached),
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

    let mut run = RunState::new(PORTFOLIO_STEPS, PORTFOLIO_ITERS);
    run.best = incumbent(
        convert(&formula, &wide_td()).vtree,
        None,
        scores,
        incumbent_cost,
        incumbent_stddev,
        None,
    );
    run.fold(&inputs(&formula), &entry(true), 0, convert(&formula, &td));
    assert_eq!(run.best.name(), "incumbent");
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

    let mut run = RunState::new(PORTFOLIO_STEPS, PORTFOLIO_ITERS);
    run.best = incumbent(
        Arc::clone(&loser_vtree),
        Some(Arc::clone(&loser_meta)),
        scores,
        scores.cost + 1.0,
        scores.clause_load_stddev + 1.0,
        Some("incumbent-param"),
    );
    let inp = inputs(&formula);
    run.fold(&inp, &entry(true), 0, built);

    let adopted = run
        .best
        .candidate
        .as_ref()
        .expect("a candidate was adopted");
    assert_eq!(adopted.name, "challenger", "the name is the challenger's");
    assert_eq!(
        adopted.param,
        Some("challenger-param"),
        "the parameter is the challenger's",
    );
    assert_eq!(
        adopted.stats, scores,
        "every score is the challenger's, the spread and the cost with them",
    );
    assert!(
        Arc::ptr_eq(&adopted.vtree, &challenger_vtree),
        "the tree is the challenger's",
    );
    assert!(
        Arc::ptr_eq(
            adopted.meta.as_ref().expect("metadata was adopted"),
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

    let mut run = RunState::new(PORTFOLIO_STEPS, PORTFOLIO_ITERS);
    run.best = incumbent(
        Arc::clone(&loser.vtree),
        Some(loser.td.meta.clone().expect("the losing tree has metadata")),
        scores,
        scores.cost + 1.0,
        scores.clause_load_stddev + 1.0,
        None,
    );
    let inp = inputs(&formula);
    run.fold(&inp, &entry(false), 0, built);

    assert_eq!(run.best.name(), "challenger", "the challenger was adopted");
    assert!(
        run.best
            .candidate
            .as_ref()
            .expect("a candidate was adopted")
            .meta
            .is_none(),
        "nothing describes the adopted tree's bags, so the incumbent carries none",
    );
}

/// A tree past the first an entry offered is published under its index, not
/// the entry's own parameter, so no two trees of one entry share a name.
#[test]
fn a_runner_up_is_named_by_its_index() {
    let formula = formula();
    let td = crate::tests::td_fixture::make_test_td();
    let mut run = RunState::new(PORTFOLIO_STEPS, PORTFOLIO_ITERS);
    run.fold(&inputs(&formula), &entry(true), 2, convert(&formula, &td));
    assert_eq!(run.best.name(), "challenger");
    assert_eq!(
        run.best
            .candidate
            .as_ref()
            .expect("a candidate was adopted")
            .param,
        Some("candidate=2")
    );
}
