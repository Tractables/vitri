//! The deadline vocabulary: what an ABSENT deadline means, and how one
//! deadline is divided among the items that share it.
//!
//! Nothing here measures elapsed time. Every assertion is either over an
//! absent deadline, over one already in the past, or an ordering that holds
//! however long the calls take.

use crate::budget::{clamp, expired, pro_rata_deadline, remaining};
use std::time::{Duration, Instant};

/// The far end of every "not yet" case below: long enough that no machine
/// reaches it while the test runs, and never actually waited for.
fn far_future() -> Instant {
    Instant::now() + Duration::from_secs(3_600)
}

fn already_passed() -> Instant {
    Instant::now() - Duration::from_secs(1)
}

/// The spelling that keeps "no deadline" from reading as "out of time": an
/// unbounded run is deliberately unbounded, so the check answers `false` rather
/// than treating the absence as expiry.
#[test]
fn an_absent_deadline_never_expires() {
    assert!(!expired(None), "an unbounded run is never out of time");
    assert!(!expired(Some(far_future())));
    assert!(expired(Some(already_passed())));
}

/// ...and the same reading on the other helper: without a deadline a stage
/// keeps the budget it was given, rather than having it clamped to nothing.
#[test]
fn a_stage_keeps_its_own_budget_when_the_run_is_unbounded() {
    let budget = Duration::from_secs(30);
    assert_eq!(clamp(budget, None), budget);
    assert_eq!(
        clamp(budget, Some(already_passed())),
        Duration::ZERO,
        "a passed deadline leaves nothing to spend",
    );
    assert_eq!(remaining(already_passed()), Duration::ZERO);
}

/// A share is counted BACK from the deadline it divides, so it can never land
/// after it — for a weight above the total as readily as one below, and with a
/// total of zero, which no division may fall over.
#[test]
fn a_share_never_lands_after_the_deadline_it_divides() {
    let deadline = far_future();
    for (weight, total) in [(0, 4), (1, 4), (3, 4), (4, 4), (9, 4), (1, 0)] {
        assert!(
            pro_rata_deadline(deadline, weight, total) <= deadline,
            "weight {weight} of {total} overshot the deadline it divides",
        );
    }
}

/// The whole weight takes the whole of what is left, exactly — and so does a
/// weight larger than the total, which is the clamp that keeps an over-declared
/// weight from asking for time that does not exist.
#[test]
fn the_whole_weight_takes_the_whole_of_what_is_left() {
    let deadline = far_future();
    assert_eq!(pro_rata_deadline(deadline, 4, 4), deadline);
    assert_eq!(pro_rata_deadline(deadline, 9, 4), deadline);
    assert_eq!(
        pro_rata_deadline(deadline, 1, 0),
        deadline,
        "a total of zero divides nothing, so the one item gets it all",
    );
}

/// Pro-rata rather than even: a bigger weight gets a later deadline out of the
/// same shared one.
///
/// The comparison is safe whatever the calls cost. A share is
/// `deadline - left * (1 - weight/total)`, and `left` only shrinks between the
/// two calls, so both the larger weight and the later call push the result the
/// same way — the ordering cannot be inverted by the clock.
#[test]
fn a_bigger_weight_gets_a_later_share_of_the_same_deadline() {
    let deadline = far_future();
    let small = pro_rata_deadline(deadline, 1, 4);
    let large = pro_rata_deadline(deadline, 3, 4);
    assert!(
        small <= large,
        "a weight of 3 in 4 must outrank one of 1 in 4"
    );
    assert!(large <= deadline);
}

/// Once the shared deadline has passed, the share is zero and the work
/// receiving it starts already expired — which is the caller's signal to stop,
/// not a fresh budget.
#[test]
fn a_share_of_a_deadline_that_has_passed_starts_already_expired() {
    let passed = already_passed();
    for (weight, total) in [(1, 4), (4, 4)] {
        let share = pro_rata_deadline(passed, weight, total);
        assert!(
            expired(Some(share)),
            "weight {weight} of {total} of a passed deadline must start expired",
        );
    }
}
