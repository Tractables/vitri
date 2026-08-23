use crate::budget::resolve_scaled;

#[test]
fn no_hint_returns_historical_default() {
    assert_eq!(resolve_scaled(None, 1.0 / 60.0, 2000, 2000, 60_000), 2000);
}

#[test]
fn budget_ratios_are_identity_at_120s() {
    // The ratios are chosen so a 120 000 ms budget reproduces
    // the historical absolute defaults exactly — every cell validated at
    // t120 is unaffected by budget scaling.
    let t = Some(120_000);
    assert_eq!(resolve_scaled(t, 1.0 / 60.0, 2000, 2000, 60_000), 2000);
    assert_eq!(resolve_scaled(t, 1.0 / 600.0, 200, 200, 6_000), 200);
    assert_eq!(resolve_scaled(t, 1.0 / 120.0, 1000, 1000, 30_000), 1000);
    assert_eq!(resolve_scaled(t, 0.25, 30_000, 30_000, 900_000), 30_000);
}

#[test]
fn scales_up_at_competition_hour() {
    let t = Some(3_600_000);
    assert_eq!(resolve_scaled(t, 1.0 / 60.0, 2000, 2000, 60_000), 60_000);
    assert_eq!(resolve_scaled(t, 1.0 / 600.0, 200, 200, 6_000), 6_000);
    assert_eq!(resolve_scaled(t, 1.0 / 120.0, 1000, 1000, 30_000), 30_000);
    assert_eq!(resolve_scaled(t, 0.25, 30_000, 30_000, 900_000), 900_000);
}

#[test]
fn short_budgets_clamp_to_historical_floor() {
    let t = Some(30_000);
    assert_eq!(resolve_scaled(t, 1.0 / 60.0, 2000, 2000, 60_000), 2000);
    assert_eq!(resolve_scaled(t, 1.0 / 120.0, 1000, 1000, 30_000), 1000);
}

use crate::budget::arjun_budget_ms;

#[test]
fn arjun_budget_120s_short_window_ratio() {
    // 120s ≤ 300s window gate ⇒ budget/12 = 10_000ms. Above the 5s floor, so
    // unclamped.
    assert_eq!(arjun_budget_ms(Some(120_000)), 10_000);
}

#[test]
fn arjun_budget_short_budget_clamps_up_to_the_floor() {
    // 12s ≤ 300s ⇒ budget/12 = 1_000ms, below the 5s floor ⇒ clamped up.
    assert_eq!(arjun_budget_ms(Some(12_000)), 5_000);
}

#[test]
fn arjun_budget_1h_long_window_ratio_caps_at_600s() {
    // 3600s > 300s window gate ⇒ budget/4 = 900_000ms, capped to the 600s max.
    assert_eq!(arjun_budget_ms(Some(3_600_000)), 600_000);
}

#[test]
fn arjun_budget_no_hint_is_historical_default() {
    assert_eq!(arjun_budget_ms(None), 600_000);
}

use crate::budget::vtree_effort_scale;

#[test]
fn effort_scale_is_the_baseline_without_a_hint() {
    assert_eq!(vtree_effort_scale(None), 1.0);
}

#[test]
fn effort_scale_grows_as_the_square_root_of_the_budget() {
    // The exponent is 1/2 against a 90s baseline, so the multiplier at budget t
    // is sqrt(t / 90 000).
    let two_minutes = vtree_effort_scale(Some(120_000));
    assert!((two_minutes - (120_000.0f64 / 90_000.0).sqrt()).abs() < 1e-9);
    let an_hour = vtree_effort_scale(Some(3_600_000));
    assert!((an_hour - 40.0f64.sqrt()).abs() < 1e-9);
}

#[test]
fn effort_scale_clamps_at_both_ends() {
    // Under the baseline the multiplier would shrink construction below what it
    // was calibrated at, so it floors at 1.
    assert_eq!(vtree_effort_scale(Some(30_000)), 1.0);
    assert_eq!(vtree_effort_scale(Some(0)), 1.0);
    // Ten hours would ask for sqrt(400) = 20x; the ceiling is 8.
    assert_eq!(vtree_effort_scale(Some(36_000_000)), 8.0);
    assert_eq!(vtree_effort_scale(Some(u64::MAX)), 8.0);
}
