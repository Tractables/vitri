use crate::cnf::Clause;
use crate::cnf::CnfFormula;
use crate::cnf::Literal;
use crate::cnf::VarId;
use crate::cnf::{Reduced, ShowSet};
use crate::preprocess::arjun::*;
use crate::tests::common::{grid_fixture, mixed_width_fixture};
use crate::tests::pmc_oracle::{brute_force_pmc, brute_force_pwmc};

/// End-to-end soundness invariant for [`run_arjun_projected_anytime`]:
/// `count(reduced, reduced_show) << multiplier_exp == count(orig, show)`.
/// Links the shim directly (no external binary), so it runs in the normal
/// tier.
#[test]
fn arjun_projected_anytime_soundness() {
    let formula = CnfFormula {
        num_vars: 5,
        clauses: vec![
            Clause::new(vec![
                Literal::new(VarId(0), true),
                Literal::new(VarId(1), true),
            ]),
            Clause::new(vec![
                Literal::new(VarId(1), false),
                Literal::new(VarId(2), true),
            ]),
            Clause::new(vec![
                Literal::new(VarId(2), false),
                Literal::new(VarId(3), true),
            ]),
        ],
    };
    let show = ShowSet::<Reduced>::from_zero_based([0, 1, 2, 4]);
    let expected = brute_force_pmc(&formula, show.as_zero_based());
    let r = match run_arjun_projected_anytime(
        &formula,
        &show,
        std::time::Duration::from_secs(30),
        false,
    )
    .expect("no VITRI_* knob is set in this test")
    {
        Some(r) => r,
        None => {
            eprintln!("Arjun shim unavailable — skipping anytime soundness test");
            return;
        }
    };
    let reduced = brute_force_pmc(&r.formula, r.show.as_zero_based());
    let got = reduced.clone() << r.multiplier_exp;
    assert_eq!(
        got, expected,
        "anytime soundness violated: reduced {} << {} = {} != {} (orig projected count)",
        reduced, r.multiplier_exp, got, expected
    );
}

/// Build (w_pos, w_neg) tables from a `(1-based lit, weight)` list, honoring
/// weights only on show vars. Non-listed literals default to weight 1.
fn pwmc_tables(
    n: usize,
    weights: &[(i32, num_rational::BigRational)],
    show: &[u32],
) -> (
    Vec<num_rational::BigRational>,
    Vec<num_rational::BigRational>,
) {
    use num_rational::BigRational;
    use num_traits::One;
    let show_set: std::collections::HashSet<u32> = show.iter().copied().collect();
    let one = BigRational::one();
    let mut w_pos = vec![one.clone(); n];
    let mut w_neg = vec![one.clone(); n];
    for (lit, w) in weights {
        let idx = VarId::from_dimacs(*lit).idx();
        if idx < n && show_set.contains(&(idx as u32)) {
            if *lit > 0 {
                w_pos[idx] = w.clone();
            } else {
                w_neg[idx] = w.clone();
            }
        }
    }
    (w_pos, w_neg)
}

/// End-to-end soundness for the in-process weighted-projected anytime path
/// (`run_arjun_weighted_projected_anytime`): `PWMC(reduced, reduced_show,
/// reduced_weights) * K == PWMC(orig, show, weights)`. Links the shim
/// directly (no external binary). Uses
/// asymmetric weights (incl. a free show var) so a soundness bug in the
/// defined-var fold or the K-multiplier readback would flip the count.
#[test]
fn arjun_weighted_projected_anytime_soundness() {
    use num_bigint::BigInt;
    use num_rational::BigRational;
    let r = |num: i64, den: i64| BigRational::new(BigInt::from(num), BigInt::from(den));

    // var 4 is a free show var (doubles the *unweighted* projected count;
    // here it contributes w_pos+w_neg).
    let formula = CnfFormula {
        num_vars: 5,
        clauses: vec![
            Clause::new(vec![
                Literal::new(VarId(0), true),
                Literal::new(VarId(1), true),
            ]),
            Clause::new(vec![
                Literal::new(VarId(1), false),
                Literal::new(VarId(2), true),
            ]),
            Clause::new(vec![
                Literal::new(VarId(2), false),
                Literal::new(VarId(3), true),
            ]),
        ],
    };
    let show = ShowSet::<Reduced>::from_zero_based([0, 1, 2, 4]);
    // Asymmetric weights on the show vars (1-based lits, both polarities).
    let weights: Vec<(i32, BigRational)> = vec![
        (1, r(2, 1)),
        (-1, r(1, 1)),
        (2, r(1, 1)),
        (-2, r(3, 1)),
        (3, r(1, 1)),
        (-3, r(1, 1)),
        (5, r(5, 1)),
        (-5, r(7, 1)),
    ];
    let (w_pos, w_neg) = pwmc_tables(5, &weights, show.as_zero_based());
    let expected = brute_force_pwmc(&formula, show.as_zero_based(), |v, val| {
        let i = v as usize;
        if val {
            w_pos[i].clone()
        } else {
            w_neg[i].clone()
        }
    });

    let a = match run_arjun_weighted_projected_anytime(
        &formula,
        &show,
        &weights,
        std::time::Duration::from_secs(30),
        false,
    )
    .expect("no VITRI_* knob is set in this test")
    {
        Some(a) => a,
        None => {
            eprintln!("Arjun shim unavailable — skipping weighted-projected soundness");
            return;
        }
    };
    let (rw_pos, rw_neg) = pwmc_tables(
        a.formula.num_vars as usize,
        &a.weights.to_dimacs_pairs(),
        a.show.as_zero_based(),
    );
    let reduced = brute_force_pwmc(&a.formula, a.show.as_zero_based(), |v, val| {
        let i = v as usize;
        if val {
            rw_pos[i].clone()
        } else {
            rw_neg[i].clone()
        }
    });
    let got = reduced * &a.multiplier;
    assert_eq!(
        got, expected,
        "weighted-projected anytime soundness violated: reduced PWMC × K = {} != {} (orig PWMC)",
        got, expected
    );
}

/// The spellings the knob accepts, through the pure parser rather than
/// `set_var` (undefined behavior under the parallel test harness).
#[test]
fn arjun_sbva_env_spellings() {
    assert_eq!(
        arjun_sbva_policy(None).expect("unset is not an error"),
        ArjunSbva::On,
    );
    assert_eq!(arjun_sbva_policy(Some("on")).expect("on"), ArjunSbva::On);
    assert_eq!(arjun_sbva_policy(Some("off")).expect("off"), ArjunSbva::Off);
    assert_eq!(
        arjun_sbva_policy(Some(" auto ")).expect("auto, whitespace trimmed"),
        ArjunSbva::Auto,
    );
    assert_eq!(arjun_sbva_policy(Some("ON")).expect("ON"), ArjunSbva::On);
    for bad in ["", "yes", "true", "1", "0"] {
        let err = arjun_sbva_policy(Some(bad)).expect_err("{bad} must not be accepted");
        assert!(
            err.to_string().contains("VITRI_ARJUN_SBVA"),
            "the error must name the variable, got: {err}"
        );
    }
}

/// The message a rejected value gets has to be enough to fix the variable
/// without reading the source: the token that was refused, and every spelling
/// that would have worked.
#[test]
fn a_rejected_policy_value_is_named_beside_every_form_that_would_have_worked() {
    let err = arjun_sbva_policy(Some("sometimes")).expect_err("`sometimes` names no policy");
    assert!(
        matches!(&err, crate::error::VitriError::Env { .. }),
        "a bad value for an environment variable is an environment error: {err:?}",
    );

    let message = err.to_string();
    assert!(
        message.contains("\"sometimes\""),
        "the message must quote the value it refused: {message}",
    );
    for form in ["on", "off", "auto"] {
        assert!(
            message.contains(form),
            "the message must offer `{form}`: {message}",
        );
    }
}

/// Arjun's weighted multiplier does not carry the weighted mass of a backbone it
/// dropped, so a reduction that resolved the instance outright would silently
/// lose it. That case is refused whatever the multiplier says.
#[test]
fn a_weighted_reduction_that_resolved_the_instance_outright_is_discarded() {
    assert!(
        !arjun_keep_reduction(ArjunKeep::Weighted {
            solved_outright: true,
            inert: false,
        }),
        "a reduction that answered the instance itself carries mass its multiplier does not",
    );
    assert!(
        arjun_keep_reduction(ArjunKeep::Weighted {
            solved_outright: false,
            inert: false,
        }),
        "a reduction that shrank something and left the instance standing is kept",
    );
}

/// Arjun eliminates variables by resolution, which can multiply the clause
/// count. Fewer variables is not the objective — a formula that compiles is —
/// so the unprojected gate compares clause counts, and only a growth is refused.
#[test]
fn a_reduction_with_more_clauses_than_the_raw_formula_is_discarded() {
    assert!(
        !arjun_keep_reduction(ArjunKeep::ClauseCount {
            raw_clauses: 10,
            reduced_clauses: 11,
        }),
        "one clause more than the raw formula is already a blowup",
    );
    assert!(
        arjun_keep_reduction(ArjunKeep::ClauseCount {
            raw_clauses: 10,
            reduced_clauses: 10,
        }),
        "an equal clause count over fewer variables is worth keeping",
    );
}

/// The projected gates are about the PROJECTION, not the formula: eliminating
/// hidden variables buys a projected count nothing unless the show set shrank or
/// the multiplier picked something up. The weighted gate is the same rule with
/// one addition — a large variable-block elimination is worth keeping on its own,
/// because a weighted projected count is compile-bound.
#[test]
fn a_projection_that_shrank_neither_the_show_set_nor_the_multiplier_is_discarded() {
    assert!(
        !arjun_keep_reduction(ArjunKeep::Projection {
            show_shrank: false,
            multiplier_nontrivial: false,
        }),
        "a pure variable elimination has no counting benefit",
    );
    assert!(arjun_keep_reduction(ArjunKeep::Projection {
        show_shrank: true,
        multiplier_nontrivial: false,
    }));
    assert!(arjun_keep_reduction(ArjunKeep::Projection {
        show_shrank: false,
        multiplier_nontrivial: true,
    }));

    assert!(
        !arjun_keep_reduction(ArjunKeep::WeightedProjection {
            show_shrank: false,
            multiplier_nontrivial: false,
            vars_shrank_10pct: false,
        }),
        "the weighted gate refuses the same no-op",
    );
    assert!(
        arjun_keep_reduction(ArjunKeep::WeightedProjection {
            show_shrank: false,
            multiplier_nontrivial: false,
            vars_shrank_10pct: true,
        }),
        "and keeps a large variable-block elimination the integer gate would refuse",
    );
}

/// The policy decides, not the fixture: on a formula the predicate accepts,
/// `on` still runs bounded variable addition and `off` still skips it, and only
/// `auto` consults the structure. That is what makes the `on` case a statement
/// about the policy rather than about the fixture.
#[test]
fn the_policy_decides_before_the_formula_does() {
    // A formula the predicate accepts — only the policy stops the skip.
    let grid = grid_fixture();
    assert!(
        !arjun_sbva_skip(&grid, ArjunSbva::On),
        "on must run bounded variable addition whatever the formula looks like",
    );
    assert!(
        arjun_sbva_skip(&grid, ArjunSbva::Off),
        "off must skip it whatever the formula looks like",
    );
    assert!(
        arjun_sbva_skip(&grid, ArjunSbva::Auto),
        "auto must skip it on a coloring-like formula",
    );
    assert!(
        !arjun_sbva_skip(&mixed_width_fixture(), ArjunSbva::Auto),
        "auto must run it on a formula the predicate rejects",
    );
}

/// What the weighted-projected stage hands back, as a set rather than as a list.
///
/// Two properties, and the readback loop can break either one on its own. It
/// walks the reduced variables in id order emitting weights, and folds any
/// weight-carrying variable Arjun left out of the sampling set back in — a
/// variable whose weight would otherwise be applied to nothing. So: every
/// weight-carrying variable must be shown, and the set must come back
/// ASCENDING, because the `c p show` line written from it is read positionally
/// by everything downstream and a set assembled by appending is ordered by the
/// order things were appended in.
///
/// The fixture makes both non-trivial: `1 ≡ 2 ∧ 3` gives the minimization a
/// definable variable to drop, the two ternary clauses keep the rest from
/// collapsing, and every variable carries an asymmetric weight, so whichever
/// one is dropped is one the fold owes.
#[test]
fn the_weighted_defined_var_fold_keeps_the_show_set_ascending() {
    use num_bigint::BigInt;
    use num_rational::BigRational;
    let r = |num: i64, den: i64| BigRational::new(BigInt::from(num), BigInt::from(den));
    let cl = |ls: &[i32]| Clause::new(ls.iter().map(|l| Literal::from(*l)).collect());

    let formula = CnfFormula {
        num_vars: 4,
        clauses: [
            // 1 ≡ 2 ∧ 3
            &[-1, 2][..],
            &[-1, 3][..],
            &[1, -2, -3][..],
            // ...and constraints that keep 2, 3, 4 alive.
            &[2, 3, 4][..],
            &[-2, -3, 4][..],
            &[-4, 2, 3][..],
        ]
        .iter()
        .map(|c| cl(c))
        .collect(),
    };
    let show = ShowSet::<Reduced>::from_zero_based([0, 1, 2, 3]);
    let weights: Vec<(i32, BigRational)> = (1..=4i32)
        .flat_map(|v| [(v, r(i64::from(v) + 1, 1)), (-v, r(1, i64::from(v) + 1))])
        .collect();

    let a = match run_arjun_weighted_projected_anytime(
        &formula,
        &show,
        &weights,
        std::time::Duration::from_secs(30),
        false,
    )
    .expect("no VITRI_* knob is set in this test")
    {
        Some(a) => a,
        None => {
            eprintln!("Arjun shim unavailable — skipping weighted defined-var fold ordering");
            return;
        }
    };

    let ids = a.show.as_zero_based();
    assert!(
        ids.windows(2).all(|w| w[0] < w[1]),
        "the returned show set must be strictly ascending, got {ids:?}",
    );
    let mut weighted = a.weights.weighted_vars().peekable();
    assert!(
        weighted.peek().is_some(),
        "the fixture must leave weights on the reduced formula, or neither property is under test",
    );
    for v in weighted {
        assert!(
            a.show.contains(v),
            "reduced variable {} carries a weight but is not shown; show = {ids:?}",
            v.idx(),
        );
    }
}
