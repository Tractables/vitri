//! The exact rational a weighted count owes back to preprocessing.
//!
//! Every expected value here is derived in the comment beside it, from the
//! fixture's own weights — never by running the lift a second time and asserting
//! it agrees with itself.

use std::collections::HashMap;

use num_rational::BigRational;
use num_traits::One;

use crate::cnf::{Clause, CnfFormula, Literal, Original, VarId, Weights, parse_weight};
use crate::preprocess::dve::types::DveFate;
use crate::preprocess::equivalence::EquivMapping;
use crate::preprocess::renumber::Renumber;
use crate::preprocess::simplify::{
    DveReduction, EquivReduction, SimplifiedFormula, SimplifyTelemetry, Stripped, VariableStripping,
};
use crate::preprocess::weighted_lift::*;
use crate::tests::common::lit;
use crate::tests::pmc_oracle::brute_force_wmc;

fn w(text: &str) -> BigRational {
    parse_weight(text).expect("an exact rational")
}

fn bare(num_vars: u32) -> CnfFormula {
    CnfFormula {
        num_vars,
        clauses: Vec::new(),
    }
}

/// A record whose stages the caller fills in.
fn record(original: CnfFormula) -> SimplifiedFormula {
    SimplifiedFormula {
        original,
        equiv_reduced: None,
        dve_reduced: None,
        preprocessed: None,
        stripped: None,
        telemetry: SimplifyTelemetry::default(),
        decision_trace: None,
    }
}

/// The weight of literal `positive` of variable `var` in `table`, as the oracle
/// asks for it.
fn weight_of(table: &Weights<Original>, var: u32, positive: bool) -> BigRational {
    let (wn, wp) = &table[VarId(var)];
    if positive { wp.clone() } else { wn.clone() }
}

/// One variable removed by each mechanism the lift pays for: x0 is forced true,
/// x1 is constrained by nothing, x2 ≡ x3 drops x3, and x4 survives beside x2.
fn stripped_and_reduced() -> SimplifiedFormula {
    SimplifiedFormula {
        original: CnfFormula {
            num_vars: 5,
            clauses: vec![
                Clause::new(vec![lit(0, true)]),
                Clause::new(vec![lit(2, true), lit(3, false)]),
                Clause::new(vec![lit(2, false), lit(3, true)]),
                Clause::new(vec![lit(2, true), lit(4, true)]),
            ],
        },
        // Stripping took x0 and x1, leaving s0 = x2, s1 = x3, s2 = x4.
        stripped: Some(Stripped {
            formula: CnfFormula {
                num_vars: 3,
                clauses: vec![
                    Clause::new(vec![lit(0, true), lit(1, false)]),
                    Clause::new(vec![lit(0, false), lit(1, true)]),
                    Clause::new(vec![lit(0, true), lit(2, true)]),
                ],
            },
            removed: VariableStripping {
                backbone: vec![(VarId(0), true)],
                dead: vec![VarId(1)],
                renumbering: Renumber::of_kept(5, [VarId(2), VarId(3), VarId(4)]),
            },
        }),
        // s1 ≡ s0 folds away, leaving e0 = x2 and e1 = x4.
        equiv_reduced: Some(EquivReduction {
            formula: CnfFormula {
                num_vars: 2,
                clauses: vec![Clause::new(vec![lit(0, true), lit(1, true)])],
            },
            mapping: EquivMapping {
                var_to_rep: vec![lit(0, true), lit(0, true), lit(2, true)],
                rep_to_equivs: HashMap::from([(VarId(0), vec![lit(1, true)])]),
                representatives: vec![VarId(0), VarId(2)],
            },
            renumbering: Renumber::of_kept(3, [VarId(0), VarId(2)]),
        }),
        dve_reduced: None,
        preprocessed: None,
        telemetry: SimplifyTelemetry::default(),
        decision_trace: None,
    }
}

/// x0 = (3, 2), x1 = (5, 7), x2 = (2, 3), x3 = (5, 11), x4 = (2, 5), each
/// written `(w⁻, w⁺)`.
fn five_var_weights() -> Weights<Original> {
    Weights::<Original>::from_dimacs_pairs(
        &[
            (1, w("2")),
            (-1, w("3")),
            (2, w("7")),
            (-2, w("5")),
            (3, w("3")),
            (-3, w("2")),
            (4, w("11")),
            (-4, w("5")),
            (5, w("5")),
            (-5, w("2")),
        ],
        5,
    )
}

/// The contract in one line: a weighted count taken over the reduced formula,
/// under the folded weights, times the lift is the weighted count of the
/// original. Every correction the lift is made of has to be right at once for
/// this to hold, and a `2^k` in place of any of them breaks it.
#[test]
fn the_weighted_lift_reproduces_the_original_weighted_count() {
    let simplified = stripped_and_reduced();
    let orig_w = five_var_weights();
    let folded = folded_weights(&simplified, &orig_w);

    // The reduced count is taken under the FOLDED weight of the original
    // variable each reduced variable stands for.
    let reduced_wmc = brute_force_wmc(simplified.reduced_formula(), |v, positive| {
        let original = simplified.reduced_var_to_original(v as usize);
        weight_of(&folded, original as u32, positive)
    });
    let original_wmc = brute_force_wmc(&simplified.original, |v, positive| {
        weight_of(&orig_w, v, positive)
    });

    assert_eq!(
        reduced_wmc * weighted_lift(&simplified, &orig_w, &folded),
        original_wmc,
        "the lifted weighted count is not the one the original formula has",
    );
}

/// A forced variable takes one value in every model, so it costs the weight of
/// THAT literal — not the sum, and not the positive one by default.
#[test]
fn a_backbone_literal_costs_the_weight_of_its_own_polarity() {
    let mut simplified = record(bare(2));
    simplified.stripped = Some(Stripped {
        formula: bare(0),
        removed: VariableStripping {
            backbone: vec![(VarId(0), true), (VarId(1), false)],
            dead: Vec::new(),
            renumbering: Renumber::of_kept(2, []),
        },
    });
    let orig_w = Weights::<Original>::from_dimacs_pairs(
        &[(1, w("2")), (-1, w("3")), (2, w("7")), (-2, w("5"))],
        2,
    );

    // x0 is forced TRUE, so it costs its w⁺ of 2; x1 is forced FALSE, so it
    // costs its w⁻ of 5.
    assert_eq!(
        stripped_correction(&simplified, &orig_w),
        w("10"),
        "each backbone literal must be charged the weight of the value it takes",
    );
}

/// A variable nothing constrains takes both values, so it costs the SUM of its
/// two weights — the weighted reading of the integer lift's factor of two.
#[test]
fn a_dead_variable_costs_the_sum_of_its_two_weights() {
    let mut simplified = record(bare(2));
    simplified.stripped = Some(Stripped {
        formula: bare(0),
        removed: VariableStripping {
            backbone: Vec::new(),
            dead: vec![VarId(0), VarId(1)],
            renumbering: Renumber::of_kept(2, []),
        },
    });
    let orig_w = Weights::<Original>::from_dimacs_pairs(
        &[(1, w("2")), (-1, w("3")), (2, w("7")), (-2, w("5"))],
        2,
    );

    // x0 costs 3 + 2 = 5 and x1 costs 5 + 7 = 12.
    assert_eq!(
        stripped_correction(&simplified, &orig_w),
        w("60"),
        "an unconstrained variable costs both of its weights, not one",
    );
}

/// One class, `x1 ≡ x0` with the polarity `partner` gives it, over three
/// original variables.
fn with_equivalence(partner: Literal) -> SimplifiedFormula {
    let mut simplified = record(bare(3));
    simplified.equiv_reduced = Some(EquivReduction {
        formula: bare(2),
        mapping: EquivMapping {
            var_to_rep: vec![
                Literal::pos(VarId(0)),
                Literal::new(VarId(0), partner.positive),
                Literal::pos(VarId(2)),
            ],
            rep_to_equivs: HashMap::from([(VarId(0), vec![partner])]),
            representatives: vec![VarId(0), VarId(2)],
        },
        renumbering: Renumber::of_kept(3, [VarId(0), VarId(2)]),
    });
    simplified
}

/// x0 = (2, 3) and x1 = (5, 11); x2 was never declared, so it weighs 1 both ways.
fn three_var_weights() -> Weights<Original> {
    Weights::<Original>::from_dimacs_pairs(
        &[(1, w("3")), (-1, w("2")), (2, w("11")), (-2, w("5"))],
        3,
    )
}

/// An equivalence is the one elimination that is NOT a scalar: the partner's
/// weights change the weight the reduced count is taken under, so charging it
/// as a factor afterwards would be both the wrong number and a double charge.
#[test]
fn an_equivalence_partner_multiplies_into_its_representative_rather_than_the_scalar() {
    let simplified = with_equivalence(Literal::pos(VarId(1)));
    let orig_w = three_var_weights();

    let folded = folded_weights(&simplified, &orig_w);

    // The representative x0 = (2, 3) absorbs its partner x1 = (5, 11)
    // polarity for polarity: (2·5, 3·11).
    assert_eq!(
        folded.as_pairs()[0],
        (w("10"), w("33")),
        "the partner's weights must land on its representative",
    );
    assert_eq!(
        stripped_correction(&simplified, &orig_w),
        BigRational::one(),
        "a folded partner must not also be charged as a scalar",
    );
}

/// `x1 ≡ ¬x0` folds the partner's weights onto the OPPOSITE polarities. Getting
/// this backwards leaves every model count right and every weighted count wrong.
#[test]
fn an_anti_equivalent_partner_folds_with_its_polarities_swapped() {
    let simplified = with_equivalence(Literal::neg(VarId(1)));
    let orig_w = three_var_weights();

    let folded = folded_weights(&simplified, &orig_w);

    // x0 = (2, 3) absorbs x1 = (5, 11) crosswise: (2·11, 3·5).
    assert_eq!(
        folded.as_pairs()[0],
        (w("22"), w("15")),
        "an anti-equivalent partner's weights must fold crosswise",
    );
}

/// A record whose DVE stage did `fates`, over a formula of one variable per fate.
fn with_dve(fates: Vec<DveFate>) -> SimplifiedFormula {
    let survivors: Vec<VarId> = fates
        .iter()
        .enumerate()
        .filter(|(_, fate)| **fate == DveFate::Kept)
        .map(|(j, _)| VarId(j as u32))
        .collect();
    let mut simplified = record(bare(fates.len() as u32));
    simplified.dve_reduced = Some(DveReduction {
        formula: bare(survivors.len() as u32),
        renumbering: Renumber::of_kept(fates.len(), survivors),
        fates,
    });
    simplified
}

/// A defined variable's value is decided by the model, so when its two literals
/// weigh differently its contribution is not a scalar at all. There is no
/// per-variable fallback: the whole stage is refused, and the caller compiles
/// the formula DVE was handed.
#[test]
fn a_defined_variable_with_unequal_weights_makes_the_whole_stage_unsupported() {
    let simplified = with_dve(vec![DveFate::Kept, DveFate::Defined]);
    let equal = Weights::<Original>::from_dimacs_pairs(&[(2, w("7")), (-2, w("7"))], 2);
    let unequal = Weights::<Original>::from_dimacs_pairs(&[(2, w("7")), (-2, w("5"))], 2);

    assert_eq!(
        dve_eligibility(&simplified, &equal),
        DveEligibility::Supported {
            defined: 1,
            free: 0,
        },
        "an equal-weight definition costs that one weight and is payable",
    );
    assert_eq!(
        dve_eligibility(&simplified, &unequal),
        DveEligibility::Unsupported,
        "a definition whose value decides its weight has no scalar to charge",
    );
}

/// The fold needs somewhere sound to land. A chain ending at a variable that was
/// itself eliminated has no survivor to carry the weight, and the answer is to
/// refuse the stage rather than to fold onto something already paid for.
#[test]
fn an_equivalence_chain_ending_at_an_eliminated_variable_is_unsupported() {
    let landed = vec![
        DveFate::Kept,
        DveFate::Equiv {
            rep: Literal::pos(VarId(0)),
        },
    ];
    let stranded = vec![
        DveFate::Free,
        DveFate::Equiv {
            rep: Literal::pos(VarId(0)),
        },
    ];
    let uniform = Weights::<Original>::from_dimacs_pairs(&[], 2);

    assert_eq!(
        dve_equiv_survivor(&landed, 1),
        Some(Literal::pos(VarId(0))),
        "a chain ending at a surviving variable folds onto it",
    );
    assert_eq!(
        dve_equiv_survivor(&stranded, 1),
        None,
        "a chain ending at an eliminated variable has no survivor",
    );
    assert_eq!(
        dve_eligibility(&with_dve(stranded), &uniform),
        DveEligibility::Unsupported,
        "the stranded chain must take the whole stage down with it",
    );
}

/// A representative may itself have been merged, so the survivor is found by
/// following the chain to its end and the polarity is the composition of every
/// hop — an even number of negations is no negation.
#[test]
fn a_chain_of_equivalences_composes_its_polarities() {
    let fates = [
        DveFate::Kept,
        DveFate::Equiv {
            rep: Literal::neg(VarId(0)),
        },
        DveFate::Equiv {
            rep: Literal::neg(VarId(1)),
        },
        DveFate::Equiv {
            rep: Literal::pos(VarId(2)),
        },
    ];

    assert_eq!(
        dve_equiv_survivor(&fates, 1),
        Some(Literal::neg(VarId(0))),
        "one hop keeps the hop's own polarity",
    );
    assert_eq!(
        dve_equiv_survivor(&fates, 2),
        Some(Literal::pos(VarId(0))),
        "v2 ≡ ¬v1 and v1 ≡ ¬v0, so v2 ≡ v0",
    );
    assert_eq!(
        dve_equiv_survivor(&fates, 3),
        Some(Literal::pos(VarId(0))),
        "v3 ≡ v2 ≡ v0",
    );
}

/// Every elimination being payable is not enough to keep the stage: resolution
/// can leave a residual formula LARGER than the one DVE was given. Keeping it is
/// a cost decision, and the caller earns it by having frozen the unequal-weight
/// variables out of DVE first.
#[test]
fn a_residual_left_by_an_unfrozen_run_is_reverted_and_says_why() {
    let simplified = with_dve(vec![DveFate::Kept, DveFate::Kept, DveFate::Free]);
    let uniform = Weights::<Original>::from_dimacs_pairs(&[], 3);

    let verdict = dve_verdict(&simplified, &uniform, /*freeze=*/ false);
    let DveVerdict::Revert(reason) = verdict else {
        panic!("a residual formula must not be kept without the freeze: {verdict:?}");
    };
    assert!(
        reason.contains("residual"),
        "the reason must name what was left behind: {reason}",
    );
    assert_eq!(
        dve_verdict(&simplified, &uniform, /*freeze=*/ true),
        DveVerdict::Keep {
            defined: 0,
            free: 1,
            residual: 2,
        },
        "the freeze is what makes a residual worth keeping",
    );
}
