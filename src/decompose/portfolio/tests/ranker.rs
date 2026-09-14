//! `PortfolioKnobs::ranker`: the caller's switch over the aggregate ranker.

use crate::component::build_vtree;
use crate::config::RunConfig;
use crate::decompose::portfolio::PortfolioKnobs;
use crate::decompose::{PairwiseWeighting, SelectionCtx};
use crate::tests::common::wide_component;

#[test]
fn the_ranker_follows_the_environment_by_default() {
    assert!(PortfolioKnobs::default().ranker);
}

#[test]
fn a_caller_that_turned_the_ranker_off_keeps_it_off_through_the_env_defaults() {
    let knobs = PortfolioKnobs {
        ranker: false,
        ..PortfolioKnobs::default()
    }
    .with_env_defaults()
    .expect("no portfolio variable is set in the test environment");
    assert!(!knobs.ranker);
}

#[test]
fn a_build_with_the_ranker_off_still_selects_a_candidate() {
    let ctx = SelectionCtx {
        portfolio: PortfolioKnobs {
            ranker: false,
            ..PortfolioKnobs::default()
        },
        ..SelectionCtx::plain()
    };
    let build = build_vtree(&wide_component(), &RunConfig::default(), &ctx)
        .expect("the wide component builds");
    assert!(build.selections[0].winning_spec.is_some());
}

#[test]
fn pairwise_weighting_defaults_to_candidates_and_preserves_caller_choice() {
    assert_eq!(
        PortfolioKnobs::default().pairwise_weighting,
        PairwiseWeighting::Candidate
    );
    let knobs = PortfolioKnobs {
        pairwise_weighting: PairwiseWeighting::Family,
        ..PortfolioKnobs::default()
    }
    .with_env_defaults()
    .expect("portfolio defaults are valid");
    assert_eq!(knobs.pairwise_weighting, PairwiseWeighting::Family);
}
