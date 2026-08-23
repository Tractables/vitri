//! The show-set → selection-context rule: what makes vtree selection show-aware,
//! and what a caller reading a show set off an emitted artifact has to do to get
//! there (nothing beyond handing it over).

use crate::cnf::{Reduced, ShowSet};
use crate::decompose::{SelectionCtx, SelectionObjective, TraceLevel};
use crate::tests::common::parse;

/// A show set becomes the mask the peak metric indexes by variable; NO show set
/// is plain selection rather than an empty mask; and an EMPTY show set is a
/// projection onto nothing, which is neither of the other two.
#[test]
fn for_show_masks_the_set_and_maps_absence_to_plain() {
    let set = ShowSet::<Reduced>::from_dimacs_ids(&[1, 4]).expect("valid ids");
    let projected = SelectionCtx::for_show(Some(&set), 5);
    assert_eq!(
        projected.objective.show_mask().map(|m| m.as_slice()),
        Some(&[true, false, false, true, false][..]),
    );

    let plain = SelectionCtx::for_show(None::<&ShowSet<Reduced>>, 5);
    assert!(matches!(plain.objective, SelectionObjective::ClauseBalance));

    let empty = SelectionCtx::for_show(Some(&ShowSet::<Reduced>::empty()), 3);
    assert_eq!(
        empty.objective.show_mask().map(|m| m.as_slice()),
        Some(&[false, false, false][..]),
    );
}

/// Every research knob this crate reads from the environment, so the test below
/// can state its own precondition instead of trusting the reader to remember
/// the list.
const RESEARCH_KNOBS: &[&str] = &[
    "VITRI_PORTFOLIO_SEED",
    "VITRI_PORTFOLIO_TRACE",
    "VITRI_PMC_FLOWCUTTER_CAP_MS",
    "VITRI_GOATD_REFINE_BUDGET_MS",
];

/// With none of the variables set, filling from the environment changes
/// NOTHING: the run is configured exactly as a caller who never asked for
/// environment defaults would have configured it. This is the whole
/// unset-is-the-production-setting contract, checked over the whole knob set so
/// a knob added later without a matching default fails here rather than in a
/// benchmark.
///
/// Skips itself rather than lying if a knob happens to be exported in the
/// shell that started the run.
#[test]
fn env_defaults_on_an_unset_environment_are_the_plain_defaults() {
    if RESEARCH_KNOBS.iter().any(|k| std::env::var_os(k).is_some()) {
        return;
    }
    let plain = SelectionCtx::plain();
    let filled = SelectionCtx::plain()
        .with_env_defaults()
        .expect("no knob is set, so nothing can be malformed");
    assert_eq!(filled.portfolio, plain.portfolio);
    assert_eq!(filled.goatd, plain.goatd);
}

/// A knob the caller set survives a variable that is not set: the environment
/// fills what nobody asked for, it does not reset what somebody did. An
/// embedder configuring a build in code, then asking for environment defaults
/// on top, keeps every value it chose.
///
/// Skips itself rather than lying if a knob happens to be exported in the shell
/// that started the run.
#[test]
fn env_defaults_keep_the_knobs_the_caller_set() {
    if RESEARCH_KNOBS.iter().any(|k| std::env::var_os(k).is_some()) {
        return;
    }
    let mut ctx = SelectionCtx::plain();
    ctx.portfolio.seed = 7;
    ctx.portfolio.trace = TraceLevel::All;
    ctx.portfolio.flowcutter_cap_ms = Some(250);
    ctx.goatd.refine_budget_ms = Some(1_500);

    let filled = ctx
        .with_env_defaults()
        .expect("no knob is set, so nothing can be malformed");
    assert_eq!(filled.portfolio.seed, 7);
    assert_eq!(filled.portfolio.trace, TraceLevel::All);
    assert_eq!(filled.portfolio.flowcutter_cap_ms, Some(250));
    assert_eq!(filled.goatd.refine_budget_ms, Some(1_500));
}

/// Filling from the environment touches ONLY the research knobs: the selection
/// mode and the band size are the caller's, whatever the shell says. A
/// projected run cannot be turned plain by an exported variable.
#[test]
fn env_defaults_leave_the_selection_mode_alone() {
    let mut ctx = SelectionCtx::for_show(
        Some(&ShowSet::<Reduced>::from_dimacs_ids(&[2]).expect("valid ids")),
        4,
    );
    ctx.portfolio.peak_tolerance = 0.25;
    let filled = match ctx.with_env_defaults() {
        Ok(c) => c,
        // A malformed knob in the shell is not what this test is about.
        Err(_) => return,
    };
    assert_eq!(
        filled.objective.show_mask().map(|m| m.as_slice()),
        Some(&[false, true, false, false][..]),
    );
    assert_eq!(filled.portfolio.peak_tolerance, 0.25);
}

/// The path a consumer actually takes: read the `c p show` line off a DIMACS
/// file and hand the parsed set over. Two conversions used to sit here — one
/// taking a 0-based set, one taking the written 1-based ids — and the only
/// thing keeping them honest was a test asserting they agreed. There is one
/// conversion now, so what is left to check is that the file's route and the
/// hand-built route reach the same mask, and that a file declaring nothing does
/// not become a projection onto nothing.
#[test]
fn for_show_from_a_parsed_show_line_matches_the_hand_built_mask() {
    let dimacs = "c t pmc\n\
                  p cnf 5 2\n\
                  c p show 1 4 0\n\
                  1 2 0\n\
                  -3 4 5 0\n";
    let (formula, meta) = parse(dimacs);
    let parsed = meta
        .declared_show_vars()
        .expect("the fixture declares a show set");

    let from_file = SelectionCtx::for_show(Some(parsed), formula.num_vars);
    let by_hand = SelectionCtx::for_show(
        Some(&ShowSet::<Reduced>::from_dimacs_ids(&[1, 4]).expect("valid ids")),
        5,
    );
    assert_eq!(
        from_file.objective.show_mask().map(|m| m.as_slice()),
        by_hand.objective.show_mask().map(|m| m.as_slice()),
    );
    assert_eq!(
        from_file.objective.show_mask().map(|m| m.as_slice()),
        Some(&[true, false, false, true, false][..]),
    );

    let (_, plain_meta) = parse("p cnf 5 1\n1 2 0\n");
    let plain = SelectionCtx::for_show(plain_meta.declared_show_vars(), 5);
    assert!(matches!(plain.objective, SelectionObjective::ClauseBalance));
}
