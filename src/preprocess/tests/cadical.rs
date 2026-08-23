use crate::cnf::Clause;
use crate::cnf::CnfFormula;
use crate::preprocess::cadical::*;
use crate::preprocess::cadical_ffi::Terminator;
use crate::tests::common::lit;
use std::time::Duration;

#[test]
fn wall_clock_terminator_fires_after_deadline() {
    let mut t = WallClockTerminator::new(Duration::from_millis(5));
    std::thread::sleep(Duration::from_millis(20));
    assert!(t.terminated());
}

#[test]
fn wall_clock_terminator_not_yet_fired() {
    let mut t = WallClockTerminator::new(Duration::from_secs(60));
    assert!(!t.terminated());
}

#[test]
fn preprocess_cadical_budgeted_with_huge_budget_matches_default() {
    let f = CnfFormula {
        num_vars: 2,
        clauses: vec![
            Clause::new(vec![lit(0, true)]),
            Clause::new(vec![lit(0, false), lit(1, true)]),
        ],
    };
    let (a, fa) = preprocess_cadical_budgeted(&f, 3, Some(Duration::from_secs(60)));
    let (b, fb) = preprocess_cadical_budgeted(&f, 3, None);
    assert_eq!(fa, fb);
    assert_eq!(a.num_vars, b.num_vars);
    assert_eq!(a.clauses.len(), b.clauses.len());
}
