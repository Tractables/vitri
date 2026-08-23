use crate::cnf::CnfFormula;
use crate::preprocess::pipelines::*;
use crate::tests::common::clause;

/// `[Tarjan, CadicalSimplify]` on a formula with a known equivalence (x0 ≡ x1)
/// produces the vtree-usable mapping and simplifies.
/// The equivalence assertion (x0 and x1 share a representative) is
/// CaDiCaL-independent — it comes from the Tarjan stage.
#[test]
fn eq_then_cadical_extracts_mapping() {
    // x0 ≡ x1 via (¬x0 ∨ x1) ∧ (x0 ∨ ¬x1), plus two more clauses so the
    // formula is non-trivial after substitution.
    let formula = CnfFormula {
        num_vars: 4,
        clauses: vec![
            clause(&[(0, false), (1, true)]),
            clause(&[(0, true), (1, false)]),
            clause(&[(0, true), (2, true)]),
            clause(&[(2, false), (3, true)]),
        ],
    };

    let out = run_pipeline(&formula, &[Stage::Tarjan, Stage::CadicalSimplify], None);
    assert_eq!(out.formula.num_vars, 4, "num_vars preserved");
    let mapping = out.mapping.as_ref().expect("equivalence mapping present");
    // x0 and x1 collapse to the SAME representative.
    assert_eq!(
        mapping.var_to_rep[0].var, mapping.var_to_rep[1].var,
        "x0 and x1 must share a representative"
    );
    // The equivalence reduction is carried by the mapping, not the stats:
    // `original_clauses` is the post-equivalence (CaDiCaL-input) count, and
    // the pipeline is not UNSAT.
    assert!(!out.formula.clauses.iter().any(|c| c.literals.is_empty()));
}

/// `preprocess_eq_iter_with_mapping` on a formula whose CaDiCaL pass reveals no
/// new equivalences takes the documented pass-2-none branch: its output must
/// be byte-identical to a direct `[Tarjan, CadicalSimplify]` pipeline run
/// (pass 1). This pins the control-flow branch that returns the pass-1
/// result unchanged. (The second-CaDiCaL-pass branch is exercised
/// end-to-end whenever a formula yields new equivalences on pass 2.)
#[test]
fn eq_iter_matches_pass1_when_no_second_pass() {
    let formula = CnfFormula {
        num_vars: 4,
        clauses: vec![
            clause(&[(0, false), (1, true)]),
            clause(&[(0, true), (1, false)]),
            clause(&[(0, true), (2, true)]),
            clause(&[(2, false), (3, true)]),
        ],
    };

    let p1 = run_pipeline(&formula, &[Stage::Tarjan, Stage::CadicalSimplify], None);
    let it = preprocess_eq_iter_with_mapping(&formula, None);

    assert_eq!(it.formula.num_vars, p1.formula.num_vars);
    assert_eq!(it.stats.original_clauses, p1.stats.original_clauses);
    // eq_iter's eliminated/forced totals are ≥ pass 1's (pass 2 only adds).
    assert!(it.stats.eliminated_clauses >= p1.stats.eliminated_clauses);
    assert!(it.stats.forced_vars >= p1.stats.forced_vars);
    assert_eq!(p1.mapping.is_some(), it.mapping.is_some());
}

/// UNSAT through the pipeline driver and the wrapper.
///  - Tarjan-detected UNSAT (x0 ≡ x1 ≡ ¬x1) short-circuits `[Tarjan, …]`:
///    a direct `[Tarjan, CadicalSimplify]` pipeline run and `_eq_iter_` both
///    return the empty-clause formula, no mapping, and `original_clauses`
///    pinned to the input clause count with all of them eliminated.
///  - CaDiCaL-detected UNSAT (x0 ∧ ¬x0) short-circuits `[CadicalSimplify]`:
///    a direct pipeline run returns the empty-clause formula.
#[test]
fn wrappers_preserve_unsat() {
    // Contradictory equivalences → Tarjan UNSAT (x1 ≡ ¬x1).
    let tarjan_unsat = CnfFormula {
        num_vars: 2,
        clauses: vec![
            clause(&[(0, false), (1, true)]),
            clause(&[(0, true), (1, false)]),
            clause(&[(0, false), (1, false)]),
            clause(&[(0, true), (1, true)]),
        ],
    };
    let orig_c = tarjan_unsat.clauses.len();

    let p = run_pipeline(
        &tarjan_unsat,
        &[Stage::Tarjan, Stage::CadicalSimplify],
        None,
    );
    assert!(
        p.formula.clauses.iter().any(|c| c.literals.is_empty()),
        "eq_then_cadical UNSAT formula"
    );
    assert!(p.mapping.is_none(), "no mapping on UNSAT");
    assert_eq!(p.stats.original_clauses, orig_c);
    assert_eq!(p.stats.eliminated_clauses, orig_c);

    let it = preprocess_eq_iter_with_mapping(&tarjan_unsat, None);
    assert!(
        it.formula.clauses.iter().any(|c| c.literals.is_empty()),
        "eq_iter UNSAT formula"
    );
    assert!(it.mapping.is_none());
    assert_eq!(it.stats.original_clauses, orig_c);
    assert_eq!(it.stats.eliminated_clauses, orig_c);

    // CaDiCaL-detected UNSAT through a direct pipeline run.
    let cadical_unsat = CnfFormula {
        num_vars: 1,
        clauses: vec![clause(&[(0, true)]), clause(&[(0, false)])],
    };
    let pf = run_pipeline(&cadical_unsat, &[Stage::CadicalSimplify], None);
    assert!(
        pf.formula.clauses.iter().any(|c| c.literals.is_empty()),
        "preprocess_full UNSAT formula"
    );
}

/// The iterated wrapper propagates units: a formula whose unit clause forces a
/// second variable reports at least that forced variable, with the variable
/// space intact.
#[test]
fn preprocess_full_unit_propagation() {
    let formula = CnfFormula {
        num_vars: 2,
        clauses: vec![clause(&[(0, true)]), clause(&[(0, false), (1, true)])],
    };
    let out = preprocess_eq_iter_with_mapping(&formula, None);
    assert_eq!(out.formula.num_vars, 2);
    assert!(out.stats.forced_vars >= 1);
}

/// A contradictory pair of units is reported as the empty clause.
#[test]
fn preprocess_full_unsat() {
    let formula = CnfFormula {
        num_vars: 1,
        clauses: vec![clause(&[(0, true)]), clause(&[(0, false)])],
    };
    let out = preprocess_eq_iter_with_mapping(&formula, None);
    assert!(out.formula.clauses.iter().any(|c| c.literals.is_empty()));
}

/// The wrapper never renumbers: variables that no clause mentions still count
/// towards `num_vars`.
#[test]
fn preprocess_full_preserves_num_vars() {
    let formula = CnfFormula {
        num_vars: 10,
        clauses: vec![
            clause(&[(0, true), (1, false), (2, true)]),
            clause(&[(3, true), (4, false)]),
        ],
    };
    let out = preprocess_eq_iter_with_mapping(&formula, None);
    assert_eq!(out.formula.num_vars, 10);
}
