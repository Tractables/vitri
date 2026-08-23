//! Each case reduces a small formula, counts the residual exhaustively,
//! applies the reported multiplier, and demands the original count back —
//! so a multiplier that is wrong in a way the reduction compensates for
//! still fails.

use super::*;
use crate::tests::common::lit;
use crate::tests::pmc_oracle::{brute_force_mc, brute_force_wmc};

/// Soundness invariant for the in-process weighted reduction: the weighted
/// count of the reduced formula (under its reduced weights) scaled by the
/// rational multiplier K must equal the original formula's weighted count.
/// Includes a free var (in no clause) with asymmetric weights so the
/// reduction MUST fold its weighted mass into K (the K-collapse hazard the
/// no-`c p show` path guards against).
#[test]
fn anytime_weighted_count_preserving() {
    // 0→1→2 implication chain (BVE-eliminable); var 3 free (no clause), var 4
    // also free. Asymmetric weights everywhere.
    let formula = CnfFormula {
        num_vars: 5,
        clauses: vec![
            Clause::new(vec![lit(0, false), lit(1, true)]), // 0 ⇒ 1
            Clause::new(vec![lit(1, false), lit(2, true)]), // 1 ⇒ 2
        ],
    };
    let wpos: Vec<BigRational> = ["2/1", "3/1", "5/1", "7/1", "1/3"]
        .iter()
        .map(|s| crate::cnf::parse_weight(s).unwrap())
        .collect();
    let wneg: Vec<BigRational> = ["1/1", "1/2", "2/3", "1/1", "4/1"]
        .iter()
        .map(|s| crate::cnf::parse_weight(s).unwrap())
        .collect();
    let expected = brute_force_wmc(&formula, |v, val| {
        let i = v as usize;
        if val {
            wpos[i].clone()
        } else {
            wneg[i].clone()
        }
    });

    // Build the weights_in vec (both polarities) the way the WMC cascade does.
    let mut weights_in: Vec<(i32, BigRational)> = Vec::new();
    for v in 0..formula.num_vars {
        let d = VarId(v).to_dimacs();
        weights_in.push((d, wpos[v as usize].clone()));
        weights_in.push((-d, wneg[v as usize].clone()));
    }

    let deadline = Instant::now() + Duration::from_secs(30);
    let r = reduce_anytime_weighted(&formula, &weights_in, deadline, /*no_sbva=*/ false)
        .expect("the environment is usable")
        .expect("reduce");

    let (rneg, rpos): (Vec<BigRational>, Vec<BigRational>) =
        r.weights.as_pairs().iter().cloned().unzip();
    let reduced = brute_force_wmc(&r.formula, |v, val| {
        let i = v as usize;
        if val {
            rpos[i].clone()
        } else {
            rneg[i].clone()
        }
    });
    let got = reduced * &r.multiplier;
    assert_eq!(
        got, expected,
        "weighted soundness violated: reduced × K = {got} != {expected} (orig weighted count)"
    );
}

#[test]
fn power_of_two_exp() {
    assert_eq!(multiplier_decimal_to_exp("1"), Some(0));
    assert_eq!(multiplier_decimal_to_exp("32"), Some(5));
    assert_eq!(multiplier_decimal_to_exp("1024"), Some(10));
    // 2^100 — exceeds u128, must still work via bigint.
    let p100 = "1267650600228229401496703205376";
    assert_eq!(multiplier_decimal_to_exp(p100), Some(100));
    assert_eq!(multiplier_decimal_to_exp("3"), None);
    assert_eq!(multiplier_decimal_to_exp("0"), None);
    assert_eq!(multiplier_decimal_to_exp("48"), None); // 16*3
}

/// Whatever `reduce_anytime` reduces, the count of the reduced formula
/// scaled by its `2^multiplier_exp` MUST equal the count of the original.
#[test]
fn anytime_count_preserving() {
    // Same shape as the subprocess soundness test: a free var (4, in no
    // clause) that doubles the count and is the canonical thing arjun folds
    // into the multiplier, plus a short implication chain BVE can eliminate.
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
    let expected = brute_force_mc(&formula);
    let deadline = Instant::now() + Duration::from_secs(30);
    let r = reduce_anytime(
        &formula,
        deadline,
        ArjunEffort::Full,
        ArjunOptions::default(),
    )
    .expect("no VITRI_* knob is set in this test")
    .expect("reduce");
    let reduced = brute_force_mc(&r.formula);
    let got = reduced.clone() << r.multiplier_exp;
    assert_eq!(
        got, expected,
        "soundness violated: reduced {} << {} = {} != {} (orig full count)",
        reduced, r.multiplier_exp, got, expected
    );
}

/// The no-SBVA reduction (the OOM-triggered revert target a caller asks for
/// with `force_no_sbva = true`) must be just as count-preserving as the
/// default.
#[test]
fn anytime_count_preserving_no_sbva() {
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
    let expected = brute_force_mc(&formula);
    let deadline = Instant::now() + Duration::from_secs(30);
    let r = reduce_anytime(
        &formula,
        deadline,
        ArjunEffort::Full,
        ArjunOptions {
            force_no_sbva: true,
            ..ArjunOptions::default()
        },
    )
    .expect("no VITRI_* knob is set in this test")
    .expect("reduce");
    let reduced = brute_force_mc(&r.formula);
    let got = reduced.clone() << r.multiplier_exp;
    assert_eq!(
        got, expected,
        "no-SBVA soundness violated: reduced {} << {} = {} != {} (orig full count)",
        reduced, r.multiplier_exp, got, expected
    );
}

/// Seeding soundness: the backbone units + equiv clauses harvested from
/// Arjun's minimize stage, when appended to the ORIGINAL formula, must
/// preserve its full model count exactly (they are redundant — satisfied by
/// every model). This is the invariant the raw-lane seeding relies on, in
/// the same var space the raw lane augments (no renumber). Also asserts the
/// harvest actually fires (backbone non-empty) on a formula with a forced
/// literal, so the test would catch a silently-empty getter.
#[test]
fn seed_backbone_equiv_count_preserving() {
    // var 0 forced true (unit); 0→1→2 implication chain; var 3 mirrors var 4
    // via (3≡4)-style binaries so a binary-xor equivalence is available.
    let formula = CnfFormula {
        num_vars: 5,
        clauses: vec![
            Clause::new(vec![lit(0, true)]),                // 0 is backbone
            Clause::new(vec![lit(0, false), lit(1, true)]), // 0 ⇒ 1
            Clause::new(vec![lit(1, false), lit(2, true)]), // 1 ⇒ 2
            Clause::new(vec![lit(3, true), lit(4, false)]), // 3 ∨ ¬4
            Clause::new(vec![lit(3, false), lit(4, true)]), // ¬3 ∨ 4  (3 ≡ 4)
        ],
    };
    let expected = brute_force_mc(&formula);
    let deadline = Instant::now() + Duration::from_secs(30);
    let r = reduce_anytime(
        &formula,
        deadline,
        ArjunEffort::Full,
        ArjunOptions::default(),
    )
    .expect("no VITRI_* knob is set in this test")
    .expect("reduce");

    // Harvest must fire: var 0 is forced, so backbone is non-empty.
    assert!(
        !r.backbone.is_empty(),
        "expected a non-empty backbone harvest"
    );

    // Augment the ORIGINAL formula exactly as the raw lane's seeding does,
    // then brute-count — must be unchanged.
    let mut seeded = formula.clone();
    for &l in &r.backbone {
        assert!(
            l.var.0 < formula.num_vars,
            "backbone var out of input space"
        );
        seeded.clauses.push(Clause::new(vec![l]));
    }
    for &(a, b) in &r.equiv {
        assert!(
            a.var.0 < formula.num_vars && b.var.0 < formula.num_vars,
            "equiv var out of input space"
        );
        seeded.clauses.push(Clause::new(vec![a, b.negated()]));
        seeded.clauses.push(Clause::new(vec![a.negated(), b]));
    }
    assert_eq!(
        brute_force_mc(&seeded),
        expected,
        "seeding backbone+equiv changed the model count (unsound translation/encoding)"
    );
}
