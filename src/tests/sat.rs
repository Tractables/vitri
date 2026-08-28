//! The SAT handle as a consumer reaches it: `vitri::sat`.
//!
//! Every test goes through the public re-export rather than the wrapper's own
//! module, because what is being held here is the surface, not the wrapper.

use std::time::{Duration, Instant};

use crate::sat::{Bounded, CaDiCal, SearchStats, Status, Terminator, WallClockTerminator};

/// A pigeonhole instance: `pigeons` items into `holes` slots, one slot each,
/// unsatisfiable whenever there are more pigeons than holes. It is what the
/// counter tests need — an answer only real search can reach — and every clause
/// has at least two literals, so each one is a clause the solver stores rather
/// than an assignment it makes.
///
/// Variable `(i, h)` is numbered `i * holes + h + 1`, so `pigeons * holes` is
/// the highest variable used. Returns the number of clauses added.
fn pigeonhole(solver: &mut CaDiCal, pigeons: i32, holes: i32) -> i64 {
    let var = |i: i32, h: i32| i * holes + h + 1;
    solver.reserve(pigeons * holes);
    let mut clauses = 0;
    // Every pigeon is somewhere.
    for i in 0..pigeons {
        for h in 0..holes {
            solver.add(var(i, h));
        }
        solver.add(0);
        clauses += 1;
    }
    // No hole takes two pigeons.
    for h in 0..holes {
        for i in 0..pigeons {
            for j in (i + 1)..pigeons {
                solver.add(-var(i, h));
                solver.add(-var(j, h));
                solver.add(0);
                clauses += 1;
            }
        }
    }
    clauses
}

/// Stop after `conflicts` conflicts, so the tests look at a solver that has
/// done a bounded, repeatable amount of search rather than however much this
/// machine gets through.
fn search_a_little(solver: &mut CaDiCal, conflicts: i32) {
    assert!(
        solver.limit(c"conflicts", conflicts),
        "the solver does not recognise a \"conflicts\" limit",
    );
    solver.solve();
}

#[test]
fn a_solver_answers_a_formula_and_then_answers_it_again_with_the_answer_ruled_out() {
    let mut solver = CaDiCal::new().expect("a solver");
    // (a ∨ b) ∧ (¬a ∨ b): satisfiable, and b holds in every model.
    for lit in [1, 2, 0, -1, 2, 0] {
        solver.add(lit);
    }
    assert_eq!(solver.solve(), Status::Satisfiable);
    assert!(solver.val(2) > 0, "b must hold in every model");

    // Rule b out and nothing is left.
    for lit in [-2, 0] {
        solver.add(lit);
    }
    assert_eq!(solver.solve(), Status::Unsatisfiable);
}

#[test]
fn a_solver_that_has_not_searched_holds_only_the_clauses_it_was_given() {
    let mut solver = CaDiCal::new().expect("a solver");
    let added = pigeonhole(&mut solver, 4, 3);
    assert_eq!(
        solver.irredundant(),
        added,
        "the irredundant count is not the number of clauses added",
    );
    assert_eq!(
        solver.redundant(),
        0,
        "a solver that has not searched cannot have learnt anything",
    );
}

#[test]
fn a_search_that_hits_conflicts_learns_redundant_clauses() {
    let mut solver = CaDiCal::new().expect("a solver");
    let added = pigeonhole(&mut solver, 6, 5);
    search_a_little(&mut solver, 50);
    assert!(
        solver.redundant() > 0,
        "a search that hit conflicts learnt no clause",
    );
    assert!(
        solver.irredundant() <= added,
        "searching added irredundant clauses the caller never gave",
    );
}

#[test]
fn the_search_counters_report_the_work_between_two_snapshots() {
    let mut solver = CaDiCal::new().expect("a solver");
    pigeonhole(&mut solver, 6, 5);
    let before = solver.search_stats();
    search_a_little(&mut solver, 50);
    let did = solver.search_stats().since(before);
    assert!(did.conflicts > 0, "a bounded search reported no conflicts");
    assert!(did.decisions > 0, "a search reported no decisions");
    assert!(
        did.propagations > 0,
        "a search with conflicts reported no propagations",
    );
}

/// The counters are cumulative, so their difference is only meaningful one way
/// round. Taken the other way it reports no work rather than a vast one.
#[test]
fn differencing_two_snapshots_the_wrong_way_round_reports_no_work() {
    let mut solver = CaDiCal::new().expect("a solver");
    pigeonhole(&mut solver, 6, 5);
    let before = solver.search_stats();
    search_a_little(&mut solver, 50);
    let after = solver.search_stats();
    assert_eq!(
        before.since(after),
        SearchStats::default(),
        "an earlier snapshot reported work done since a later one",
    );
}

/// The slot order is a contract with the C accessor, spelled once on each side.
/// This pins the Rust half: each slot reaches its own field, so a reordering
/// shows up as a swapped value instead of as a plausible-looking number.
#[test]
fn every_counter_slot_reaches_its_own_field() {
    assert_eq!(
        SearchStats::SLOTS,
        6,
        "the accessor is asked for a different number of slots than are read",
    );
    let stats = SearchStats::from_slots([1, 2, 3, 4, 5, 6]);
    assert_eq!(stats.conflicts, 1);
    assert_eq!(stats.decisions, 2);
    assert_eq!(stats.propagations, 3);
    assert_eq!(stats.restarts, 4);
    assert_eq!(stats.learned_clauses, 5);
    assert_eq!(stats.searched, 6);
}

/// Activity is something the search accumulates, so a variable that appears in
/// no clause can never gain any. Both halves of the documented behaviour: the
/// scores are flat before the search, and separated after one long enough to
/// have accumulated any.
///
/// The search has to be given room. CaDiCaL keeps its variable scores in one of
/// two schemes and alternates between them, and only one of the two is what
/// `score_of` reads — so a search stopped after a few hundred conflicts can
/// leave every score at its initial value. The bound below is past that point
/// and still takes a fraction of a second.
#[test]
fn only_the_variables_a_search_touches_gain_activity() {
    let mut solver = CaDiCal::new().expect("a solver");
    let (pigeons, holes) = (8, 7);
    pigeonhole(&mut solver, pigeons, holes);
    // One variable past the instance, in no clause at all.
    let untouched = pigeons * holes + 1;
    solver.reserve(untouched);

    assert_eq!(
        solver.score_of(1),
        solver.score_of(untouched),
        "a solver that has not searched already separates its variables",
    );

    search_a_little(&mut solver, 3000);

    let busiest = (1..=pigeons * holes)
        .map(|v| solver.score_of(v))
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        busiest > solver.score_of(untouched),
        "no variable the search branched on outscores one it never saw",
    );
}

/// Activity is a property of the variable, so the two literals over it read the
/// same score.
#[test]
fn the_two_literals_over_a_variable_score_the_same() {
    let mut solver = CaDiCal::new().expect("a solver");
    pigeonhole(&mut solver, 8, 7);
    search_a_little(&mut solver, 3000);
    for v in 1..=8 * 7 {
        assert_eq!(
            solver.score_of(v),
            solver.score_of(-v),
            "variable {v} scores differently through its two literals",
        );
    }
}

/// A budget that is already spent is expired on its first check, not on the
/// second — a search handed one is stopped before it starts.
#[test]
fn a_spent_wall_clock_budget_is_expired_immediately() {
    let mut wall = WallClockTerminator::new(Duration::ZERO);
    assert!(wall.terminated(), "a zero budget had time left in it");
}

/// A handle moves the deadline observed by both the original terminator and a
/// clone handed to a bounded solver operation.
#[test]
fn a_wall_clock_deadline_can_be_moved_through_its_handle() {
    let mut wall = WallClockTerminator::new(Duration::ZERO);
    let bounded_clone = wall.clone();
    let handle = wall.deadline_handle();
    assert!(wall.terminated(), "a zero budget had time left in it");

    handle.set(Instant::now() + Duration::from_secs(60));
    assert!(!wall.terminated(), "the extended deadline was not shared");

    handle.set(Instant::now());
    assert!(wall.terminated(), "the shortened deadline was not shared");

    let mut solver = CaDiCal::new().expect("a solver");
    pigeonhole(&mut solver, 5, 4);
    assert_eq!(
        Bounded::new(&mut solver, bounded_clone).solve(),
        Status::Unknown,
        "the bounded clone did not observe the shortened deadline",
    );
}

/// The guard's whole point: no path out of the bounded region — including an
/// unwind — leaves the solver holding a pointer to a terminator that is gone.
#[test]
fn the_guard_disconnects_its_terminator_even_when_the_scope_unwinds() {
    struct NeverStop;
    impl Terminator for NeverStop {
        fn terminated(&mut self) -> bool {
            false
        }
    }

    let mut solver = CaDiCal::new().expect("a solver");
    for lit in [1, 2, 0] {
        solver.add(lit);
    }
    // The panic is in plain Rust, never inside a solver callback: unwinding
    // across the C++ frames would abort rather than reach the guard's drop.
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _bounded = Bounded::new(&mut solver, NeverStop);
        panic!("intentional panic inside the bounded region");
    }));
    assert!(unwound.is_err(), "the panic did not reach the caller");

    // The terminator is gone. A solver still holding a pointer to it would call
    // through freed stack on this next solve.
    assert_eq!(solver.solve(), Status::Satisfiable);
}
