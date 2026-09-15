use super::{GoatdKnobs, GoatdPolishing, refine_budget_ms};
use crate::error::VitriError;

#[test]
fn default_polishing_has_bounded_adaptive_effort() {
    assert_eq!(
        GoatdKnobs::default().polishing_policy(),
        GoatdPolishing::adaptive(8, 128).with_wall_limit(100).unwrap()
    );
}

#[test]
fn disabling_final_polishing_disables_both_passes() {
    let knobs = GoatdKnobs {
        final_polishing: false,
        ..GoatdKnobs::default()
    };
    knobs.validate().unwrap();
    assert_eq!(knobs.polishing_policy(), GoatdPolishing::legacy(false, false));
}

#[test]
fn explicit_polishing_overrides_the_default() {
    for policy in [
        GoatdPolishing::legacy(true, true),
        GoatdPolishing::legacy(false, false),
        GoatdPolishing::adaptive(3, 27).with_wall_limit(12).unwrap(),
    ] {
        let knobs = GoatdKnobs {
            polishing: Some(policy),
            ..GoatdKnobs::default()
        };
        knobs.validate().unwrap();
        assert_eq!(knobs.polishing_policy(), policy);
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
