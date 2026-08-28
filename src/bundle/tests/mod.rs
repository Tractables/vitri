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

#[test]
fn count_stage1_is_retained_for_plain_mc_arjun_retries() {
    use crate::bundle::StageOutcome;
    use crate::cnf::Mode;

    assert!(super::retain_count_stage1(
        Mode::Mc,
        Some(&StageOutcome::Ran),
    ));
    assert!(!super::retain_count_stage1(
        Mode::Wmc,
        Some(&StageOutcome::Ran),
    ));
    assert!(!super::retain_count_stage1(Mode::Mc, None,));
}

#[test]
fn retry_budget_refuses_a_zero_arjun_window() {
    let error = super::RetryBudget::new(std::time::Instant::now(), std::time::Duration::ZERO)
        .expect_err("a zero Arjun allowance must not construct a retry budget");

    assert!(error.to_string().contains("non-zero Arjun budget"));
}

#[test]
fn a_discarded_arjun_retry_is_not_a_frontend_attempt() {
    use crate::bundle::{DiscardReason, StageOutcome};

    let outcome = StageOutcome::Discarded(DiscardReason::NotSmaller);

    assert!(
        !super::retry_produced_reduction(Some(&outcome)),
        "a discarded retry must stop before duplicate vtree construction",
    );
}

#[test]
fn frontend_retries_reuse_primary_simplification_and_apply_independent_policies() {
    use std::time::{Duration, Instant};

    use crate::bundle::{SkipReason, StageOutcome};
    use crate::cnf::{CnfMeta, Mode};
    use crate::config::{ArjunBudget, PreprocessStages, RunConfig};

    let formula = crate::tests::common::grid_fixture();
    let now = Instant::now();
    let config = RunConfig {
        deadline: Some(now + Duration::from_secs(5)),
        arjun_budget: ArjunBudget::Exact(Duration::from_millis(200)),
        stages: PreprocessStages {
            simplify: false,
            arjun: true,
        },
        mode: Some(Mode::Mc),
        ..RunConfig::default()
    };
    let meta = CnfMeta::default();
    let mut session = super::frontend(
        &formula,
        &meta,
        &config,
        &crate::decompose::SelectionCtx::plain(),
    )
    .expect("the retryable frontend session must be created");
    session
        .prepare()
        .expect("the primary attempt must retain its simplify checkpoint");
    assert!(
        session.count_stage1.is_some(),
        "the plain-MC primary must retain its Arjun checkpoint",
    );

    let reroll = session
        .retry(
            super::RetryBudget::new(
                Instant::now() + Duration::from_secs(4),
                Duration::from_millis(200),
            )
            .unwrap(),
            &super::FrontendRetryConfig::default(),
        )
        .expect("the same-policy reroll must finish through the retained checkpoint")
        .expect("the retained checkpoint makes this reroll eligible");
    assert_eq!(
        reroll.preprocessed.stages.sbva,
        Some(StageOutcome::Ran),
        "an omitted override must preserve the primary SBVA policy",
    );

    let retry = session
        .retry(
            super::RetryBudget::new(
                Instant::now() + Duration::from_secs(4),
                Duration::from_millis(200),
            )
            .unwrap(),
            &super::FrontendRetryConfig {
                arjun_sbva: Some(crate::preprocess::ArjunSbva::Off),
                vtree_spec: Some("minfill-primal".to_owned()),
            },
        )
        .expect("the independently configured retry must finish")
        .expect("the retained checkpoint makes this retry eligible");
    assert_eq!(
        retry.preprocessed.stages.sbva,
        Some(StageOutcome::Skipped(SkipReason::NotRequested)),
        "the retry must apply its independently selected SBVA policy",
    );
    let super::RunVtree::Built(built) = retry.vtree else {
        panic!("the fixture must leave a formula for vtree construction");
    };

    assert_eq!(
        built.selections[0].winning_spec.as_deref(),
        Some("minfill-primal"),
        "the retry must build the vtree spec supplied for that attempt",
    );
}
