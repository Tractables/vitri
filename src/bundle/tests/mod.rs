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
fn count_stage1_is_retained_only_for_the_plain_mc_coloring_retry_policy() {
    use crate::bundle::StageOutcome;
    use crate::cnf::Mode;
    use crate::score::StructureProfile;

    let coloring = StructureProfile::from_coefficients(0.01, 0.01);
    let non_coloring = StructureProfile::from_coefficients(1.0, 1.0);

    assert!(super::retain_count_stage1(
        coloring,
        Mode::Mc,
        Some(&StageOutcome::Ran),
    ));
    assert!(!super::retain_count_stage1(
        non_coloring,
        Mode::Mc,
        Some(&StageOutcome::Ran),
    ));
    assert!(!super::retain_count_stage1(
        coloring,
        Mode::Wmc,
        Some(&StageOutcome::Ran),
    ));
    assert!(!super::retain_count_stage1(coloring, Mode::Mc, None,));
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
fn frontend_retry_reuses_primary_simplification_and_disables_sbva() {
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
        "the coloring-like plain-MC primary actually ran SBVA",
    );

    let retry = session
        .retry_without_sbva(
            super::RetryBudget::new(
                Instant::now() + Duration::from_secs(4),
                Duration::from_millis(200),
            )
            .unwrap(),
        )
        .expect("the retry must finish through the retained checkpoint")
        .expect("the retained checkpoint makes this retry eligible");
    assert_eq!(
        retry.preprocessed.stages.sbva,
        Some(StageOutcome::Skipped(SkipReason::NotRequested)),
        "the second finish must differ only by its typed no-SBVA policy",
    );
    assert!(
        session
            .retry_without_sbva(
                super::RetryBudget::new(
                    Instant::now() + Duration::from_secs(4),
                    Duration::from_millis(200),
                )
                .unwrap(),
            )
            .is_err(),
        "the session must never repeat the same retry",
    );
}
