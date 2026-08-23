use crate::cnf::{Clause, CnfFormula};
use crate::preprocess::unit_propagation::*;
use crate::tests::common::clause;
use crate::tests::pmc_oracle::brute_force_mc;

fn clauses(cs: &[&[(u32, bool)]]) -> Vec<Clause> {
    cs.iter().copied().map(clause).collect()
}

/// The pass is EQUIVALENCE-preserving, and the forced literals it reports are
/// half of what it produced: they leave the clause set, so a caller that pins
/// them back as unit clauses holds a formula with exactly the models it started
/// with. Dropping them instead multiplies the count by two per forced variable.
#[test]
fn unit_propagation_preserves_the_model_count_of_its_input() {
    let input = CnfFormula {
        num_vars: 4,
        clauses: clauses(&[
            &[(0, true)],
            &[(0, false), (1, true)],
            &[(1, false), (2, true), (3, true)],
            &[(2, false), (3, true)],
        ]),
    };

    let (residual, forced) = propagate(&input.clauses, input.num_vars);
    assert!(
        !forced.is_empty(),
        "the fixture must give the pass something to propagate",
    );

    let mut restored = residual;
    restored.extend(forced.iter().map(|l| Clause::new(vec![*l])));
    assert_eq!(
        brute_force_mc(&CnfFormula {
            num_vars: input.num_vars,
            clauses: restored,
        }),
        brute_force_mc(&input),
        "propagating and re-pinning the forced literals changed the model count",
    );
}

#[test]
fn empty_formula() {
    let (result, forced) = propagate(&[], 3);
    assert!(result.is_empty());
    assert!(forced.is_empty());
}

#[test]
fn no_units() {
    let input = clauses(&[&[(0, true), (1, false)], &[(1, true), (2, true)]]);
    let (result, forced) = propagate(&input, 3);
    assert_eq!(result.len(), 2);
    assert!(forced.is_empty());
}

#[test]
fn single_unit() {
    let input = clauses(&[
        &[(0, true)],
        &[(0, false), (1, true)],
        &[(0, true), (2, true)],
    ]);
    let (result, forced) = propagate(&input, 3);
    // x0 forced. Clause 2 satisfied (contains x0). Clause 1 shortened to (x1).
    // (x1) is now unit -> x1 forced too.
    assert!(!forced.is_empty());
    // All forced variables' unit clauses are removed (re-added by caller).
    for c in &result {
        assert!(
            c.literals.len() > 1 || {
                let l = c.literals[0];
                !forced.iter().any(|f| f.var == l.var)
            }
        );
    }
}

#[test]
fn cascading_units() {
    let input = clauses(&[
        &[(0, true)],
        &[(0, false), (1, true)],
        &[(1, false), (2, true)],
    ]);
    let (result, forced) = propagate(&input, 3);
    assert_eq!(forced.len(), 3);
    assert!(result.is_empty());
}

#[test]
fn two_opposing_units_propagate_to_the_empty_clause() {
    let input = clauses(&[&[(0, true)], &[(0, false)]]);
    let (result, _forced) = propagate(&input, 1);
    assert!(result.iter().any(|c| c.literals.is_empty()));
}

#[test]
fn a_cascade_that_ends_in_a_conflict_propagates_to_the_empty_clause() {
    let input = clauses(&[&[(0, true)], &[(0, false), (1, true)], &[(1, false)]]);
    let (result, _forced) = propagate(&input, 2);
    assert!(result.iter().any(|c| c.literals.is_empty()));
}
