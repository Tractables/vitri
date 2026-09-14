use super::refine_budget_ms;
use crate::error::VitriError;

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
