use crate::cnf::Clause;
use crate::cnf::CnfFormula;
use crate::cnf::Literal;
use crate::cnf::VarId;
use crate::preprocess::equivalence::*;
use crate::tests::common::clause;
use crate::tests::pmc_oracle::brute_force_mc;

/// Two independent classes plus two variables neither of them names, so a class
/// really is dropped and something is left for it to constrain.
fn two_equivalence_classes() -> CnfFormula {
    CnfFormula {
        num_vars: 6,
        clauses: vec![
            // x0 ≡ x1
            clause(&[(0, true), (1, false)]),
            clause(&[(0, false), (1, true)]),
            // x2 ≡ x3
            clause(&[(2, true), (3, false)]),
            clause(&[(2, false), (3, true)]),
            clause(&[(0, true), (2, true), (4, true)]),
            clause(&[(0, false), (2, false), (5, true)]),
        ],
    }
}

/// The substitution rewrites clauses onto representatives and RE-ADDS each
/// `x ↔ rep` as two binary clauses, so a partner stays a constrained variable of
/// the same formula: the model count is the one it started with, over the same
/// variable space.
#[test]
fn equivalence_substitution_preserves_the_model_count() {
    let formula = two_equivalence_classes();
    let result = extract_equivalences_with_mapping(&formula).0;

    assert!(
        result.num_equivalences >= 1,
        "the fixture must give the substitution a class to work on",
    );
    assert_eq!(
        result.formula.num_vars, formula.num_vars,
        "the substitution keeps every variable of its input",
    );
    assert_eq!(
        brute_force_mc(&result.formula),
        brute_force_mc(&formula),
        "substituting onto representatives changed the model count",
    );
}

/// The REDUCTION is the other half, and it costs the count nothing: a dropped
/// partner is DETERMINED by its representative, so each model of the reduced
/// formula extends to exactly one model of the original. There is no `2^k` owed
/// here — that factor belongs to the variables nothing constrains.
#[test]
fn a_dropped_equivalence_partner_leaves_the_model_count_unchanged() {
    let formula = two_equivalence_classes();
    let mapping = extract_equivalences_with_mapping(&formula)
        .1
        .expect("the fixture has equivalences");

    let (reduced, renumbering) = mapping.reduce_formula(&formula);

    assert_eq!(
        reduced.num_vars,
        formula.num_vars - 2,
        "one partner per class must be dropped",
    );
    assert_eq!(renumbering.num_new_vars(), reduced.num_vars);
    assert_eq!(
        brute_force_mc(&reduced),
        brute_force_mc(&formula),
        "dropping a determined partner must not change the model count",
    );
}

#[test]
fn a_formula_with_no_binary_clauses_yields_no_equivalences() {
    let formula = CnfFormula {
        num_vars: 3,
        clauses: vec![clause(&[(0, true), (1, false), (2, true)])],
    };
    let result = extract_equivalences_with_mapping(&formula).0;
    assert_eq!(result.num_equivalences, 0);
    assert!(!result.is_unsat);
    assert_eq!(result.formula.clauses.len(), 1);
}

#[test]
fn simple_equivalence() {
    // (¬x0 ∨ x1) ∧ (x0 ∨ ¬x1) means x0 ≡ x1
    let formula = CnfFormula {
        num_vars: 3,
        clauses: vec![
            clause(&[(0, false), (1, true)]),
            clause(&[(0, true), (1, false)]),
            clause(&[(0, true), (2, true)]), // should become rep ∨ x2
        ],
    };
    let result = extract_equivalences_with_mapping(&formula).0;
    assert!(result.num_equivalences >= 1);
    assert!(!result.is_unsat);
    // The equivalence clauses themselves may be simplified
    assert!(result.formula.clauses.len() <= 3);
}

#[test]
fn unsat_detection() {
    // Two contradictory equivalences over genuine 2-literal clauses:
    //   x0 ≡ x1  (¬x0 ∨ x1, x0 ∨ ¬x1)
    //   x0 ≡ ¬x1 (¬x0 ∨ ¬x1, x0 ∨ x1)
    // Composing: x1 ≡ ¬x1 → UNSAT.
    let formula = CnfFormula {
        num_vars: 2,
        clauses: vec![
            clause(&[(0, false), (1, true)]),
            clause(&[(0, true), (1, false)]),
            clause(&[(0, false), (1, false)]),
            clause(&[(0, true), (1, true)]),
        ],
    };
    let result = extract_equivalences_with_mapping(&formula).0;
    assert!(result.is_unsat);
    assert!(result.formula.clauses.iter().any(|c| c.literals.is_empty()));
}

#[test]
fn tautology_removal() {
    // x0 ≡ x1, then clause (x0 ∨ ¬x1) becomes (rep ∨ ¬rep) → tautology
    let formula = CnfFormula {
        num_vars: 3,
        clauses: vec![
            clause(&[(0, false), (1, true)]),
            clause(&[(0, true), (1, false)]),
            clause(&[(0, true), (1, false), (2, true)]), // → tautology after subst
        ],
    };
    let result = extract_equivalences_with_mapping(&formula).0;
    assert!(!result.is_unsat);
}

#[test]
fn preserves_num_vars() {
    let formula = CnfFormula {
        num_vars: 10,
        clauses: vec![
            clause(&[(0, false), (1, true)]),
            clause(&[(0, true), (1, false)]),
        ],
    };
    let result = extract_equivalences_with_mapping(&formula).0;
    assert_eq!(result.formula.num_vars, 10);
}

#[test]
fn empty_formula() {
    let formula = CnfFormula {
        num_vars: 0,
        clauses: vec![],
    };
    let result = extract_equivalences_with_mapping(&formula).0;
    assert_eq!(result.num_equivalences, 0);
    assert!(!result.is_unsat);
}

#[test]
fn the_mapping_covers_every_variable_and_records_the_inverse_of_each_merge() {
    // x0 ≡ x1 (same polarity)
    let formula = CnfFormula {
        num_vars: 3,
        clauses: vec![
            clause(&[(0, false), (1, true)]),
            clause(&[(0, true), (1, false)]),
            clause(&[(0, true), (2, true)]),
        ],
    };
    let (result, mapping) = extract_equivalences_with_mapping(&formula);
    assert!(!result.is_unsat);
    assert!(result.num_equivalences >= 1);

    let mapping = mapping.unwrap();
    assert_eq!(mapping.var_to_rep[0], Literal::pos(VarId(0))); // x0 → x0
    assert_eq!(mapping.var_to_rep[1].var, VarId(0)); // x1 → x0
    assert!(mapping.var_to_rep[1].positive); // same polarity
    assert_eq!(mapping.var_to_rep[2], Literal::pos(VarId(2))); // x2 → itself

    assert_eq!(mapping.representatives.len(), 2);
    assert!(mapping.representatives.contains(&VarId(0)));
    assert!(mapping.representatives.contains(&VarId(2)));

    let equivs = &mapping.rep_to_equivs[&VarId(0)];
    assert_eq!(equivs.len(), 1);
    assert_eq!(equivs[0], Literal::pos(VarId(1)));
}

#[test]
fn no_mapping_is_produced_when_nothing_is_equivalent() {
    let formula = CnfFormula {
        num_vars: 3,
        clauses: vec![clause(&[(0, true), (1, false), (2, true)])],
    };
    let (_, mapping) = extract_equivalences_with_mapping(&formula);
    assert!(mapping.is_none());
}

#[test]
fn reduce_formula_simple() {
    // x0 ≡ x1, plus clause (x0 ∨ x2)
    let formula = CnfFormula {
        num_vars: 3,
        clauses: vec![
            clause(&[(0, false), (1, true)]),
            clause(&[(0, true), (1, false)]),
            clause(&[(0, true), (2, true)]),
        ],
    };
    let (_, mapping) = extract_equivalences_with_mapping(&formula);
    let mapping = mapping.unwrap();

    let (reduced, renumbering) = mapping.reduce_formula(&formula);

    // representatives x0 and x2
    assert_eq!(reduced.num_vars, 2);
    assert_eq!(renumbering.num_new_vars(), 2);

    // The equivalence clauses become tautologies after substitution, so only
    // the non-equivalence clause survives
    // (¬x0 ∨ x1) → (¬r0 ∨ r0) → tautology
    // (x0 ∨ ¬x1) → (r0 ∨ ¬r0) → tautology
    // (x0 ∨ x2) → (r0 ∨ r1) → kept
    assert_eq!(reduced.clauses.len(), 1);
    assert_eq!(reduced.clauses[0].literals.len(), 2);
}

#[test]
fn reduce_formula_preserves_polarity_flip() {
    // x0 ≡ ¬x1: (x0 ∨ x1) ∧ (¬x0 ∨ ¬x1) — means x0 and x1 are opposite
    let formula = CnfFormula {
        num_vars: 3,
        clauses: vec![
            clause(&[(0, true), (1, true)]),
            clause(&[(0, false), (1, false)]),
            clause(&[(1, true), (2, true)]),
        ],
    };
    let (_, mapping) = extract_equivalences_with_mapping(&formula);
    let mapping = mapping.unwrap();

    let rep0 = mapping.var_to_rep[0];
    let rep1 = mapping.var_to_rep[1];
    assert_eq!(rep0.var, rep1.var);
    assert_ne!(rep0.positive, rep1.positive);

    let (reduced, _) = mapping.reduce_formula(&formula);
    assert_eq!(reduced.num_vars, 2);
}

#[test]
fn reduce_formula_preserves_empty_clause() {
    // Regression: reduce_formula must not drop empty clauses (UNSAT).
    // substitute_clause used to return None for empty clauses, treating them
    // as tautologies.
    let formula = CnfFormula {
        num_vars: 3,
        clauses: vec![
            clause(&[(0, false), (1, true)]), // equivalence: x0 ≡ x1
            clause(&[(0, true), (1, false)]),
            Clause::new(vec![]), // empty clause = UNSAT
        ],
    };
    let (_, mapping) = extract_equivalences_with_mapping(&formula);
    let mapping = mapping.unwrap();

    let (reduced, _) = mapping.reduce_formula(&formula);
    assert!(
        reduced.clauses.iter().any(|c| c.literals.is_empty()),
        "empty clause (UNSAT) was dropped by reduce_formula"
    );
}
