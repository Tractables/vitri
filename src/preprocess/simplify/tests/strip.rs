use crate::cnf::Clause;
use crate::cnf::CnfFormula;
use crate::cnf::Literal;
use crate::cnf::VarId;
use crate::preprocess::renumber::Renumber;
use crate::preprocess::simplify::*;
use crate::tests::pmc_oracle::brute_force_mc;
use num_bigint::BigUint;

/// Reconstruct the original count from a stripping: stripped-formula models
/// times 2^dead (each dead var free), times 1 per backbone var (forced).
fn reconstruct_count(stripped: &CnfFormula, red: &VariableStripping) -> BigUint {
    brute_force_mc(stripped) * BigUint::from(1u64 << red.dead.len())
}

/// Regression: an interrupted preprocess can leave a forced (backbone) var
/// inside a surviving non-unit clause. `strip_backbone_vars` used to
/// `.expect()`-panic there ("non-backbone variable missing from renumber map",
/// the simplify.rs deadline panic). It must now finish the missing
/// unit-propagation, strip the fully-propagated formula, and preserve the count.
#[test]
fn strip_recovers_via_cleanup_when_forced_var_survives() {
    // v0 forced by the unit clause [v0], yet also present in [v0, ¬v1] —
    // exactly the shape produced when backbone elimination is cut short.
    let formula = CnfFormula {
        num_vars: 2,
        clauses: vec![
            Clause::new(vec![Literal::new(VarId(0), true)]),
            Clause::new(vec![
                Literal::new(VarId(0), true),
                Literal::new(VarId(1), false),
            ]),
        ],
    };
    let stripped = strip_backbone_vars(&formula);
    assert!(
        stripped.is_some(),
        "cleanup UP should let stripping proceed, not panic or bail",
    );
    let (f, red) = stripped.unwrap();
    assert_eq!(
        reconstruct_count(&f, &red),
        brute_force_mc(&formula),
        "stripped+cleanup count must equal the original model count",
    );
}

/// A larger incomplete shape with several unpropagated units feeding a chain of
/// longer clauses: the cleanup must still land a count-exact stripping.
#[test]
fn strip_cleanup_is_count_exact_on_chained_units() {
    // v0, v1 forced (units) but still present in longer clauses; v3 genuinely free.
    let formula = CnfFormula {
        num_vars: 4,
        clauses: vec![
            Clause::new(vec![Literal::new(VarId(0), true)]),
            Clause::new(vec![Literal::new(VarId(1), true)]),
            Clause::new(vec![
                Literal::new(VarId(0), true),
                Literal::new(VarId(2), true),
            ]),
            Clause::new(vec![
                Literal::new(VarId(1), true),
                Literal::new(VarId(2), false),
                Literal::new(VarId(3), true),
            ]),
        ],
    };
    let (f, red) = strip_backbone_vars(&formula).expect("cleanup should strip");
    assert_eq!(
        reconstruct_count(&f, &red),
        brute_force_mc(&formula),
        "chained-units cleanup count must match brute force",
    );
}

/// Sanity: when forced vars are fully eliminated from the longer clauses, the
/// fast path strips directly (no cleanup) and stays count-exact.
#[test]
fn strip_proceeds_when_forced_var_fully_eliminated() {
    // v0 forced by [v0]; the only other clause [v1, v2] does not mention it.
    let formula = CnfFormula {
        num_vars: 3,
        clauses: vec![
            Clause::new(vec![Literal::new(VarId(0), true)]),
            Clause::new(vec![
                Literal::new(VarId(1), true),
                Literal::new(VarId(2), false),
            ]),
        ],
    };
    let stripped = strip_backbone_vars(&formula);
    assert!(stripped.is_some(), "should strip when the invariant holds");
    let (f, red) = stripped.unwrap();
    assert_eq!(f.num_vars, 2, "v0 stripped, v1/v2 renumbered");
    assert_eq!(red.backbone.len(), 1, "v0 recorded as backbone");
    assert_eq!(
        reconstruct_count(&f, &red),
        brute_force_mc(&formula),
        "fast-path strip count must match brute force",
    );
}

/// The cleanup pass can DERIVE unsatisfiability, and a refuted formula has no
/// stripping to record: the caller is handed nothing and settles a count of zero
/// on the formula it already holds, rather than a backbone fabricated for a
/// formula with no models.
#[test]
fn a_cleanup_that_derives_unsat_declines_to_strip() {
    // v0 is forced both ways and still sits in a longer clause, so the first
    // pass reports the shape incomplete and hands the formula to the cleanup.
    let formula = CnfFormula {
        num_vars: 2,
        clauses: vec![
            Clause::new(vec![Literal::new(VarId(0), true)]),
            Clause::new(vec![Literal::new(VarId(0), false)]),
            Clause::new(vec![
                Literal::new(VarId(0), true),
                Literal::new(VarId(1), false),
            ]),
        ],
    };
    assert_eq!(
        brute_force_mc(&formula),
        BigUint::ZERO,
        "the fixture must be the refuted case",
    );
    assert!(
        strip_backbone_vars(&formula).is_none(),
        "a formula the cleanup refuted must not come back with a stripping",
    );
}

/// The fates are TOTAL over the original variable space, and one stripping
/// reaches all three kinds: a forced variable reports the polarity every model
/// gives it, a live one reports the reduced index now standing for it, and a
/// variable no kept clause mentions reports that nothing constrains it.
#[test]
fn a_forced_original_variable_reports_its_polarity_and_a_dead_one_reports_unconstrained() {
    let original = CnfFormula {
        num_vars: 5,
        clauses: vec![
            Clause::new(vec![Literal::new(VarId(0), true)]),
            Clause::new(vec![Literal::new(VarId(1), false)]),
            Clause::new(vec![
                Literal::new(VarId(2), true),
                Literal::new(VarId(3), false),
            ]),
        ],
    };
    let record = SimplifiedFormula {
        original,
        equiv_reduced: None,
        dve_reduced: None,
        preprocessed: None,
        stripped: Some(Stripped {
            formula: CnfFormula {
                num_vars: 2,
                clauses: vec![Clause::new(vec![
                    Literal::new(VarId(0), true),
                    Literal::new(VarId(1), false),
                ])],
            },
            removed: VariableStripping {
                backbone: vec![(VarId(0), true), (VarId(1), false)],
                dead: vec![VarId(4)],
                renumbering: Renumber::of_kept(5, [VarId(2), VarId(3)]),
            },
        }),
    };

    assert_eq!(
        record.original_fates(),
        vec![
            OriginalFate::Forced(true),
            OriginalFate::Forced(false),
            OriginalFate::Variable {
                index: 0,
                same_polarity: true,
            },
            OriginalFate::Variable {
                index: 1,
                same_polarity: true,
            },
            OriginalFate::Unconstrained,
        ],
        "every original variable must report the fate its stripping gave it",
    );
}
