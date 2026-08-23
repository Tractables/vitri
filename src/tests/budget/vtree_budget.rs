use crate::budget::{expired, pro_rata_deadline, vtree_budget_ms, vtree_deadline};
use std::time::{Duration, Instant};

#[test]
fn clamps_to_floor_on_short_budgets() {
    // remaining/3 below the floor ⇒ floor. At a 120 s budget the budget is the
    // floor and therefore inert (it exceeds what is left).
    assert_eq!(vtree_budget_ms(0), 90_000);
    assert_eq!(vtree_budget_ms(120_000), 90_000);
    assert_eq!(vtree_budget_ms(269_999), 90_000);
}

#[test]
fn one_third_in_the_middle_band() {
    assert_eq!(vtree_budget_ms(270_000), 90_000); // exactly at the floor
    assert_eq!(vtree_budget_ms(600_000), 200_000);
    assert_eq!(vtree_budget_ms(1_800_000), 600_000);
}

#[test]
fn clamps_to_cap_on_long_budgets() {
    // 1 h budget ⇒ 1200 s by ratio, capped to 900 s.
    assert_eq!(vtree_budget_ms(3_600_000), 900_000);
    assert_eq!(vtree_budget_ms(u64::MAX / 4), 900_000);
}

#[test]
fn deadline_none_without_a_budget() {
    assert!(vtree_deadline(None, Instant::now()).is_none());
}

#[test]
fn deadline_is_now_plus_budget_when_the_budget_is_long() {
    let now = Instant::now();
    let pd = now + Duration::from_secs(3600);
    let d = vtree_deadline(Some(pd), now).expect("budget present");
    assert_eq!(d, now + Duration::from_millis(900_000));
}

/// One construction deadline divided between the components that share it:
/// each gets its clause count's fraction of the time still to run, which is
/// what stops one big component from being starved by a set of tiny ones.
///
/// The window is long enough that the clock read inside is noise against the
/// fraction being pinned.
#[test]
fn a_share_of_a_shared_deadline_is_its_weight_fraction_of_the_time_left() {
    let shared = Instant::now() + Duration::from_secs(100);
    for (weight, total, want_secs) in [(3usize, 4usize, 75u64), (1, 4, 25), (1, 2, 50)] {
        let share = pro_rata_deadline(shared, weight, total);
        let left = share.saturating_duration_since(Instant::now());
        assert!(
            left >= Duration::from_secs(want_secs - 1) && left <= Duration::from_secs(want_secs),
            "{weight} of {total} must leave about {want_secs} s of the 100 s, left {left:?}",
        );
    }
}

/// No share outlives the deadline it divides — an item cannot be handed time
/// the whole set does not have. At the far end, a deadline already gone hands
/// out a share that starts expired, which is how a component learns to stop.
#[test]
fn no_share_outlives_the_deadline_it_divides() {
    let shared = Instant::now() + Duration::from_secs(100);
    assert_eq!(
        pro_rata_deadline(shared, 4, 4),
        shared,
        "the only item divides nothing",
    );
    assert_eq!(
        pro_rata_deadline(shared, 9, 4),
        shared,
        "a weight past the total is still capped at the whole",
    );

    let gone = Instant::now() - Duration::from_secs(1);
    assert!(
        expired(Some(pro_rata_deadline(gone, 1, 4))),
        "a share of a deadline already past starts past it too",
    );
}

#[test]
fn deadline_never_exceeds_the_program_deadline() {
    let now = Instant::now();
    // 60 s left: the floor (90 s) would overshoot the budget, so we clamp.
    let pd = now + Duration::from_secs(60);
    assert_eq!(vtree_deadline(Some(pd), now), Some(pd));
}
