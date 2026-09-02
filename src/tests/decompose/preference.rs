//! Steering portfolio selection toward a named candidate.
//!
//! The preference is a decision about which tree to take AWAY, not about what
//! to build: everything the portfolio does before the last step — the catalog
//! it walks, the gates it applies, the scores it computes — is the same either
//! way. The cases below are written as the caller's question, which is what it
//! got and whether it was told when it did not get it.

use crate::component::{VtreeBuild, build_vtree};
use crate::config::RunConfig;
use crate::decompose::{CandidatePreference, PortfolioKnobs, SelectionCtx};
use crate::error::VitriError;
use crate::tests::common::wide_component;

/// Build [`wide_component`] under `prefer`, which is the whole difference
/// between the cases here.
fn build_preferring(prefer: Option<CandidatePreference>) -> Result<VtreeBuild, VitriError> {
    let ctx = SelectionCtx {
        portfolio: PortfolioKnobs {
            prefer,
            ..PortfolioKnobs::default()
        },
        ..SelectionCtx::plain()
    };
    build_vtree(&wide_component(), &RunConfig::default(), &ctx)
}

/// Which construction a build settled on.
fn winner(build: &VtreeBuild) -> String {
    build.selections[0]
        .winning_spec
        .clone()
        .expect("a portfolio build names the candidate that won")
}

/// The two halves of the contract, on one fixture so they are the same
/// question asked of the same catalog: a candidate that built is selected even
/// though the scores preferred another, and a candidate that did NOT build
/// leaves the scores to decide.
///
/// Written over the whole catalog rather than against two hard-coded names: the
/// claim is about every candidate the portfolio offers, and a catalog that
/// gained an entry would otherwise leave it untested.
#[test]
fn a_preference_selects_the_candidate_it_names_when_that_candidate_built() {
    let on_score = winner(&build_preferring(None).expect("the fixture builds"));

    let mut honored = Vec::new();
    let mut did_not_build = Vec::new();
    for name in PortfolioKnobs::candidate_names() {
        let build = build_preferring(Some(CandidatePreference::Preferred(name.clone())))
            .expect("a preference never fails a build that would have succeeded");
        if winner(&build) == name {
            honored.push(name);
        } else {
            assert_eq!(
                winner(&build),
                on_score,
                "a preference that could not be met leaves selection exactly as it was",
            );
            did_not_build.push(name);
        }
    }

    assert!(
        honored.iter().any(|n| *n != on_score),
        "the preference has to be able to overturn the score, or it decides \
         nothing: honored {honored:?}, score picked {on_score}",
    );
    assert!(
        !did_not_build.is_empty(),
        "this fixture must keep at least one candidate that does not build over \
         it, or the fallback half of the contract is untested",
    );
}

/// The difference between the two preferences is entirely in what happens when
/// the candidate did not build: one takes the ordinary answer, the other would
/// rather fail than hand back a tree the caller did not ask for.
#[test]
fn a_required_candidate_that_did_not_build_is_an_error_rather_than_a_substitute() {
    let on_score = winner(&build_preferring(None).expect("the fixture builds"));
    let unbuildable = PortfolioKnobs::candidate_names()
        .into_iter()
        .find(|name| {
            let build = build_preferring(Some(CandidatePreference::Preferred(name.clone())))
                .expect("a soft preference never fails the build");
            winner(&build) != *name
        })
        .expect("this fixture keeps a candidate that does not build over it");

    let err = build_preferring(Some(CandidatePreference::Required(unbuildable.clone())))
        .expect_err("a required candidate that did not build must fail the build");
    assert!(
        matches!(err, VitriError::Construction { .. }),
        "the build ran and could not deliver, which is a construction failure: {err:?}",
    );
    assert!(
        err.to_string().contains(&unbuildable),
        "the error names the candidate that was required: {err}",
    );
    assert_ne!(
        on_score, unbuildable,
        "a candidate that did not build cannot also be the one the scores chose",
    );
}

/// A name no catalog entry answers to is refused where it is READ, not after a
/// whole construction budget has been spent selecting on score as if nothing
/// had been asked for. The message carries the names that would have worked,
/// because a caller that mistyped one is one edit away.
#[test]
fn an_unknown_candidate_name_is_refused_before_any_build_runs() {
    // A real vtree spec that no catalog entry builds, which is the mistake
    // worth catching: the name parses, so only the catalog can refuse it.
    let err = build_preferring(Some(CandidatePreference::Preferred(
        "minfill-primal".into(),
    )))
    .expect_err("an unknown candidate name must be refused");
    assert!(
        matches!(err, VitriError::Config { .. }),
        "the request is unanswerable, not the formula: {err:?}",
    );
    let text = err.to_string();
    for name in PortfolioKnobs::candidate_names() {
        assert!(
            text.contains(&name),
            "the refusal lists what the catalog does build, and is missing {name}: {text}",
        );
    }
}

/// The names a caller may ask for are the names a build reports back, so a
/// selection record read off one run can steer the next one with no
/// translation.
#[test]
fn every_name_a_build_can_report_is_a_name_a_preference_accepts() {
    let names = PortfolioKnobs::candidate_names();
    let reported = winner(&build_preferring(None).expect("the fixture builds"));
    assert!(
        names.contains(&reported),
        "a build reported {reported}, which a preference would refuse: {names:?}",
    );
}
