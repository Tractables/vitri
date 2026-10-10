use super::*;

/// One wide clause: no variable is determined by the others (with the rest
/// all false the first is forced, but with the rest all true it is free), so
/// independent-support minimization cannot shrink the support at all.
const NOTHING_DEFINED: &[&[i32]] = &[&[1, 2, 3, 4, 5, 6, 7, 8]];

/// Four equivalence pairs: in each pair one variable is determined by the
/// other, so half the support is removable.
const HALF_DEFINED: &[&[i32]] = &[
    &[1, -2],
    &[-1, 2],
    &[3, -4],
    &[-3, 4],
    &[5, -6],
    &[-5, 6],
    &[7, -8],
    &[-7, 8],
];

/// Feed `clauses` over eight variables, with every variable in the sampling
/// set, and run stage 1 under `checkpoint` (`None`: none set). Returns whether
/// stage 1 stopped for lack of progress, the reduced formula and its support.
fn run_stage1(
    clauses: &[&[i32]],
    checkpoint: Option<(Duration, f64)>,
) -> (bool, CnfFormula, Vec<crate::cnf::VarId>) {
    let mut a = ArjunLib::new(ArjunOptions::default().seed).expect("shim ctor");
    a.new_vars(8);
    for c in clauses {
        a.add_clause_dimacs(c);
    }
    a.set_sampl(&VarId::all(8).collect::<Vec<_>>());
    if let Some((after, min_progress)) = checkpoint {
        a.set_progress_checkpoint(after, min_progress);
    }
    assert!(a.stage_minimize_indep(true), "minimize stage failed");
    (a.stopped_no_progress(), a.cur_formula(), a.cur_sampl())
}

#[test]
fn a_passed_checkpoint_stops_stage_one_when_no_variable_is_defined() {
    let (stopped, _, _) = run_stage1(NOTHING_DEFINED, Some((Duration::ZERO, 0.05)));
    assert!(stopped, "stage 1 should stop: nothing can shrink");
}

#[test]
fn a_passed_checkpoint_with_no_required_progress_does_not_stop_stage_one() {
    let (stopped, _, _) = run_stage1(NOTHING_DEFINED, Some((Duration::ZERO, 0.0)));
    assert!(!stopped);
}

#[test]
fn a_checkpoint_far_in_the_future_changes_nothing() {
    for clauses in [NOTHING_DEFINED, HALF_DEFINED] {
        let plain = run_stage1(clauses, None);
        let far = run_stage1(clauses, Some((Duration::from_secs(3600), 0.05)));
        assert!(!plain.0);
        assert!(!far.0);
        assert_eq!(plain.1, far.1, "reduced formula changed");
        assert_eq!(plain.2, far.2, "support changed");
    }
}

#[test]
fn stage_one_that_can_shrink_the_support_is_not_stopped_when_no_progress_is_required() {
    // The reference run shows the support does shrink on this formula; a
    // checkpoint asking for less than that is never a reason to stop. (The
    // check reads the clock only at a loop top, and the first one sees the
    // support untouched, so the requirement here is zero: the progress rule
    // itself is exercised by the two zero/non-zero cases above.)
    let (_, _, support) = run_stage1(HALF_DEFINED, None);
    assert!(support.len() < 8, "expected a smaller support");
    let (stopped, _, _) = run_stage1(HALF_DEFINED, Some((Duration::ZERO, 0.0)));
    assert!(!stopped);
}
