use std::time::Duration;

use crate::cnf::{Clause, CnfFormula, Literal, Reduced, ShowSet, VarId};
use crate::projection::{
    HiddenDefinabilityConfig, classify_hidden_defined_by_show, eliminate_hidden,
};
use crate::tests::common::make_formula;

fn unbounded_config() -> HiddenDefinabilityConfig {
    HiddenDefinabilityConfig {
        time_budget: None,
        ..HiddenDefinabilityConfig::default()
    }
}

fn pigeonhole(offset: u32, pigeons: u32, holes: u32, guard: Option<i32>) -> Vec<Clause> {
    let variable = |pigeon: u32, hole: u32| (offset + pigeon * holes + hole + 1) as i32;
    let mut clauses = Vec::new();
    for pigeon in 0..pigeons {
        let mut literals = Vec::new();
        literals.extend(guard);
        literals.extend((0..holes).map(|hole| variable(pigeon, hole)));
        clauses.push(Clause::new(
            literals.into_iter().map(Literal::from).collect(),
        ));
    }
    for hole in 0..holes {
        for first in 0..pigeons {
            for second in (first + 1)..pigeons {
                let mut literals = Vec::new();
                literals.extend(guard);
                literals.push(-variable(first, hole));
                literals.push(-variable(second, hole));
                clauses.push(Clause::new(
                    literals.into_iter().map(Literal::from).collect(),
                ));
            }
        }
    }
    clauses
}

#[test]
fn bounded_projection_eliminates_only_hidden_variables() {
    let input = make_formula(3, vec![vec![1, 2], vec![-1, 3]]);
    let show = ShowSet::<Reduced>::from_vars([VarId(2), VarId(3)]).unwrap();
    let reduced = eliminate_hidden(&input, &show).expect("the show set is valid");

    assert!(
        reduced
            .clauses
            .iter()
            .all(|clause| clause.iter().all(|literal| literal.var != VarId(1)))
    );
    assert!(
        reduced
            .clauses
            .iter()
            .flat_map(|clause| clause.literals.iter())
            .any(|literal| literal.var == VarId(2))
    );
    assert!(
        reduced
            .clauses
            .iter()
            .flat_map(|clause| clause.literals.iter())
            .any(|literal| literal.var == VarId(3))
    );
}

#[test]
fn shown_variables_can_prove_a_hidden_variable_defined() {
    // y <-> x: the shown x determines hidden y. z is absent and therefore free.
    let input = make_formula(3, vec![vec![-1, 2], vec![1, -2]]);
    let show = ShowSet::<Reduced>::from_vars([VarId(1)]).unwrap();
    let result =
        classify_hidden_defined_by_show(&input, &show, [VarId(2), VarId(3)], unbounded_config())
            .expect("the request is valid");

    assert_eq!(result.defined, vec![VarId(2)]);
    assert_eq!(result.not_defined, vec![VarId(3)]);
    assert!(result.unknown.is_empty());
}

#[test]
fn a_counterexample_never_claims_hidden_definability() {
    // With x=true, both values of y satisfy x or y, so shown x does not
    // determine hidden y.
    let input = make_formula(2, vec![vec![1, 2]]);
    let show = ShowSet::<Reduced>::from_vars([VarId(1)]).unwrap();
    let result = classify_hidden_defined_by_show(&input, &show, [VarId(2)], unbounded_config())
        .expect("the request is valid");

    assert!(result.defined.is_empty());
    assert_eq!(result.not_defined, vec![VarId(2)]);
    assert!(result.unknown.is_empty());
}

#[test]
fn an_unsatisfiable_formula_vacuously_defines_absent_hidden_variables() {
    // Neither requested variable appears, so this exercises the preliminary
    // base solve rather than an appearing-variable probe.
    let input = make_formula(3, vec![vec![2], vec![-2]]);
    let show = ShowSet::<Reduced>::empty();
    let result =
        classify_hidden_defined_by_show(&input, &show, [VarId(3), VarId(1)], unbounded_config())
            .expect("the request is valid");

    assert_eq!(result.defined, vec![VarId(3), VarId(1)]);
    assert!(result.not_defined.is_empty());
    assert!(result.unknown.is_empty());
}

#[test]
fn an_unknown_base_solve_leaves_an_absent_hidden_variable_unknown() {
    let pigeons = 8;
    let holes = 7;
    let absent = VarId(pigeons * holes + 1);
    let input = CnfFormula {
        num_vars: absent.0,
        clauses: pigeonhole(0, pigeons, holes, None),
    };
    let show = ShowSet::<Reduced>::empty();
    let result = classify_hidden_defined_by_show(
        &input,
        &show,
        [absent],
        HiddenDefinabilityConfig {
            max_conflicts_per_var: 1,
            time_budget: None,
        },
    )
    .expect("the request is valid");

    assert!(result.defined.is_empty());
    assert!(result.not_defined.is_empty());
    assert_eq!(result.unknown, vec![absent]);
}

#[test]
fn a_conflict_cutoff_never_promotes_an_unfinished_probe() {
    let pigeons = 8;
    let holes = 7;
    let input = CnfFormula {
        num_vars: 1 + pigeons * holes,
        clauses: pigeonhole(1, pigeons, holes, Some(1)),
    };
    let show = ShowSet::<Reduced>::empty();
    let result = classify_hidden_defined_by_show(
        &input,
        &show,
        [VarId(1)],
        HiddenDefinabilityConfig {
            max_conflicts_per_var: 1,
            time_budget: None,
        },
    )
    .expect("the request is valid");

    assert!(result.defined.is_empty());
    assert!(result.not_defined.is_empty());
    assert_eq!(result.unknown, vec![VarId(1)]);
}

#[test]
fn probe_categories_follow_descending_incidence_then_descending_id() {
    let repeated = vec![vec![1, 4], vec![1, 4], vec![1, 4], vec![2, 4], vec![3, 4]];
    let input = make_formula(4, repeated);
    let show = ShowSet::<Reduced>::empty();
    let result = classify_hidden_defined_by_show(
        &input,
        &show,
        [VarId(2), VarId(1), VarId(3)],
        unbounded_config(),
    )
    .expect("the request is valid");

    assert!(result.defined.is_empty());
    assert_eq!(result.not_defined, vec![VarId(1), VarId(3), VarId(2)]);
    assert!(result.unknown.is_empty());
}

#[test]
fn a_zero_classification_budget_is_refused_by_name() {
    let input = make_formula(2, vec![vec![1, 2]]);
    let show = ShowSet::<Reduced>::from_vars([VarId(1)]).unwrap();
    let error = classify_hidden_defined_by_show(
        &input,
        &show,
        [VarId(2)],
        HiddenDefinabilityConfig {
            time_budget: Some(Duration::ZERO),
            ..HiddenDefinabilityConfig::default()
        },
    )
    .expect_err("an armed zero budget is inert");

    assert!(matches!(error, crate::VitriError::Config { .. }));
    assert!(error.to_string().contains("time_budget"));
}

#[test]
fn a_nonpositive_conflict_limit_is_refused_by_name() {
    let input = make_formula(2, vec![vec![1, 2]]);
    let show = ShowSet::<Reduced>::empty();
    let error = classify_hidden_defined_by_show(
        &input,
        &show,
        [VarId(1)],
        HiddenDefinabilityConfig {
            max_conflicts_per_var: 0,
            time_budget: None,
        },
    )
    .expect_err("a zero conflict limit is inert");

    assert!(matches!(error, crate::VitriError::Config { .. }));
    assert!(error.to_string().contains("max_conflicts_per_var"));
}

#[test]
fn an_out_of_range_show_variable_is_refused_by_name() {
    let input = make_formula(2, vec![vec![1, 2]]);
    let show = ShowSet::<Reduced>::from_vars([VarId(3)]).unwrap();
    let error = eliminate_hidden(&input, &show).expect_err("the show set exceeds the formula");

    assert!(matches!(error, crate::VitriError::Input { .. }));
    assert!(error.to_string().contains("show variable 3"));
}

#[test]
fn a_variable_cannot_be_both_shown_and_hidden() {
    let input = make_formula(2, vec![vec![1, 2]]);
    let show = ShowSet::<Reduced>::from_vars([VarId(1)]).unwrap();
    let error = classify_hidden_defined_by_show(&input, &show, [VarId(1)], unbounded_config())
        .expect_err("shown and hidden sets must be disjoint");

    assert!(matches!(error, crate::VitriError::Input { .. }));
    assert!(error.to_string().contains("variable 1"));
}

/// `hidden` is the one variable list a caller hands over raw — `show` has been
/// through a constructor that already refused `VarId(0)`. Refused by the same
/// rule, and before any table lookup: `VarId::idx` is `n - 1`, so `0` would
/// index past the end of every per-variable table in the sweep.
#[test]
fn zero_is_refused_as_a_hidden_variable() {
    let input = make_formula(2, vec![vec![1, 2]]);
    let show = ShowSet::<Reduced>::from_vars([VarId(1)]).unwrap();
    let error = classify_hidden_defined_by_show(&input, &show, [VarId(0)], unbounded_config())
        .expect_err("0 names no variable");

    assert!(matches!(error, crate::VitriError::Input { .. }));
    assert!(
        error.to_string().contains("0 is not a hidden variable"),
        "{error}"
    );
}
