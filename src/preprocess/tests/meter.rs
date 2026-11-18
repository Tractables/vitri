use std::time::{Duration, Instant};

use super::wall_meter;
use crate::bundle::PreprocessPhase;
use crate::config::PreprocessClock;
use crate::preprocess::meter::PreprocessMeter;

#[test]
fn deterministic_marks_read_charged_work_and_configured_clamps() {
    let mut meter = PreprocessMeter::new(PreprocessClock::Deterministic {
        configured_wall_ms: Some(7_000),
    });
    let budget = meter.clamp(Duration::from_millis(20_000), None);
    assert_eq!(budget, Duration::from_millis(7_000));
    let mark = meter.begin(PreprocessPhase::Equivalence, budget);
    let per_ms = PreprocessPhase::Equivalence.units_per_ms();
    meter.charge(2 * per_ms);
    assert_eq!(meter.elapsed_ms(mark), 2);
    meter.finish_phase(mark);
    let trace = meter.into_trace().expect("deterministic mode traces");
    assert_eq!(trace.total_units, 2 * per_ms);
    assert_eq!(trace.phases[0].budget_units, 7_000 * per_ms);
    assert_eq!(trace.phases[0].spent_units, 2 * per_ms);
    assert!(!trace.phases[0].wall_capped);
}

#[test]
fn a_deterministic_phase_ends_at_three_times_its_nominal_wall_when_that_comes_before_its_work() {
    let mut meter = PreprocessMeter::new(PreprocessClock::Deterministic {
        configured_wall_ms: None,
    });
    let nominal = Duration::from_millis(1);
    let capped = meter.begin(PreprocessPhase::Dve, nominal);
    std::thread::sleep(Duration::from_millis(5));
    assert_eq!(meter.elapsed(capped), Duration::MAX, "wall first: spent");
    meter.finish_phase(capped);

    let worked = meter.begin(PreprocessPhase::Dve, nominal);
    meter.charge(PreprocessPhase::Dve.units_per_ms());
    std::thread::sleep(Duration::from_millis(5));
    assert_eq!(
        meter.elapsed(worked),
        nominal,
        "work first: the work reading"
    );
    meter.finish_phase(worked);

    let trace = meter.into_trace().expect("deterministic mode traces");
    assert!(trace.phases[0].wall_capped);
    assert!(!trace.phases[1].wall_capped);
    assert!(!trace.backstop_reached);
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
    meter.charge(2 * PreprocessPhase::Backbone.units_per_ms());
    assert_eq!(meter.elapsed_ms(mark), 2);
    let sooner = Instant::now() + Duration::from_secs(60);
    assert_eq!(
        meter.wall_stop(Some(later + Duration::from_secs(1))),
        Some(later)
    );
    assert_eq!(
        meter.wall_stop(Some(sooner)),
        Some(sooner),
        "the phase's wall cap"
    );
    assert!(
        !meter
            .into_trace()
            .expect("deterministic mode traces")
            .backstop_reached
    );
}
