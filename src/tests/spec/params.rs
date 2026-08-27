//! Parameters: the `key=value` pairs that follow the base name.
//!
//! A parameter carries its own unit in its value — milliseconds, steps — and
//! the shapes accepted for each are enumerated here, including what an absent
//! one means.

use super::*;

/// A spec that writes no budget and one that writes it have always used
/// different patience defaults; that difference is behavior, so it is pinned
/// here.
#[test]
fn parse_flowcutter_budget_shapes() {
    match parse_ok("flowcutter-primal").param {
        SpecParam::FcTimed {
            timeout_ms,
            iters,
            patience_ms,
        } => {
            assert_eq!((timeout_ms, iters, patience_ms), (200, 100_000, 100));
        }
        _ => panic!("a spec with no budget is timed mode"),
    }
    match parse_ok("flowcutter-incidence:budget=250ms").param {
        SpecParam::FcTimed {
            timeout_ms,
            iters,
            patience_ms,
        } => {
            assert_eq!((timeout_ms, iters, patience_ms), (250, 100_000, 150));
        }
        _ => panic!("'budget=<N>ms' is timed mode"),
    }
    match parse_ok("flowcutter-primal:budget=250ms,iters=50,patience=20").param {
        SpecParam::FcTimed {
            timeout_ms,
            iters,
            patience_ms,
        } => {
            assert_eq!((timeout_ms, iters, patience_ms), (250, 50, 20));
        }
        _ => panic!("a fully written timed budget is timed mode"),
    }
    match parse_ok("flowcutter-primal:budget=100000steps").param {
        SpecParam::FcSteps { steps, iters } => assert_eq!((steps, iters), (100_000, 900)),
        _ => panic!("'budget=<N>steps' is step-budgeted mode"),
    }
    match parse_ok("flowcutter-primal:budget=100000steps,iters=900").param {
        SpecParam::FcSteps { steps, iters } => assert_eq!((steps, iters), (100_000, 900)),
        _ => panic!("a step budget with an iteration count is step-budgeted mode"),
    }
}

/// The step-budgeted shape typing through unchanged is what lets a caller
/// name the portfolio's effort and get the portfolio's candidate.
#[test]
fn parse_guided_bisect_budget_shapes() {
    let base = "guided-bisect";
    match parse_ok(base).param {
        SpecParam::FcTimed {
            timeout_ms,
            iters,
            patience_ms,
        } => assert_eq!(
            (timeout_ms, iters, patience_ms),
            (200, 100_000, 100),
            "{base} with no budget is the no-budget timed mode",
        ),
        _ => panic!("a bare {base} is timed mode"),
    }
    let steps = format!("{base}:budget=150000steps,iters=15");
    match parse_ok(&steps).param {
        SpecParam::FcSteps { steps, iters } => assert_eq!((steps, iters), (150_000, 15)),
        _ => panic!("'{steps}' is step-budgeted mode"),
    }
}

/// Seeds and imbalances come back typed, with the documented defaults.
#[test]
fn parse_seed_and_imbalance_params() {
    match parse_ok("goatd-primal:seed=7").param {
        SpecParam::Goatd { seed, refine } => {
            assert_eq!(seed, 7);
            assert!(refine, "the refinement pass is what a goatd spec means");
        }
        _ => panic!("a goatd param is a seed and a refinement"),
    }
    match parse_ok("goatd-incidence").param {
        SpecParam::Goatd { seed, .. } => assert_eq!(seed, 0, "an absent seed is 0"),
        _ => panic!("a goatd param is a seed and a refinement"),
    }
    match parse_ok("hypergraph-bisect:imbalance=0.4").param {
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

/// The seed reaches the elimination's own tie-breaking, so it is live on every
/// order — not only the two that sample. Pinned because refusing it on the
/// deterministic orders would delete a tree a caller can currently ask for.
#[test]
fn a_seed_is_accepted_by_every_elimination_order() {
    for name in crate::decompose::elimination_spec_names() {
        for (view, _) in crate::decompose::VIEW_SUFFIXES {
            let spec = format!("{name}{view}:seed=7");
            match parse_ok(&spec).param {
                SpecParam::Elimination { seed, .. } => {
                    assert_eq!(seed, 7, "{spec} carries its seed")
                }
                _ => panic!("{spec} takes a seed"),
            }
        }
    }
}

/// A key written twice would leave one of the two values unused, so the spec
/// that built the vtree would not be the spec anyone wrote.
#[test]
fn a_parameter_written_twice_is_refused_rather_than_last_wins() {
    for (spec, key) in [
        ("flowcutter-primal:budget=200ms,iters=10,iters=20", "iters"),
        (
            "flowcutter-incidence:budget=200ms,patience=10,patience=20",
            "patience",
        ),
        ("force:dim=3,dim=4", "dim"),
        ("goatd-incidence:seed=1,seed=2", "seed"),
        ("hypergraph-bisect:imbalance=0.1,imbalance=0.2", "imbalance"),
    ] {
        let err = validate_vtree_spec(spec)
            .expect_err(&format!("{spec} writes a key twice"))
            .to_string();
        assert!(
            err.contains(key),
            "{spec} must be refused naming {key:?}, got: {err}",
        );
    }
    // Each written once is the shape being repeated, and it still types through.
    match parse_ok("flowcutter-primal:budget=200ms,iters=10,patience=5").param {
        SpecParam::FcTimed {
            timeout_ms,
            iters,
            patience_ms,
        } => assert_eq!((timeout_ms, iters, patience_ms), (200, 10, 5)),
        _ => panic!("one of each key is the timed mode"),
    }
}

/// A parameter has to be written `key=value`; a bare word names no key, so
/// there is nothing to read it as.
#[test]
fn a_parameter_that_is_not_key_equals_value_is_refused() {
    for (spec, offender) in [
        ("flowcutter-primal:200ms", "200ms"),
        ("goatd-incidence:7", "7"),
        ("hypergraph-bisect:0.40", "0.40"),
        ("force:cut", "cut"),
        ("flowcutter-primal:budget=200ms,best", "best"),
        ("goatd-incidence:=3", "=3"),
    ] {
        let err = validate_vtree_spec(spec)
            .expect_err(&format!("{spec} writes a parameter without its key"))
            .to_string();
        assert!(
            err.contains(offender),
            "{spec} must be refused naming {offender:?}, got: {err}",
        );
    }
}

/// The imbalance is the permitted deviation from a half-and-half split, so a
/// value outside `0.0..=0.5` — the non-finite ones included — names a bound no
/// bisection can be asked for.
#[test]
fn goatd_contract_rejects_a_bisect_imbalance_outside_the_legal_range() {
    for value in ["2.0", "-1", "1.0", "0.5001", "-0.001", "nan", "inf", "-inf"] {
        let spec = format!("hypergraph-bisect:imbalance={value}");
        assert!(
            validate_vtree_spec(&spec).is_err(),
            "{spec} is outside 0.0..=0.5 and must be refused",
        );
    }
    // Both ends of the range are inside it.
    for (value, expected) in [("0", 0.0f64), ("0.5", 0.5), ("0.4", 0.4)] {
        let spec = format!("hypergraph-bisect:imbalance={value}");
        match parse_ok(&spec).param {
            SpecParam::Imbalance(v) => assert!(
                (v - expected).abs() < 1e-12,
                "{spec} must type to {expected}, got {v}",
            ),
            _ => panic!("{spec} takes an imbalance"),
        }
    }
}

/// Each numeric parameter is read under its own key, so a malformed one has to
/// say which key it failed to read — they otherwise report the same sentence
/// about different values.
#[test]
fn a_malformed_number_names_its_own_key() {
    for (spec, offender, key) in [
        ("flowcutter-primal:budget=200ms,iters=abc", "abc", "iters"),
        (
            "flowcutter-primal:budget=200ms,patience=abc",
            "abc",
            "patience",
        ),
        ("flowcutter-incidence:budget=ms", "ms", "budget"),
        ("force:dim=abc", "abc", "dim"),
        ("force:restarts=abc", "abc", "restarts"),
    ] {
        let err = validate_vtree_spec(spec)
            .expect_err(&format!("{spec} has an unreadable number"))
            .to_string();
        assert!(
            err.contains(offender) && err.contains(key),
            "{spec} must name {offender:?} and the {key} key, got: {err}",
        );
    }
}
