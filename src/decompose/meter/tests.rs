//! What the meter promises, checked without reading a clock: the construction
//! clock advances by charged work and by nothing else, and the same charges
//! move it to the same instant every time.

use super::{Armed, UNITS_PER_MS, arm, charge, metering, now, spent, wall_ms_for_units};
use std::time::{Duration, Instant};

/// Off the seam nothing is recorded and nothing is claimed: a charge is inert,
/// and the clock is the real one.
#[test]
fn an_unarmed_meter_records_nothing() {
    assert!(!metering());
    let before = spent();
    charge(1_000_000);
    assert_eq!(spent(), before, "an unarmed meter recorded a charge");
}

/// The whole point, stated as an equality: the clock is a function of the
/// charges, so the same charges put it in the same place.
#[test]
fn the_clock_advances_by_charged_work_alone() {
    let epoch = Instant::now();
    let _armed: Armed = arm(epoch);
    assert_eq!(now(), epoch, "an armed meter starts at its epoch");

    charge(UNITS_PER_MS * 40);
    assert_eq!(now(), epoch + Duration::from_millis(40));

    // A second charge continues from where the first left off rather than
    // restarting: the meter is cumulative.
    charge(UNITS_PER_MS * 2);
    assert_eq!(now(), epoch + Duration::from_millis(42));
}

/// Two runs charging the same sequence reach the same reading, whatever else
/// the machine was doing between them. This is the property a consumer buys.
#[test]
fn identical_charges_reach_an_identical_reading() {
    let charges = [3_u64, 100_000, 7, UNITS_PER_MS, 1];
    let readings: Vec<Duration> = (0..2)
        .map(|_| {
            let epoch = Instant::now();
            let _armed = arm(epoch);
            for c in charges {
                charge(c * UNITS_PER_MS);
            }
            now() - epoch
        })
        .collect();
    assert_eq!(readings[0], readings[1]);
}

/// Disarming restores the clock the enclosing code was reading, so one
/// construction cannot leave the meter running for whatever follows it.
#[test]
fn the_guard_disarms_when_it_drops() {
    {
        let _armed = arm(Instant::now());
        assert!(metering());
    }
    assert!(!metering(), "the meter stayed armed past its guard");
}

/// A budget in units and the milliseconds it converts to are the same budget.
#[test]
fn units_and_milliseconds_convert_back_and_forth() {
    for ms in [0_u64, 1, 90_000, 3_600_000] {
        assert_eq!(wall_ms_for_units(ms.saturating_mul(UNITS_PER_MS)), ms);
    }
}

/// Enough charged work to overflow the epoch addition saturates rather than
/// panicking: a budget nobody would set must still not take the process down.
#[test]
fn an_absurd_charge_saturates_instead_of_panicking() {
    let epoch = Instant::now();
    let _armed = arm(epoch);
    charge(u64::MAX);
    assert!(now() >= epoch);
}
