//! The one fold: what a freshly scored candidate has to beat, and what
//! changes when it does.

use crate::candidates::CandidateRankMetric;
use crate::cnf::CnfFormula;
use crate::decompose::flowcutter::built_from_td_best;
use crate::decompose::portfolio::catalog::{
    AdoptRule, CatalogEntry, Derived, Gate, Incumbent, Inputs, RunState,
};
use crate::decompose::{SelectionCtx, TdConversion, TreeDecomposition};
use crate::score::VtreeScores;
use crate::tests::common::{clause_dimacs, make_td};
use std::sync::Arc;

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

fn inputs(formula: &CnfFormula) -> Inputs<'_> {
    Inputs {
        formula,
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
        rank_metric: CandidateRankMetric::ClauseLoadStddev,
        effort_scale: 1.0,
    }
}

/// The fold is handed what was built, so an entry's builder is never reached
/// from here.
fn builder_not_reached(_: &Inputs, _: &mut RunState) -> Option<TdConversion> {
    unreachable!("fold folds a candidate that is already built")
}

fn entry(adopt: AdoptRule, td_based: bool) -> CatalogEntry {
    CatalogEntry {
        name: "challenger",
        param: Some("challenger-param"),
        td_based,
        gate: Gate::Always,
        build: builder_not_reached,
        adopt,
    }
}

/// One candidate, strictly the tighter spread and strictly the costlier tree,
/// meets each of the three adoption rules in turn: the spread rule takes it,
/// the joint rule holds the cost against it, and the gated rule refuses a
/// formula that is not coloring-like however good the scores are.
///
/// The rule is the only thing that differs between the three folds, so the
/// three answers are the rules disagreeing and nothing else.
#[test]
fn the_three_adoption_rules_disagree_on_one_challenger() {
    let formula = formula();
    let td = crate::tests::td_fixture::make_test_td();
    let scores = {
        let built = built_from_td_best(&formula, &td, 1.0);
        VtreeScores::compute(&built.vtree, &formula, None).expect("the tree covers the formula")
    };

    // An incumbent the challenger out-spreads and undercuts on nothing else.
    let incumbent_stddev = scores.clause_load_stddev + 1.0;
    let incumbent_cost = scores.cost.saturating_sub(1);
    assert!(
        scores.cost > incumbent_cost,
        "the challenger has to be the costlier tree for the joint rule to have \
         something to refuse",
    );
    assert!(
        scores.clause_load_stddev < incumbent_stddev * 0.9,
        "the challenger has to clear the gated rule's spread margin, so that a \
         refusal there is the gate and not the margin",
    );

    // Nothing adopted has an MCL, so the gated rule's first two conjuncts hold
    // and `coloring_like` is what it turns on.
    let derived = Derived {
        coloring_like: false,
        best_mcl: None,
        hypergraph_bisect_gen_gate: true,
    };

    let cases: [(&str, AdoptRule, bool); 3] = [
        ("the spread rule", AdoptRule::MinStddev, true),
        (
            "the joint spread-and-cost rule",
            AdoptRule::JointStddevCost,
            false,
        ),
        ("the coloring-gated rule", AdoptRule::ColoringGated, false),
    ];

    for (label, adopt, adopts) in cases {
        let mut run = RunState::new(150_000, 15);
        run.best = Incumbent {
            stddev: incumbent_stddev,
            cost: incumbent_cost,
            vtree: Some(built_from_td_best(&formula, &wide_td(), 1.0).vtree),
            meta: None,
            name: "incumbent",
            param: None,
        };
        let inp = inputs(&formula);
        run.fold(
            &inp,
            Some(&derived),
            &entry(adopt, true),
            built_from_td_best(&formula, &td, 1.0),
        );
        assert_eq!(
            run.best.name,
            if adopts { "challenger" } else { "incumbent" },
            "{label} was asked about a challenger of tighter spread and higher cost",
        );
    }
}

/// Adoption swaps the whole incumbent: scores, tree, metadata, name and
/// parameter all become the challenger's. A field left behind would describe
/// the tree that lost — bag metadata for a tree nobody holds any more, or a
/// name that no longer spells the spec rebuilding what was selected.
#[test]
fn an_adopted_incumbent_replaces_every_field_at_once() {
    let formula = formula();
    let td = crate::tests::td_fixture::make_test_td();
    let built = built_from_td_best(&formula, &td, 1.0);
    let challenger_vtree = Arc::clone(&built.vtree);
    let challenger_meta = built
        .td
        .meta
        .clone()
        .expect("a decomposition conversion carries the bag metadata of its tree");
    let scores = VtreeScores::compute(&challenger_vtree, &formula, None)
        .expect("the tree covers the formula");

    let loser = built_from_td_best(&formula, &wide_td(), 1.0);
    let loser_vtree = Arc::clone(&loser.vtree);
    let loser_meta = loser.td.meta.clone().expect("the same, for the other tree");

    let mut run = RunState::new(150_000, 15);
    run.best = Incumbent {
        stddev: scores.clause_load_stddev + 1.0,
        cost: scores.cost + 1,
        vtree: Some(Arc::clone(&loser_vtree)),
        meta: Some(Arc::clone(&loser_meta)),
        name: "incumbent",
        param: Some("incumbent-param"),
    };
    let inp = inputs(&formula);
    run.fold(&inp, None, &entry(AdoptRule::MinStddev, true), built);

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
    let built = built_from_td_best(&formula, &td, 1.0);
    let scores =
        VtreeScores::compute(&built.vtree, &formula, None).expect("the tree covers the formula");
    let loser = built_from_td_best(&formula, &wide_td(), 1.0);

    let mut run = RunState::new(150_000, 15);
    run.best = Incumbent {
        stddev: scores.clause_load_stddev + 1.0,
        cost: scores.cost + 1,
        vtree: Some(Arc::clone(&loser.vtree)),
        meta: Some(loser.td.meta.clone().expect("the losing tree has metadata")),
        name: "incumbent",
        param: None,
    };
    let inp = inputs(&formula);
    run.fold(&inp, None, &entry(AdoptRule::MinStddev, false), built);

    assert_eq!(run.best.name, "challenger", "the challenger was adopted");
    assert!(
        run.best.meta.is_none(),
        "nothing describes the adopted tree's bags, so the incumbent carries none",
    );
}
