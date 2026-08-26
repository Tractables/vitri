//! The count-preserving chain's clause-blowup gate.

use super::super::count_chain::grew_clause_count;
use super::super::projection_chain::{projection_gain_discard, projection_tail};
use super::super::stage::arjun_stage;
use crate::bundle::{DiscardReason, StageOutcome, StageReport};
use crate::cnf::{CnfFormula, Reduced, ShowSet};
use crate::config::{ArjunClauseGrowth, ProjectionNoGain, ProjectionPolicy, RunConfig};
use crate::preprocess::VarMap;
use crate::preprocess::arjun::ArjunResult;
use crate::tests::common::make_formula;

/// Variable 1 with three positive and three negative occurrences — the shape
/// eliminating it by resolution turns into every pairing of the two sides.
fn raw() -> CnfFormula {
    make_formula(
        7,
        vec![
            vec![1, 2],
            vec![1, 3],
            vec![1, 4],
            vec![-1, 5],
            vec![-1, 6],
            vec![-1, 7],
        ],
    )
}

/// [`raw`] with variable 1 resolved away: six clauses become the nine pairings,
/// and what was 2..=7 is renumbered 1..=6. Fewer variables, more clauses — the
/// trade the gate exists to refuse.
fn resolved_away() -> CnfFormula {
    let clauses = [2, 3, 4]
        .into_iter()
        .flat_map(|pos| [5, 6, 7].into_iter().map(move |neg| vec![pos - 1, neg - 1]))
        .collect();
    make_formula(6, clauses)
}

/// A reduction that came back the same size: six clauses again, over one
/// variable fewer.
fn traded_evenly() -> CnfFormula {
    make_formula(
        6,
        vec![
            vec![1, 4],
            vec![1, 5],
            vec![1, 6],
            vec![2, 4],
            vec![3, 5],
            vec![2, 6],
        ],
    )
}

/// The gate reads clause counts and nothing else — not the variable count, not
/// the numbering, which is what lets it compare two formulas over different
/// variable spaces. It refuses on `>`, so an even trade is kept.
#[test]
fn a_reduction_that_grew_the_clause_count_is_discarded() {
    let raw = raw();

    assert_eq!(
        grew_clause_count(raw.clauses.len(), &resolved_away()),
        Some(DiscardReason::NotSmaller),
        "one variable saved does not pay for three more clauses",
    );
    assert_eq!(
        grew_clause_count(raw.clauses.len(), &traded_evenly()),
        None,
        "the same clause count over fewer variables is a reduction worth keeping",
    );
    assert_eq!(
        grew_clause_count(
            raw.clauses.len(),
            &make_formula(6, vec![vec![1, 2], vec![3, 4]])
        ),
        None,
        "fewer clauses is the ordinary case, and carries no reason",
    );
}

#[test]
fn an_external_clause_baseline_can_accept_a_candidate_the_arjun_input_would_reject() {
    let input = raw();
    let candidate = resolved_away();

    assert_eq!(
        grew_clause_count(
            ArjunClauseGrowth::Reject.clause_count_baseline(input.clauses.len()),
            &candidate,
        ),
        Some(DiscardReason::NotSmaller),
        "the default baseline is the formula handed to Arjun",
    );
    assert_eq!(
        grew_clause_count(
            ArjunClauseGrowth::RejectAgainst(candidate.clauses.len())
                .clause_count_baseline(input.clauses.len()),
            &candidate,
        ),
        None,
        "an embedding caller's compile formula may have a larger baseline",
    );
    assert_eq!(
        grew_clause_count(
            ArjunClauseGrowth::RejectAgainst(candidate.clauses.len() - 1)
                .clause_count_baseline(input.clauses.len()),
            &candidate,
        ),
        Some(DiscardReason::NotSmaller),
        "the external baseline remains a strict clause-count quality gate",
    );
}

fn stage_candidate(map: VarMap<Reduced, Reduced>) -> ArjunResult {
    ArjunResult {
        formula: resolved_away(),
        multiplier_exp: 0,
        backbone: Vec::new(),
        equiv: Vec::new(),
        learnt_clauses: Vec::new(),
        independent_support: ShowSet::from_zero_based([0, 2]),
        input_to_reduced_lit: map,
    }
}

fn run_candidate(
    policy: ArjunClauseGrowth,
    reason: DiscardReason,
    map: VarMap<Reduced, Reduced>,
) -> (Option<ArjunResult>, StageReport) {
    let config = RunConfig {
        arjun_clause_growth: policy,
        ..RunConfig::default()
    };
    let mut report = StageReport::default();
    let result = arjun_stage(
        &raw(),
        &config,
        &mut report,
        |_budget, _no_sbva| Ok(Some(stage_candidate(map))),
        |_candidate| Some(reason),
    )
    .expect("the synthetic stage cannot fail");
    (result, report)
}

#[test]
fn an_external_baseline_reaches_the_shared_arjun_discard_and_report_path() {
    let input = raw();
    let candidate = resolved_away();
    let map = || VarMap::from_entries((1..=6).map(Some).collect());
    let run = |policy| {
        let config = RunConfig {
            arjun_clause_growth: policy,
            ..RunConfig::default()
        };
        let mut report = StageReport::default();
        let result = arjun_stage(
            &input,
            &config,
            &mut report,
            |_budget, _no_sbva| Ok(Some(stage_candidate(map()))),
            |reduction| {
                grew_clause_count(
                    policy.clause_count_baseline(input.clauses.len()),
                    &reduction.formula,
                )
            },
        )
        .expect("the synthetic stage cannot fail");
        (result, report)
    };

    let (rejected, report) = run(ArjunClauseGrowth::Reject);
    assert!(rejected.is_none());
    assert_eq!(
        report.arjun,
        Some(StageOutcome::Discarded(DiscardReason::NotSmaller)),
    );

    let (kept, report) = run(ArjunClauseGrowth::RejectAgainst(candidate.clauses.len()));
    assert!(kept.is_some());
    assert_eq!(report.arjun, Some(StageOutcome::Ran));
}

#[test]
fn clause_growth_is_rejected_by_default_and_kept_only_when_asked() {
    let injective = || VarMap::from_entries((1..=6).map(Some).collect());
    let (rejected, report) = run_candidate(
        ArjunClauseGrowth::default(),
        DiscardReason::NotSmaller,
        injective(),
    );
    assert!(rejected.is_none());
    assert_eq!(
        report.arjun,
        Some(StageOutcome::Discarded(DiscardReason::NotSmaller)),
    );

    let (kept, report) = run_candidate(
        ArjunClauseGrowth::KeepSound,
        DiscardReason::NotSmaller,
        injective(),
    );
    assert!(kept.is_some());
    assert_eq!(report.arjun, Some(StageOutcome::Ran));
}

#[test]
fn keep_sound_bypasses_no_discard_except_clause_growth() {
    let injective = VarMap::from_entries((1..=6).map(Some).collect());
    let (kept, report) = run_candidate(
        ArjunClauseGrowth::KeepSound,
        DiscardReason::WeightedUnusable,
        injective,
    );
    assert!(kept.is_none());
    assert_eq!(
        report.arjun,
        Some(StageOutcome::Discarded(DiscardReason::WeightedUnusable)),
    );
}

#[test]
fn keep_sound_never_bypasses_the_noninjective_map_discard() {
    let noninjective = VarMap::from_entries(vec![Some(1), Some(1)]);
    let (kept, report) = run_candidate(
        ArjunClauseGrowth::KeepSound,
        DiscardReason::NotSmaller,
        noninjective,
    );
    assert!(kept.is_none());
    assert_eq!(
        report.arjun,
        Some(StageOutcome::Discarded(DiscardReason::NonInjectiveMap)),
    );
}

#[test]
fn arjun_only_skips_the_projection_tail_that_full_runs() {
    let formula = make_formula(2, vec![vec![1, 2], vec![-1, 2]]);
    let show = ShowSet::from_zero_based([1]);

    let arjun_only = projection_tail(
        formula.clone(),
        show.clone(),
        ProjectionPolicy::ArjunOnly(ProjectionNoGain::Reject),
        None,
    );
    assert_eq!(arjun_only.formula, formula);
    assert_eq!(arjun_only.show_set, show);
    assert!(arjun_only.folds.is_empty());

    let full = projection_tail(formula.clone(), show, ProjectionPolicy::Full, None);
    assert_ne!(
        full.formula, formula,
        "the full tail must eliminate the hidden resolution variable on this fixture",
    );
}

#[test]
fn only_arjun_only_keep_sound_bypasses_no_projection_gain() {
    assert_eq!(
        projection_gain_discard(false, ProjectionPolicy::Full),
        Some(DiscardReason::NoProjectionGain),
    );
    assert_eq!(
        projection_gain_discard(false, ProjectionPolicy::ArjunOnly(ProjectionNoGain::Reject),),
        Some(DiscardReason::NoProjectionGain),
    );
    assert_eq!(
        projection_gain_discard(
            false,
            ProjectionPolicy::ArjunOnly(ProjectionNoGain::KeepSound),
        ),
        None,
    );
    for policy in [
        ProjectionPolicy::Full,
        ProjectionPolicy::ArjunOnly(ProjectionNoGain::Reject),
        ProjectionPolicy::ArjunOnly(ProjectionNoGain::KeepSound),
    ] {
        assert_eq!(projection_gain_discard(true, policy), None);
    }
}

#[test]
fn projection_keep_sound_never_bypasses_the_noninjective_map_discard() {
    let config = RunConfig {
        projection_policy: ProjectionPolicy::ArjunOnly(ProjectionNoGain::KeepSound),
        ..RunConfig::default()
    };
    let noninjective = VarMap::from_entries(vec![Some(1), Some(1)]);
    let mut report = StageReport::default();
    let kept = arjun_stage(
        &raw(),
        &config,
        &mut report,
        |_budget, _no_sbva| Ok(Some(stage_candidate(noninjective))),
        |_candidate| projection_gain_discard(false, config.projection_policy),
    )
    .expect("the synthetic stage cannot fail");

    assert!(kept.is_none());
    assert_eq!(
        report.arjun,
        Some(StageOutcome::Discarded(DiscardReason::NonInjectiveMap)),
    );
}
