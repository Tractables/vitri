use std::time::{Duration, Instant};

use super::wall_meter;
use crate::bundle::PreprocessPhase;
use crate::config::PreprocessClock;
use crate::preprocess::meter::PreprocessMeter;

#[test]
fn deterministic_marks_read_charged_work_and_configured_clamps() {
    let mut meter = PreprocessMeter::new(PreprocessClock::Deterministic {
        configured_wall_ms: Some(7),
    });
    let budget = meter.clamp(Duration::from_millis(20), None);
    assert_eq!(budget, Duration::from_millis(7));
    let mark = meter.begin(PreprocessPhase::Equivalence, budget);
    meter.charge(3_800);
    assert_eq!(meter.elapsed_ms(mark), 2);
    meter.finish_phase(mark);
    let trace = meter.into_trace().expect("deterministic mode traces");
    assert_eq!(trace.total_units, 3_800);
    assert_eq!(trace.phases[0].budget_units, 13_300);
    assert_eq!(trace.phases[0].spent_units, 3_800);
}

#[test]
fn wall_clock_keeps_remaining_deadline_clamps_and_no_decision_trace() {
    let meter = wall_meter();
    let past = Instant::now() - Duration::from_secs(1);
    assert_eq!(
        meter.clamp(Duration::from_secs(5), Some(past)),
        Duration::ZERO,
    );
    assert!(meter.into_trace().is_none());
}

#[test]
fn a_passed_backstop_spends_every_deterministic_allowance() {
    let past = Instant::now() - Duration::from_secs(1);
    let meter = PreprocessMeter::new(PreprocessClock::Deterministic {
        configured_wall_ms: None,
    })
    .with_backstop(Some(past));
    let budget = meter.clamp(Duration::from_secs(5), None);
    assert_eq!(
        budget,
        Duration::from_secs(5),
        "work allowances ignore the wall"
    );
    let mark = meter.begin(PreprocessPhase::Backbone, budget);
    assert!(meter.elapsed(mark) >= budget);
    assert_eq!(meter.wall_stop(None), Some(past));
    assert!(
        meter
            .into_trace()
            .expect("deterministic mode traces")
            .backstop_reached
    );
}

#[test]
fn a_backstop_not_yet_reached_leaves_charged_work_the_only_reading() {
    let later = Instant::now() + Duration::from_secs(3_600);
    let mut meter = PreprocessMeter::new(PreprocessClock::Deterministic {
        configured_wall_ms: None,
    })
    .with_backstop(Some(later));
    let mark = meter.begin(PreprocessPhase::Backbone, Duration::from_secs(5));
    meter.charge(25_000);
    assert_eq!(meter.elapsed_ms(mark), 2);
    assert_eq!(meter.wall_stop(Some(Instant::now())), Some(later));
    assert!(
        !meter
            .into_trace()
            .expect("deterministic mode traces")
            .backstop_reached
    );
}
