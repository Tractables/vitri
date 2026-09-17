use std::time::{Duration, Instant};

use super::{GOATD_ELIMINATION_SOFT_MS, elimination_budget};

/// The three readings of a construction deadline an elimination pass takes: no
/// deadline spends the ceiling, a deadline sooner than the ceiling caps the
/// pass at what is left, and a deadline already spent leaves the one attempt
/// rather than nothing.
#[test]
fn an_elimination_pass_spends_the_lesser_of_its_ceiling_and_the_deadline() {
    let ceiling = Duration::from_millis(GOATD_ELIMINATION_SOFT_MS);
    assert_eq!(elimination_budget(None), ceiling);

    let soon = Instant::now() + Duration::from_millis(50);
    let capped = elimination_budget(Some(soon));
    assert!(capped <= Duration::from_millis(50), "{capped:?}");

    let spent = Instant::now() - Duration::from_secs(1);
    assert_eq!(
        elimination_budget(Some(spent)),
        Duration::from_millis(crate::budget::LAST_ATTEMPT_MS),
    );
}
