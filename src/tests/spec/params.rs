//! Parameters: the `:`-separated values that follow the base name.
//!
//! A parameter carries its own unit — milliseconds, iterations, steps —
//! and the shapes accepted for each are enumerated here, including the
//! combined forms and the bare number that takes the default unit.

use super::*;

/// The bare form and the parametrized timed form have always used different
/// patience defaults; that difference is behavior, so it is pinned here.
#[test]
fn parse_flowcutter_param_shapes() {
    match parse_ok("flowcutter-primal").param {
        SpecParam::FcTimed {
            timeout_ms,
            iters,
            patience_ms,
        } => {
            assert_eq!((timeout_ms, iters, patience_ms), (200, 100_000, 100));
        }
        _ => panic!("a bare flowcutter spec is timed mode"),
    }
    match parse_ok("flowcutter-incidence:250ms").param {
        SpecParam::FcTimed {
            timeout_ms,
            iters,
            patience_ms,
        } => {
            assert_eq!((timeout_ms, iters, patience_ms), (250, 100_000, 150));
        }
        _ => panic!("':<N>ms' is timed mode"),
    }
    match parse_ok("flowcutter-primal:250ms,50iters,20patience").param {
        SpecParam::FcTimed {
            timeout_ms,
            iters,
            patience_ms,
        } => {
            assert_eq!((timeout_ms, iters, patience_ms), (250, 50, 20));
        }
        _ => panic!("':<N>ms,<N>iters,<N>patience' is timed mode"),
    }
    match parse_ok("flowcutter-primal:100000steps").param {
        SpecParam::FcSteps { steps, iters } => assert_eq!((steps, iters), (100_000, 900)),
        _ => panic!("':<N>steps' is step-budgeted mode"),
    }
    match parse_ok("flowcutter-primal:100000,900steps").param {
        SpecParam::FcSteps { steps, iters } => assert_eq!((steps, iters), (100_000, 900)),
        _ => panic!("':<N>,<iters>steps' is step-budgeted mode"),
    }
}

/// The step-budgeted shape typing through unchanged is what lets a caller
/// name the portfolio's effort and get the portfolio's candidate.
#[test]
fn parse_flowcutter_combiner_param_shapes() {
    let base = "hybrid-flowcutter-incidence";
    match parse_ok(base).param {
        SpecParam::FcTimed {
            timeout_ms,
            iters,
            patience_ms,
        } => assert_eq!(
            (timeout_ms, iters, patience_ms),
            (200, 100_000, 100),
            "a bare {base} is the bare timed mode",
        ),
        _ => panic!("a bare {base} is timed mode"),
    }
    let steps = format!("{base}:150000,15steps");
    match parse_ok(&steps).param {
        SpecParam::FcSteps { steps, iters } => assert_eq!((steps, iters), (150_000, 15)),
        _ => panic!("'{steps}' is step-budgeted mode"),
    }
}

/// Seeds and imbalances come back typed, with the documented defaults.
#[test]
fn parse_seed_and_imbalance_params() {
    match parse_ok("goatd-primal:7").param {
        SpecParam::Seed(s) => assert_eq!(s, 7),
        _ => panic!("a goatd param is a seed"),
    }
    match parse_ok("goatd").param {
        SpecParam::Seed(s) => assert_eq!(s, 0, "an absent seed is 0"),
        _ => panic!("a goatd param is a seed"),
    }
    match parse_ok("hypergraph-bisect:0.4").param {
        SpecParam::Imbalance(v) => assert!((v - 0.4).abs() < 1e-12),
        _ => panic!("a bisect param is an imbalance"),
    }
    match parse_ok("hypergraph-bisect").param {
        SpecParam::Imbalance(v) => {
            assert!((v - crate::decompose::IMBALANCE_BALANCED).abs() < 1e-12)
        }
        _ => panic!("a bisect param is an imbalance"),
    }
}

/// The parse reads a step budget out of two comma-separated fields, so a third
/// one names nothing it can use. Dropping it would build the vtree of a spec
/// nobody typed.
#[test]
fn a_third_field_in_a_step_budget_is_refused_rather_than_dropped() {
    for (spec, offender) in [
        ("flowcutter-primal:100000,900,7steps", "7"),
        ("flowcutter-incidence:100000,900,7,8steps", "7"),
        ("hybrid-flowcutter-incidence:150000,15,3steps", "3"),
    ] {
        let err = validate_vtree_spec(spec)
            .expect_err(&format!("{spec} carries a field the parse cannot use"))
            .to_string();
        assert!(
            err.contains(offender),
            "{spec} must be refused naming {offender:?}, got: {err}",
        );
    }
    // The two-field shape it is a third field OF still parses.
    assert!(validate_vtree_spec("flowcutter-primal:100000,900steps").is_ok());
}

/// A timed budget names each of its two optional fields at most once. A repeat
/// would overwrite the value the caller gave first, so the spec that built the
/// vtree would not be the spec they wrote.
#[test]
fn a_timed_field_given_twice_is_refused_rather_than_last_wins() {
    for (spec, offender) in [
        ("flowcutter-primal:200ms,10iters,20iters", "20iters"),
        (
            "flowcutter-incidence:200ms,10patience,20patience",
            "20patience",
        ),
        (
            "flowcutter-primal:200ms,10iters,5patience,20iters",
            "20iters",
        ),
        ("hybrid-flowcutter-incidence:200ms,1iters,2iters", "2iters"),
    ] {
        let err = validate_vtree_spec(spec)
            .expect_err(&format!("{spec} names a timed field twice"))
            .to_string();
        assert!(
            err.contains(offender),
            "{spec} must be refused naming {offender:?}, got: {err}",
        );
    }
    // One of each is the shape being repeated, and it still types through.
    match parse_ok("flowcutter-primal:200ms,10iters,5patience").param {
        SpecParam::FcTimed {
            timeout_ms,
            iters,
            patience_ms,
        } => assert_eq!((timeout_ms, iters, patience_ms), (200, 10, 5)),
        _ => panic!("one of each field is the timed mode"),
    }
}

/// The imbalance is a fraction of the total weight either side of a split may
/// carry, so a value outside `0.0..=1.0` — the non-finite ones included — names
/// a bound no bisection can be asked for.
#[test]
fn a_bisect_imbalance_outside_the_legal_range_is_refused() {
    for value in ["2.0", "-1", "1.5", "-0.001", "nan", "inf", "-inf"] {
        let spec = format!("hypergraph-bisect:{value}");
        let err = validate_vtree_spec(&spec)
            .expect_err(&format!("{spec} is outside 0.0..=1.0"))
            .to_string();
        assert!(
            err.contains(value),
            "{spec} must be refused naming {value:?}, got: {err}",
        );
    }
    // Both ends of the range are inside it.
    for (value, expected) in [("0", 0.0f64), ("1", 1.0), ("0.4", 0.4)] {
        let spec = format!("hypergraph-bisect:{value}");
        match parse_ok(&spec).param {
            SpecParam::Imbalance(v) => assert!(
                (v - expected).abs() < 1e-12,
                "{spec} must type to {expected}, got {v}",
            ),
            _ => panic!("{spec} takes an imbalance"),
        }
    }
}

/// Each numeric field of a timed budget is read by its own suffix, so a
/// malformed one has to say which field it failed to read — the two of them
/// otherwise report the same sentence about different values.
#[test]
fn a_malformed_number_in_a_timed_suffix_names_its_own_field() {
    for (spec, offender, field) in [
        ("flowcutter-primal:200ms,abciters", "abciters", "iters"),
        (
            "flowcutter-primal:200ms,abcpatience",
            "abcpatience",
            "patience",
        ),
        ("flowcutter-incidence:ms", "<N>ms", "timeout"),
    ] {
        let err = validate_vtree_spec(spec)
            .expect_err(&format!("{spec} has an unreadable number"))
            .to_string();
        assert!(
            err.contains(offender) && err.contains(field),
            "{spec} must name {offender:?} and the {field} field, got: {err}",
        );
    }
}
