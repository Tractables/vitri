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
    CnfFormula::from_parts(num_vars, Vec::new())
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

/// The weight of literal `positive` of the variable at index `var` in `table`,
/// as the oracle asks for it.
fn weight_of(table: &Weights<Original>, var: u32, positive: bool) -> BigRational {
    let (wn, wp) = &table[VarId::from_idx(var as usize)];
    if positive { wp.clone() } else { wn.clone() }
}

/// One variable removed by each mechanism the lift pays for: x1 is forced true,
/// x2 is constrained by nothing, x3 ≡ x4 drops x4, and x5 survives beside x3.
fn stripped_and_reduced() -> SimplifiedFormula {
    SimplifiedFormula {
        original: CnfFormula::from_parts(
            5,
            vec![
                Clause::new(vec![lit(1, true)]),
                Clause::new(vec![lit(3, true), lit(4, false)]),
                Clause::new(vec![lit(3, false), lit(4, true)]),
                Clause::new(vec![lit(3, true), lit(5, true)]),
            ],
        ),
        // Stripping took x1 and x2, leaving s1 = x3, s2 = x4, s3 = x5.
        stripped: Some(Stripped {
            formula: CnfFormula::from_parts(
                3,
                vec![
                    Clause::new(vec![lit(1, true), lit(2, false)]),
                    Clause::new(vec![lit(1, false), lit(2, true)]),
                    Clause::new(vec![lit(1, true), lit(3, true)]),
                ],
            ),
            removed: VariableStripping {
                backbone: vec![(VarId::from_dimacs(1), true)],
                dead: vec![VarId::from_dimacs(2)],
                renumbering: Renumber::of_kept(
                    5,
                    [
                        VarId::from_dimacs(3),
                        VarId::from_dimacs(4),
                        VarId::from_dimacs(5),
                    ],
                ),
            },
        }),
        // s2 ≡ s1 folds away, leaving e1 = x3 and e2 = x5.
        equiv_reduced: Some(EquivReduction {
            formula: CnfFormula::from_parts(2, vec![Clause::new(vec![lit(1, true), lit(2, true)])]),
            mapping: EquivMapping {
                var_to_rep: vec![lit(1, true), lit(1, true), lit(3, true)],
                rep_to_equivs: HashMap::from([(VarId::from_dimacs(1), vec![lit(2, true)])]),
                representatives: vec![VarId::from_dimacs(1), VarId::from_dimacs(3)],
            },
            renumbering: Renumber::of_kept(3, [VarId::from_dimacs(1), VarId::from_dimacs(3)]),
        }),
        dve_reduced: None,
        preprocessed: None,
        telemetry: SimplifyTelemetry::default(),
        decision_trace: None,
    }
}

/// x1 = (3, 2), x2 = (5, 7), x3 = (2, 3), x4 = (5, 11), x5 = (2, 5), each
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
            backbone: vec![
                (VarId::from_dimacs(1), true),
                (VarId::from_dimacs(2), false),
            ],
            dead: Vec::new(),
            renumbering: Renumber::of_kept(2, []),
        },
    });
    let orig_w = Weights::<Original>::from_dimacs_pairs(
        &[(1, w("2")), (-1, w("3")), (2, w("7")), (-2, w("5"))],
        2,
    );

    // x1 is forced TRUE, so it costs its w⁺ of 2; x2 is forced FALSE, so it
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
            dead: vec![VarId::from_dimacs(1), VarId::from_dimacs(2)],
            renumbering: Renumber::of_kept(2, []),
        },
    });
    let orig_w = Weights::<Original>::from_dimacs_pairs(
        &[(1, w("2")), (-1, w("3")), (2, w("7")), (-2, w("5"))],
        2,
    );

    // x1 costs 3 + 2 = 5 and x2 costs 5 + 7 = 12.
    assert_eq!(
        stripped_correction(&simplified, &orig_w),
        w("60"),
        "an unconstrained variable costs both of its weights, not one",
    );
}

/// One class, `x2 ≡ x1` with the polarity `partner` gives it, over three
/// original variables.
fn with_equivalence(partner: Literal) -> SimplifiedFormula {
    let mut simplified = record(bare(3));
    simplified.equiv_reduced = Some(EquivReduction {
        formula: bare(2),
        mapping: EquivMapping {
            var_to_rep: vec![
                Literal::pos(VarId::from_dimacs(1)),
                Literal::new(VarId::from_dimacs(1), partner.positive),
                Literal::pos(VarId::from_dimacs(3)),
            ],
            rep_to_equivs: HashMap::from([(VarId::from_dimacs(1), vec![partner])]),
            representatives: vec![VarId::from_dimacs(1), VarId::from_dimacs(3)],
        },
        renumbering: Renumber::of_kept(3, [VarId::from_dimacs(1), VarId::from_dimacs(3)]),
    });
    simplified
}

/// x1 = (2, 3) and x2 = (5, 11); x3 was never declared, so it weighs 1 both ways.
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
    let simplified = with_equivalence(Literal::pos(VarId::from_dimacs(2)));
    let orig_w = three_var_weights();

    let folded = folded_weights(&simplified, &orig_w);

    // The representative x1 = (2, 3) absorbs its partner x2 = (5, 11)
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

/// `x2 ≡ ¬x1` folds the partner's weights onto the OPPOSITE polarities. Getting
/// this backwards leaves every model count right and every weighted count wrong.
#[test]
fn an_anti_equivalent_partner_folds_with_its_polarities_swapped() {
    let simplified = with_equivalence(Literal::neg(VarId::from_dimacs(2)));
    let orig_w = three_var_weights();

    let folded = folded_weights(&simplified, &orig_w);

    // x1 = (2, 3) absorbs x2 = (5, 11) crosswise: (2·11, 3·5).
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
        .map(|(j, _)| VarId::from_idx(j))
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
            rep: Literal::pos(VarId::from_dimacs(1)),
        },
    ];
    let stranded = vec![
        DveFate::Free,
        DveFate::Equiv {
            rep: Literal::pos(VarId::from_dimacs(1)),
        },
    ];
    let uniform = Weights::<Original>::from_dimacs_pairs(&[], 2);

    assert_eq!(
        dve_equiv_survivor(&landed, 1),
        Some(Literal::pos(VarId::from_dimacs(1))),
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

/// The walk is over a chain the caller did not necessarily build, so the shape
/// that is not a chain at all ends it: a cycle, which has no end to reach. It
/// reads as no survivor, which is what takes the reduction down.
#[test]
fn a_malformed_chain_has_no_survivor_and_ends_the_walk() {
    let two_cycle = [
        DveFate::Equiv {
            rep: Literal::pos(VarId::from_dimacs(2)),
        },
        DveFate::Equiv {
            rep: Literal::pos(VarId::from_dimacs(1)),
        },
    ];
    assert_eq!(dve_equiv_survivor(&two_cycle, 0), None);

    let self_loop = [DveFate::Equiv {
        rep: Literal::pos(VarId::from_dimacs(1)),
    }];
    assert_eq!(dve_equiv_survivor(&self_loop, 0), None);
}

/// A representative may itself have been merged, so the survivor is found by
/// following the chain to its end and the polarity is the composition of every
/// hop — an even number of negations is no negation.
#[test]
fn a_chain_of_equivalences_composes_its_polarities() {
    let fates = [
        DveFate::Kept,
        DveFate::Equiv {
            rep: Literal::neg(VarId::from_dimacs(1)),
        },
        DveFate::Equiv {
            rep: Literal::neg(VarId::from_dimacs(2)),
        },
        DveFate::Equiv {
            rep: Literal::pos(VarId::from_dimacs(3)),
        },
    ];

    assert_eq!(
        dve_equiv_survivor(&fates, 1),
        Some(Literal::neg(VarId::from_dimacs(1))),
        "one hop keeps the hop's own polarity",
    );
    assert_eq!(
        dve_equiv_survivor(&fates, 2),
        Some(Literal::pos(VarId::from_dimacs(1))),
        "v3 ≡ ¬v2 and v2 ≡ ¬v1, so v3 ≡ v1",
    );
    assert_eq!(
        dve_equiv_survivor(&fates, 3),
        Some(Literal::pos(VarId::from_dimacs(1))),
        "v4 ≡ v3 ≡ v1",
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
