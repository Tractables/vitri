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
