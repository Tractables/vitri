//! The one place the existing Arjun stage selects its budget policy.

use super::super::stage::arjun_budget;
use crate::config::{ArjunBudget, RunConfig};
use std::time::{Duration, Instant};

#[test]
fn a_derived_arjun_budget_keeps_the_existing_ratio_floor_and_cap_policy() {
    for (budget_ms, expected) in [
        (Some(120_000), Duration::from_secs(20)),
        (Some(12_000), Duration::from_secs(5)),
        (Some(3_600_000), Duration::from_secs(600)),
        (None, Duration::from_secs(600)),
    ] {
        let config = RunConfig {
            budget_ms,
            arjun_budget: ArjunBudget::Derived,
            ..RunConfig::default()
        };
        assert_eq!(arjun_budget(&config), expected, "for {budget_ms:?}");
    }
}

#[test]
fn an_exact_arjun_budget_preserves_a_nonround_duration() {
    let exact = Duration::from_millis(10_574);
    let config = RunConfig {
        budget_ms: Some(12_000),
        arjun_budget: ArjunBudget::Exact(exact),
        ..RunConfig::default()
    };
    assert_eq!(
        arjun_budget(&config),
        exact,
        "the derived floor must not replace an exact duration",
    );
}

#[test]
fn an_exact_arjun_budget_above_the_derived_cap_is_unchanged() {
    let exact = Duration::from_millis(601_234);
    let config = RunConfig {
        budget_ms: Some(3_600_000),
        arjun_budget: ArjunBudget::Exact(exact),
        ..RunConfig::default()
    };
    assert_eq!(
        arjun_budget(&config),
        exact,
        "the 600 s derived cap must not truncate an exact duration",
    );
}

#[test]
fn an_earlier_absolute_deadline_clamps_an_exact_arjun_budget() {
    let now = Instant::now();
    let _clock = crate::decompose::meter::arm(now);
    let left = Duration::from_secs(3);
    let config = RunConfig {
        deadline: Some(now + left),
        arjun_budget: ArjunBudget::Exact(Duration::from_secs(20)),
        ..RunConfig::default()
    };
    assert_eq!(arjun_budget(&config), left);
}
