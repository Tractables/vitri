use crate::cnf::CnfFormula;
use crate::config::PreprocessClock;
use crate::preprocess::meter::PreprocessMeter;
use crate::preprocess::pipelines::*;
use crate::preprocess::tests::wall_meter;
use crate::tests::common::clause;

/// `[Tarjan, CadicalSimplify]` on a formula with a known equivalence (x1 ≡ x2)
/// produces the vtree-usable mapping and simplifies.
/// The equivalence assertion (x1 and x2 share a representative) is
/// CaDiCaL-independent — it comes from the Tarjan stage.
#[test]
fn eq_then_cadical_extracts_mapping() {
    // x1 ≡ x2 via (¬x1 ∨ x2) ∧ (x1 ∨ ¬x2), plus two more clauses so the
    // formula is non-trivial after substitution.
    let formula = CnfFormula::from_parts(
        4,
        vec![
            clause(&[(1, false), (2, true)]),
            clause(&[(1, true), (2, false)]),
            clause(&[(1, true), (3, true)]),
            clause(&[(3, false), (4, true)]),
        ],
    );

    let out = run_pipeline_with_meter(
        &formula,
        &[Stage::Tarjan, Stage::CadicalSimplify],
        None,
        &mut wall_meter(),
    );
    assert_eq!(out.formula.num_vars(), 4, "num_vars preserved");
    let mapping = out.mapping.as_ref().expect("equivalence mapping present");
    // x1 and x2 collapse to the SAME representative.
    assert_eq!(
        mapping.var_to_rep[0].var, mapping.var_to_rep[1].var,
        "x1 and x2 must share a representative"
    );
    // The equivalence reduction is carried by the mapping, not the stats:
    // `original_clauses` is the post-equivalence (CaDiCaL-input) count, and
    // the pipeline is not UNSAT.
    assert!(!out.formula.clauses().iter().any(|c| c.literals.is_empty()));
}

/// The Tarjan stage keeps each partner bound to its representative by two
/// binary clauses, so re-running Tarjan over pass 1's result finds pass 1's own
/// class `x1 ≡ x2` again. That class is not new, and
/// `preprocess_eq_iter_with_mapping_and_meter` runs no second CaDiCaL pass for
/// it: the result is pass 1's, clause for clause, and the deterministic clock
/// charges no more work than for pass 1 alone.
#[test]
fn eq_iter_runs_no_second_pass_for_a_class_pass_one_already_found() {
    let formula = CnfFormula::from_parts(
        4,
        vec![
            clause(&[(1, false), (2, true)]),
            clause(&[(1, true), (2, false)]),
            clause(&[(1, true), (3, true)]),
            clause(&[(3, false), (4, true)]),
        ],
    );
    let deterministic = || {
        PreprocessMeter::new(PreprocessClock::Deterministic {
            configured_wall_ms: None,
        })
    };

    let mut pass_one = deterministic();
    let p1 = run_pipeline_with_meter(
        &formula,
        &[Stage::Tarjan, Stage::CadicalSimplify],
        None,
        &mut pass_one,
    );
    let mut iterated = deterministic();
    let it = preprocess_eq_iter_with_mapping_and_meter(&formula, None, &mut iterated);

    assert_eq!(it.formula.clauses(), p1.formula.clauses());
    assert_eq!(
        iterated.into_trace().map(|t| t.total_units),
        pass_one.into_trace().map(|t| t.total_units),
    );
}

/// UNSAT through the pipeline driver and the wrapper.
///  - Tarjan-detected UNSAT (x1 ≡ x2 ≡ ¬x2) short-circuits `[Tarjan, …]`:
///    a direct `[Tarjan, CadicalSimplify]` pipeline run and `_eq_iter_` both
///    return the empty-clause formula, no mapping, and `original_clauses`
///    pinned to the input clause count with all of them eliminated.
///  - CaDiCaL-detected UNSAT (x1 ∧ ¬x1) short-circuits `[CadicalSimplify]`:
///    a direct pipeline run returns the empty-clause formula.
#[test]
fn wrappers_preserve_unsat() {
    // Contradictory equivalences → Tarjan UNSAT (x2 ≡ ¬x2).
    let tarjan_unsat = CnfFormula::from_parts(
        2,
        vec![
            clause(&[(1, false), (2, true)]),
            clause(&[(1, true), (2, false)]),
            clause(&[(1, false), (2, false)]),
            clause(&[(1, true), (2, true)]),
        ],
    );
    let orig_c = tarjan_unsat.clauses().len();

    let p = run_pipeline_with_meter(
        &tarjan_unsat,
        &[Stage::Tarjan, Stage::CadicalSimplify],
        None,
        &mut wall_meter(),
    );
    assert!(
        p.formula.clauses().iter().any(|c| c.literals.is_empty()),
        "eq_then_cadical UNSAT formula"
    );
    assert!(p.mapping.is_none(), "no mapping on UNSAT");
    assert_eq!(p.stats.original_clauses, orig_c);
    assert_eq!(p.stats.eliminated_clauses, orig_c);

    let it = preprocess_eq_iter_with_mapping_and_meter(&tarjan_unsat, None, &mut wall_meter());
    assert!(
        it.formula.clauses().iter().any(|c| c.literals.is_empty()),
        "eq_iter UNSAT formula"
    );
    assert!(it.mapping.is_none());
    assert_eq!(it.stats.original_clauses, orig_c);
    assert_eq!(it.stats.eliminated_clauses, orig_c);

    // CaDiCaL-detected UNSAT through a direct pipeline run.
    let cadical_unsat =
        CnfFormula::from_parts(1, vec![clause(&[(1, true)]), clause(&[(1, false)])]);
    let pf = run_pipeline_with_meter(
        &cadical_unsat,
        &[Stage::CadicalSimplify],
        None,
        &mut wall_meter(),
    );
    assert!(
        pf.formula.clauses().iter().any(|c| c.literals.is_empty()),
        "preprocess_full UNSAT formula"
    );
}

/// The iterated wrapper propagates units: a formula whose unit clause forces a
/// second variable reports at least that forced variable, with the variable
/// space intact.
#[test]
fn preprocess_full_unit_propagation() {
    let formula = CnfFormula::from_parts(
        2,
        vec![clause(&[(1, true)]), clause(&[(1, false), (2, true)])],
    );
    let out = preprocess_eq_iter_with_mapping_and_meter(&formula, None, &mut wall_meter());
    assert_eq!(out.formula.num_vars(), 2);
    assert!(out.stats.forced_vars >= 1);
}

/// A contradictory pair of units is reported as the empty clause.
#[test]
fn preprocess_full_unsat() {
    let formula = CnfFormula::from_parts(1, vec![clause(&[(1, true)]), clause(&[(1, false)])]);
    let out = preprocess_eq_iter_with_mapping_and_meter(&formula, None, &mut wall_meter());
    assert!(out.formula.clauses().iter().any(|c| c.literals.is_empty()));
}

/// The wrapper never renumbers: variables that no clause mentions still count
/// towards `num_vars`.
#[test]
fn preprocess_full_preserves_num_vars() {
    let formula = CnfFormula::from_parts(
        10,
        vec![
            clause(&[(1, true), (2, false), (3, true)]),
            clause(&[(4, true), (5, false)]),
        ],
    );
    let out = preprocess_eq_iter_with_mapping_and_meter(&formula, None, &mut wall_meter());
    assert_eq!(out.formula.num_vars(), 10);
}
