//! The deterministic construction budget: what a unit count means as a
//! deadline, and what the clock every deadline here is read against does once
//! the meter is armed.

use std::time::{Duration, Instant};

use crate::budget::{deterministic_deadline, expired, remaining};
use crate::config::ConstructionBudget;
use crate::decompose::meter;

/// A unit budget names a deadline that is that budget's worth of work past the
/// epoch — and nothing about the run's own wall enters into it.
#[test]
fn a_unit_budget_names_a_deadline_that_much_work_ahead() {
    let epoch = Instant::now();
    for ms in [1_u64, 90_000, 900_000] {
        let units = ConstructionBudget::units_for_wall_ms(ms);
        assert_eq!(
            deterministic_deadline(units, epoch),
            Some(epoch + Duration::from_millis(ms)),
        );
    }
    // A budget nothing could spend must not take the process down. Whether an
    // instant that far ahead exists at all is the platform's business; either
    // answer here is a deadline no charged work reaches.
    if let Some(absurd) = deterministic_deadline(u64::MAX, epoch) {
        assert!(absurd > epoch);
    }
}

/// The two deadline predicates every budgeted construction reads answer on the
/// meter's clock once it is armed: charged work moves them, and elapsed wall
/// does not.
#[test]
fn the_deadline_predicates_answer_on_charged_work() {
    let epoch = Instant::now();
    let units = ConstructionBudget::units_for_wall_ms(50);
    let deadline = deterministic_deadline(units, epoch).expect("a 50 ms budget is representable");

    let _armed = meter::arm(epoch);
    assert!(!expired(Some(deadline)), "nothing charged, nothing spent");
    assert_eq!(remaining(deadline), Duration::from_millis(50));

    meter::charge(units / 2);
    assert!(!expired(Some(deadline)));
    assert_eq!(remaining(deadline), Duration::from_millis(25));

    meter::charge(units);
    assert!(expired(Some(deadline)), "the budget was spent twice over");
    assert_eq!(remaining(deadline), Duration::ZERO);
}

/// An unbounded construction is unbounded whatever the meter says.
#[test]
fn an_absent_deadline_never_expires_under_the_meter() {
    let _armed = meter::arm(Instant::now());
    meter::charge(u64::MAX / 2);
    assert!(!expired(None));
}
