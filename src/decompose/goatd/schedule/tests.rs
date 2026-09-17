use super::{GoatdKnobs, GoatdPolishing, refine_budget_ms};
use crate::error::VitriError;

#[test]
fn default_polishing_has_bounded_adaptive_effort() {
    assert_eq!(
        GoatdKnobs::default().polishing,
        GoatdPolishing::adaptive(8, 128)
            .with_wall_limit(100)
            .unwrap()
    );
}

/// Switching the refinement off is one value, and it is the value neither
/// legacy pass is enabled at.
#[test]
fn polishing_off_is_neither_legacy_pass() {
    let knobs = GoatdKnobs {
        polishing: GoatdPolishing::off(),
        ..GoatdKnobs::default()
    };
    knobs.validate().unwrap();
    assert_eq!(knobs.polishing, GoatdPolishing::legacy(false, false));
}

#[test]
fn a_named_policy_is_the_one_a_build_uses() {
    for policy in [
        GoatdPolishing::legacy(true, true),
        GoatdPolishing::off(),
        GoatdPolishing::adaptive(3, 27).with_wall_limit(12).unwrap(),
    ] {
        let knobs = GoatdKnobs {
            polishing: policy,
            ..GoatdKnobs::default()
        };
        knobs.validate().unwrap();
        assert_eq!(knobs.polishing, policy);
    }
}

#[test]
fn zero_environment_budget_clears_explicit_allocation() {
    for value in ["0", " 0 "] {
        assert_eq!(refine_budget_ms(Some(value), Some(1_500)).unwrap(), None);
    }
}

#[test]
fn unset_environment_preserves_explicit_budget() {
    for budget in [None, Some(0), Some(1_500)] {
        assert_eq!(refine_budget_ms(None, budget).unwrap(), budget);
    }
}

#[test]
fn positive_environment_budget_overrides_explicit_allocation() {
    assert_eq!(
        refine_budget_ms(Some("250"), Some(1_500)).unwrap(),
        Some(250)
    );
}

#[test]
fn invalid_environment_budget_names_the_variable() {
    for value in ["", "-1", "unknown"] {
        assert!(matches!(
            refine_budget_ms(Some(value), Some(1_500)),
            Err(VitriError::Env {
                var: "VITRI_GOATD_REFINE_BUDGET_MS",
                ..
            })
        ));
    }
}
