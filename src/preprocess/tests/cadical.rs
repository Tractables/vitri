use crate::preprocess::cadical::*;
use crate::preprocess::cadical_ffi::Terminator;
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
