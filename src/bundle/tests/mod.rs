//! Tests of items `bundle` keeps to itself. Everything reachable from the
//! crate root is tested from `src/tests/bundle/` instead; this tree is for the
//! `pub(super)` and `pub(crate)` decisions the chains make internally, which
//! the privacy rules put out of reach from there.

mod chains;
mod simplify_policy;
mod stage_budget;

#[test]
fn frontend_creation_anchors_the_deadline_before_prepare() {
    use std::time::{Duration, Instant};

    let (formula, meta) = crate::tests::common::parse(crate::tests::common::FULLY_RESOLVED);
    let now = Instant::now();
    let config = crate::config::RunConfig {
        budget_ms: Some(60_000),
        ..crate::config::RunConfig::default()
    };
    let mut session = super::frontend_at(
        &formula,
        &meta,
        &config,
        &crate::decompose::SelectionCtx::plain(),
        now,
    )
    .expect("the frontend session must be created");
    let anchored = Some(now + Duration::from_millis(60_000));

    assert_eq!(
        session.config.deadline, anchored,
        "the relative budget must become an absolute deadline at session creation",
    );
    session.prepare().expect("the primary attempt must prepare");
    assert_eq!(
        session.config.deadline, anchored,
        "prepare must consume the session's existing deadline, not start a new budget",
    );
}

#[test]
fn full_run_selection_uses_the_profile_owned_by_the_run() {
    let measured = crate::score::StructureProfile::from_coefficients(0.25, 0.5);
    let wrong = crate::score::StructureProfile::from_coefficients(9.0, 9.0);
    let mut caller = crate::decompose::SelectionCtx::plain();
    caller.source_profile = Some(wrong);

    let selection = super::run_selection(&caller, measured, None, 4);

    assert_eq!(
        selection.source_profile,
        Some(measured),
        "the construction path must receive the profile measured by run",
    );
}
