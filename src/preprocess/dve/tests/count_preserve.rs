//! Every case enumerates both counts exhaustively with [`brute_force_mc`]
//! rather than comparing against a second implementation, so a bug shared by
//! both sides cannot hide.

use super::*;

/// 12 variables, 19 clauses, 64 models. Variables 3, 5, 10, 11 and 12 are
/// detectable gates; variable 4 hides `4 ↔ 1 ∧ ¬2`, an AND with a negated
/// input, which no syntactic pattern here matches but the probe proves.
fn mixed_gates_formula() -> CnfFormula {
    let f = make_formula(
        12,
        vec![
            vec![3, 4, -5],
            vec![1, -3],
            vec![2, -3],
            vec![-3, 5],
            vec![-1, -2, 3],
            vec![1, -4],
            vec![-2, -4],
            vec![-4, 5],
            vec![-1, 2, 4],
            vec![5, -12],
            vec![-5, -10, -11, 12],
            vec![10, -12],
            vec![6, -10],
            vec![7, -10],
            vec![-6, -7, 10],
            vec![11, -12],
            vec![8, -11],
            vec![9, -11],
            vec![-8, -9, 11],
        ],
    );
    assert_eq!(brute_force_mc(&f), BigUint::from(64u32));
    f
}

/// Eliminating only the syntactic gate outputs, outputs before inputs and one by
/// one through their definitions, preserves the count: each is defined in the formula it is
/// eliminated from.
#[test]
fn eliminating_the_gate_outputs_through_their_definitions_preserves_the_count() {
    let f = mixed_gates_formula();
    let frozen = rustc_hash::FxHashSet::default();
    let mut fates = vec![DveFate::Kept; 12];
    let mut e = engine(&f, &mut fates, &frozen, true);
    let eliminated = [12, 11, 10, 5, 3]
        .into_iter()
        .filter(|&x| eliminate_by_gate(&mut e, x) == Attempt::Eliminated)
        .count();
    assert!(
        eliminated >= 1,
        "no gate was eliminated; the test guards nothing"
    );
    let clauses = e.db.into_clauses();
    assert_eq!(count_after(&mut fates, 12, clauses), BigUint::from(64u32));
}

/// Gate elimination followed by a probed round on what is left (the hidden
/// gate on variable 4 is only found by the probe).
#[test]
fn a_probed_round_after_gate_elimination_preserves_the_count() {
    let f = mixed_gates_formula();
    let frozen = rustc_hash::FxHashSet::default();
    let mut fates = vec![DveFate::Kept; 12];
    let mut e = engine(&f, &mut fates, &frozen, true);
    for x in [12, 11, 10, 5, 3] {
        eliminate_by_gate(&mut e, x);
    }
    let mut clauses = e.db.into_clauses();
    dve_round(
        &mut clauses,
        12,
        &mut fates,
        10_000,
        false,
        &frozen,
        &mut wall_meter(),
    );
    assert_eq!(count_after(&mut fates, 12, clauses), BigUint::from(64u32));
}

/// Regression: the probe shares gate variables between the two copies of the
/// formula, so a candidate can look defined only because the gate clauses
/// still pin them. Eliminating the gates and the candidate together would not
/// preserve the count; the elimination is one variable at a time, each against
/// the formula as it then stands, so the count must come out whole.
///
/// Combined elimination of this formula once produced 48 for 64 models.
#[test]
fn the_full_pass_preserves_the_count_of_a_formula_with_hidden_gates() {
    let f = mixed_gates_formula();
    for gate_definitions in [true, false] {
        let result = preprocess_dve(
            &f,
            config(&rustc_hash::FxHashSet::default(), gate_definitions),
        );
        assert_count_preserved(&f, &result);
    }
}

/// Regression: pure-literal elimination is sound for SAT but UNSOUND for #SAT.
/// A variable with one polarity left must not be forced and its clauses
/// dropped: that undercounts models.
///
/// φ = (V ∨ A) has 3 models. Forcing V true drops (V ∨ A) and loses the model
/// V=0, A=1. V is not defined by A, so it must stay in the formula.
#[test]
fn a_variable_with_one_polarity_is_not_dropped_as_if_pure() {
    let f = make_formula(2, vec![vec![1, 2]]);
    assert_eq!(brute_force_mc(&f), BigUint::from(3u32));
    for gate_definitions in [true, false] {
        let result = preprocess_dve(
            &f,
            config(&rustc_hash::FxHashSet::default(), gate_definitions),
        );
        assert_count_preserved(&f, &result);
    }
}

#[test]
fn an_equivalence_followed_by_gate_elimination_preserves_the_count() {
    // Y=1, X=2, A=3. OR gate Y = A ∨ X plus (¬Y ∨ X), which makes Y ≡ X.
    let f = make_formula(
        3,
        vec![vec![1, -3], vec![1, -2], vec![-1, 3, 2], vec![-1, 2]],
    );
    // Satisfying (Y,X,A): (0,0,0), (1,1,0), (1,1,1).
    assert_eq!(brute_force_mc(&f), BigUint::from(3u32));
    for gate_definitions in [true, false] {
        let result = preprocess_dve(
            &f,
            config(&rustc_hash::FxHashSet::default(), gate_definitions),
        );
        assert_count_preserved(&f, &result);
    }
}

/// Regression for the shared-XOR bug: of two variables of one XOR cluster
/// that are both eliminable, resolving the first removes all four XOR clauses
/// and leaves the second in none. The second must be credited as free (×2),
/// not defined (×1), or the count is halved.
#[test]
fn the_second_variable_of_a_shared_xor_is_counted_free() {
    // v1 ⊕ v2 ⊕ v3 = 0: four models of eight.
    let f = make_formula(
        3,
        vec![
            vec![1, 2, -3],
            vec![1, -2, 3],
            vec![-1, 2, 3],
            vec![-1, -2, -3],
        ],
    );
    assert_eq!(brute_force_mc(&f), BigUint::from(4u32));
    for gate_definitions in [true, false] {
        let result = preprocess_dve(
            &f,
            config(&rustc_hash::FxHashSet::default(), gate_definitions),
        );
        assert_count_preserved(&f, &result);
        assert_eq!(
            result.num_free(),
            2,
            "gate_definitions={gate_definitions}: eliminating one XOR variable leaves the \
             other two in no clause, so both are free"
        );
    }
}

// ---------------------------------------------------------------------------
// Property test.

/// The clauses of a gate over random literals: AND, ITE or XOR, so that the
/// gate phase has something to find.
fn planted_gate(rng: &mut Rng, num_vars: usize) -> Vec<Vec<i32>> {
    let l = rng.literals(num_vars, 4);
    let (x, a, b, c) = (l[0], l[1], l[2], l[3]);
    match rng.below(3) {
        0 => vec![vec![-x, a], vec![-x, b], vec![x, -a, -b]],
        1 => vec![
            vec![-x, -a, b],
            vec![x, -a, -b],
            vec![-x, a, c],
            vec![x, a, -c],
        ],
        _ => vec![
            vec![-x, a, b],
            vec![x, -a, b],
            vec![x, a, -b],
            vec![-x, -a, -b],
        ],
    }
}

/// Random small formula: up to 12 variables, clauses of 1 to 4 literals, and
/// planted gates over some of them.
fn random_formula(rng: &mut Rng) -> CnfFormula {
    let num_vars = 4 + rng.below(9);
    let mut clauses: Vec<Vec<i32>> = Vec::new();
    for _ in 0..rng.below(3) + 1 {
        if rng.below(4) != 0 {
            clauses.extend(planted_gate(rng, num_vars));
        }
    }
    for _ in 0..num_vars / 2 + rng.below(num_vars) {
        let len = 1 + rng.below(4);
        clauses.push(rng.literals(num_vars, len));
    }
    make_formula(num_vars as u32, clauses)
}

/// On many seeded random formulas, with and without gate definitions and with
/// a random frozen set, the pass keeps the model count and never eliminates a
/// frozen variable.
#[test]
fn the_pass_preserves_the_model_count_of_random_formulas() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut eliminated_somewhere = false;
    for case in 0..250 {
        let f = random_formula(&mut rng);
        let n = f.num_vars() as usize;
        let mut frozen = rustc_hash::FxHashSet::default();
        if case % 3 == 0 {
            for _ in 0..1 + rng.below(2) {
                frozen.insert(VarId::from_idx(rng.below(n)));
            }
        }
        for gate_definitions in [true, false] {
            let result = preprocess_dve(&f, config(&frozen, gate_definitions));
            assert_count_preserved(&f, &result);
            for &v in &frozen {
                assert_ne!(
                    result.fates[v.idx()],
                    DveFate::Defined,
                    "case {case}, gate_definitions={gate_definitions}: frozen variable {} eliminated",
                    v.to_dimacs()
                );
            }
            eliminated_somewhere |= result.num_defined() > 0;
        }
    }
    assert!(
        eliminated_somewhere,
        "no case eliminated anything; the test guards nothing"
    );
}
