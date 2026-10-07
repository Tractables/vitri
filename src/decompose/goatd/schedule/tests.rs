use super::{GoatdKnobs, GoatdPolishing, refine_budget_ms};
use crate::error::VitriError;

#[test]
fn default_polishing_has_bounded_adaptive_effort() {
    assert_eq!(
        GoatdKnobs::default().polishing,
        GoatdPolishing::adaptive(8, 128)
            .with_work_limit(110)
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
        GoatdPolishing::adaptive(3, 27).with_work_limit(12).unwrap(),
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

/// Polishing stops on the work it has done rather than the time it has taken,
/// so a construction nobody meters polishes exactly as far as one whose meter
/// starts with the stage, and as far on every run, however fast or loaded the
/// machine is.
#[test]
fn polishing_stops_after_the_same_work_whether_or_not_the_construction_is_metered() {
    use crate::decompose::td_to_vtree::{ConversionRequest, convert_td};
    use crate::decompose::{GraphKind, Reading, meter};
    let formula = crate::tests::circuit_fixture::multiplier();
    let pace = GraphKind::Incidence.build(&formula);
    let graph = pace.as_goatd();
    let td = ::goatd::elimination::decompose(graph, ::goatd::elimination::Order::MinFill, 0, None)
        .expect("min-fill decomposes the fixture");
    let polish = |policy: GoatdPolishing, metered: bool| {
        let request = ConversionRequest::open(Reading::default(), None);
        let baseline = convert_td(&formula, &td, request);
        let _clock = metered.then(|| meter::arm(std::time::Instant::now()));
        let before = meter::units_spent();
        let polished = policy
            .refine(graph, td.clone(), baseline, &formula, request, false)
            .expect("the fixture polishes");
        (
            polished.vtree.to_vtree_text(),
            meter::units_spent() - before,
        )
    };
    let unbounded = GoatdPolishing::adaptive(8, 128);
    let bounded = unbounded.with_work_limit(1).unwrap();
    let metered = polish(bounded, true);
    assert!(
        metered.1 < polish(unbounded, true).1,
        "the bound stopped nothing, so the comparison below says nothing",
    );
    for run in 0..2 {
        assert_eq!(
            polish(bounded, false),
            metered,
            "unmetered run {run} polished differently",
        );
    }
}
