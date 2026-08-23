//! The diagnostic switch. It is process-wide, so the assertions here drive it
//! through both positions relative to whatever it was found at and put it back,
//! rather than assuming a starting value.

use crate::diagnostics::{set_verbose, verbose};

/// The switch hands back what it was before, which is what lets a caller that
/// turned diagnostics on for one call put the setting back as it found it.
#[test]
fn switching_diagnostics_hands_back_the_previous_setting() {
    let found = set_verbose(true);
    assert!(
        verbose(),
        "the switch must report the value it was just set to"
    );
    assert!(
        set_verbose(false),
        "the previous value comes back, not the new one",
    );
    assert!(!verbose());
    assert!(
        !set_verbose(found),
        "and again in the other direction, so restoring is one call",
    );
    assert_eq!(verbose(), found, "the setting is back where it started");
}
