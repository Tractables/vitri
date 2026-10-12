//! What one round eliminates, and what each elimination does to the clauses.
//!
//! The phases are checked on the clause set they leave; counts are checked
//! against enumeration in `count_preserve`.

use super::*;

fn no_frozen() -> rustc_hash::FxHashSet<VarId> {
    rustc_hash::FxHashSet::default()
}

#[test]
fn a_variable_defined_by_an_and_gate_is_eliminated() {
    // y(3) = x1(1) ∧ x2(2), plus a non-gate clause (1 ∨ 2).
    let f = make_formula(
        3,
        vec![vec![-3, 1], vec![-3, 2], vec![3, -1, -2], vec![1, 2]],
    );
    for gate_definitions in [true, false] {
        let result = preprocess_dve(&f, config(&no_frozen(), gate_definitions));
        assert!(
            result.num_defined() >= 1,
            "gate_definitions={gate_definitions}: expected a defined variable eliminated, got {}",
            result.num_defined()
        );
        assert!(
            result.formula.num_vars() <= 2,
            "gate_definitions={gate_definitions}: expected at most 2 variables left, got {}",
            result.formula.num_vars()
        );
        assert_count_preserved(&f, &result);
    }
}

#[test]
fn nothing_is_eliminated_when_no_variable_is_a_function_of_the_others() {
    // 5 models, no variable is a function of the others.
    let f = make_formula(3, vec![vec![1, 2, 3], vec![-1, 2, -3], vec![1, -2, -3]]);
    for gate_definitions in [true, false] {
        let result = preprocess_dve(&f, config(&no_frozen(), gate_definitions));
        assert_eq!(
            result.num_defined(),
            0,
            "gate_definitions={gate_definitions}"
        );
    }
}

#[test]
fn a_gate_defined_in_terms_of_another_gate_is_eliminated_too() {
    // y(3) = x1(1) ∧ x2(2); z(4) = y(3) ∧ x3(5).
    let f = make_formula(
        5,
        vec![
            vec![-3, 1],
            vec![-3, 2],
            vec![3, -1, -2],
            vec![-4, 3],
            vec![-4, 5],
            vec![4, -3, -5],
        ],
    );
    for gate_definitions in [true, false] {
        let result = preprocess_dve(&f, config(&no_frozen(), gate_definitions));
        assert!(
            result.num_defined() >= 2,
            "gate_definitions={gate_definitions}: expected at least 2 defined, got {}",
            result.num_defined()
        );
        assert_count_preserved(&f, &result);
    }
}

#[test]
fn bve_equiv_within_dve_folded() {
    // x1 ≡ x2 through binary clauses, plus a gate y(4) = x1 ∧ x3.
    let f = make_formula(
        4,
        vec![
            vec![1, -2],
            vec![-1, 2],
            vec![-4, 1],
            vec![-4, 3],
            vec![4, -1, -3],
        ],
    );
    let frozen = no_frozen();
    let mut cfg = config(&frozen, true);
    cfg.keep_original_vars = true;
    let result = preprocess_dve(&f, cfg);
    assert!(
        result.num_equiv() >= 1,
        "Expected at least 1 equiv var, got {}",
        result.num_equiv()
    );
}

/// With a gate definition, only the pairs with a definition clause on one side
/// are resolved; the pair of two other clauses is implied by them and left out.
#[test]
fn a_gate_elimination_does_not_resolve_two_clauses_outside_the_definition() {
    // x(3) = a(1) ∧ b(2); outside the definition: (x ∨ c) and (¬x ∨ d).
    let f = make_formula(
        5,
        vec![
            vec![-3, 1],
            vec![-3, 2],
            vec![3, -1, -2],
            vec![3, 4],
            vec![-3, 5],
        ],
    );
    let frozen = no_frozen();
    let mut fates = vec![DveFate::Kept; 5];
    let mut e = engine(&f, &mut fates, &frozen, true);
    assert_eq!(eliminate_by_gate(&mut e, 3), Attempt::Eliminated);
    let clauses = live_clauses(&e);
    assert_eq!(
        clauses,
        vec![vec![-1, -2, 5], vec![1, 4], vec![2, 4]],
        "definition-side resolvents only; (4 ∨ 5), from the two outside clauses, must be absent"
    );
    assert!(!clauses.contains(&vec![4, 5]));

    let original = brute_force_mc(&f);
    let remaining = e.db.into_clauses();
    assert_eq!(count_after(&mut fates, 5, remaining), original);
}

/// Without a definition every pair is resolved, so the same variable gives the
/// pair of outside clauses too (when it is not subsumed by what is there).
#[test]
fn an_elimination_without_a_definition_resolves_every_pair() {
    let f = make_formula(5, vec![vec![3, 4], vec![-3, 5]]);
    let frozen = no_frozen();
    let mut fates = vec![DveFate::Kept; 5];
    let mut e = engine(&f, &mut fates, &frozen, false);
    assert_eq!(
        e.eliminate(VarId::from_dimacs(3), None),
        Attempt::Eliminated
    );
    assert_eq!(live_clauses(&e), vec![vec![4, 5]]);
    assert_eq!(fates[2], DveFate::Defined);
}

/// A resolvent that subsumes a live clause removes it, and the resolvents of one
/// elimination are kept minimal among themselves.
#[test]
fn a_resolvent_removes_the_live_clauses_it_subsumes() {
    // Eliminating b(2): resolvents (3) and (4); (3 ∨ ¬1) is subsumed by (3).
    // (3 ∨ 4) and (3 ∨ 5), also resolvents, are subsumed by the others.
    let f = make_formula(
        5,
        vec![
            vec![1, 5],
            vec![-1, 3],
            vec![2, 3],
            vec![2, 4],
            vec![-2, 3],
            vec![-2, 4],
            vec![-2, 5],
        ],
    );
    let frozen = no_frozen();
    let mut fates = vec![DveFate::Kept; 5];
    let mut e = engine(&f, &mut fates, &frozen, false);
    assert_eq!(
        e.eliminate(VarId::from_dimacs(2), None),
        Attempt::Eliminated
    );
    assert_eq!(live_clauses(&e), vec![vec![1, 5], vec![3], vec![4]]);
}

/// A resolvent that a live clause subsumes is not added; an equal one is not
/// added twice.
#[test]
fn a_resolvent_a_live_clause_subsumes_is_not_added() {
    let frozen = no_frozen();

    // (x ∨ a), (¬x ∨ b) resolve to (a ∨ b), which is already there.
    let f = make_formula(3, vec![vec![1, 2], vec![-1, 3], vec![2, 3]]);
    let mut fates = vec![DveFate::Kept; 3];
    let mut e = engine(&f, &mut fates, &frozen, false);
    assert_eq!(
        e.eliminate(VarId::from_dimacs(1), None),
        Attempt::Eliminated
    );
    assert_eq!(live_clauses(&e), vec![vec![2, 3]]);

    // The unit (a) subsumes the resolvent (a ∨ b) outright.
    let f = make_formula(3, vec![vec![1, 2], vec![-1, 3], vec![2]]);
    let mut fates = vec![DveFate::Kept; 3];
    let mut e = engine(&f, &mut fates, &frozen, false);
    assert_eq!(
        e.eliminate(VarId::from_dimacs(1), None),
        Attempt::Eliminated
    );
    assert_eq!(live_clauses(&e), vec![vec![2]]);
}

#[test]
fn an_elimination_that_adds_two_more_clauses_than_it_removes_is_refused() {
    // Two positive and four negative clauses, no resolvent a tautology or
    // subsumed: eight resolvents for six clauses, one beyond the bound.
    assert_eq!(
        RESOLVENT_GROWTH_BOUND, 1,
        "the cases here assume growth one"
    );
    let f = make_formula(
        7,
        vec![
            vec![1, 2],
            vec![1, 3],
            vec![-1, 4],
            vec![-1, 5],
            vec![-1, 6],
            vec![-1, 7],
        ],
    );
    let frozen = no_frozen();
    let mut fates = vec![DveFate::Kept; 7];
    let mut e = engine(&f, &mut fates, &frozen, false);
    let before = live_clauses(&e);
    assert_eq!(e.eliminate(VarId::from_dimacs(1), None), Attempt::Kept);
    assert_eq!(
        live_clauses(&e),
        before,
        "a refused elimination edits nothing"
    );
    assert_eq!(fates[0], DveFate::Kept);
}

#[test]
fn an_elimination_that_adds_one_more_clause_than_it_removes_goes_through() {
    // Two and three: six resolvents for five clauses, at the bound.
    assert_eq!(
        RESOLVENT_GROWTH_BOUND, 1,
        "the cases here assume growth one"
    );
    let f = make_formula(
        6,
        vec![
            vec![1, 2],
            vec![1, 3],
            vec![-1, 4],
            vec![-1, 5],
            vec![-1, 6],
        ],
    );
    let frozen = no_frozen();
    let mut fates = vec![DveFate::Kept; 6];
    let mut e = engine(&f, &mut fates, &frozen, false);
    assert_eq!(
        e.eliminate(VarId::from_dimacs(1), None),
        Attempt::Eliminated
    );
    assert_eq!(live_clauses(&e).len(), 6);
}

/// `n` positive-side literals `2..` and `n` negative-side ones after them: the
/// one resolvent of `(x ∨ …)` and `(¬x ∨ …)` has `2n` literals.
fn long_pair(n: usize) -> CnfFormula {
    let pos: Vec<i32> = std::iter::once(1).chain(2..2 + n as i32).collect();
    let neg: Vec<i32> = std::iter::once(-1)
        .chain(2 + n as i32..2 + 2 * n as i32)
        .collect();
    make_formula(1 + 2 * n as u32, vec![pos, neg])
}

#[test]
fn a_resolvent_longer_than_the_length_bound_is_refused_but_one_at_it_is_not() {
    let frozen = no_frozen();

    let f = long_pair(MAX_RESOLVENT_LEN / 2 + 1);
    let mut fates = vec![DveFate::Kept; f.num_vars() as usize];
    let mut e = engine(&f, &mut fates, &frozen, false);
    assert_eq!(e.eliminate(VarId::from_dimacs(1), None), Attempt::Kept);
    assert_eq!(live_clauses(&e).len(), 2);

    let f = long_pair(MAX_RESOLVENT_LEN / 2);
    let mut fates = vec![DveFate::Kept; f.num_vars() as usize];
    let mut e = engine(&f, &mut fates, &frozen, false);
    assert_eq!(
        e.eliminate(VarId::from_dimacs(1), None),
        Attempt::Eliminated
    );
    assert_eq!(live_clauses(&e).len(), 1);
}

#[test]
fn a_variable_in_no_live_clause_is_absent_not_eliminated() {
    let f = make_formula(3, vec![vec![1, 2]]);
    let frozen = no_frozen();
    let mut fates = vec![DveFate::Kept; 3];
    let mut e = engine(&f, &mut fates, &frozen, false);
    assert_eq!(e.eliminate(VarId::from_dimacs(3), None), Attempt::Absent);
    assert_eq!(fates[2], DveFate::Kept);
}

/// The probed phase eliminates members of the probed set and nothing else:
/// variable 4 below has a cheap elimination but is not defined.
#[test]
fn the_probed_phase_eliminates_only_the_probed_set() {
    // y(3) = a(1) ∧ b(2); (4 ∨ 5), (¬4 ∨ 6): 4 is no function of the others.
    let f = make_formula(
        6,
        vec![
            vec![-3, 1],
            vec![-3, 2],
            vec![3, -1, -2],
            vec![1, 2, 6],
            vec![4, 5],
            vec![-4, 6],
        ],
    );
    let n = f.num_vars() as usize;
    let frozen = no_frozen();

    // The set the probe returns, found on its own copy of the formula.
    let mut fates = vec![DveFate::Kept; n];
    let e = engine(&f, &mut fates, &frozen, false);
    let order = e.probe_order();
    let probed =
        pick_def_vars_with_meter(f.clauses(), n, &order, 10_000, &mut wall_meter(), |_, _| {
            false
        });
    assert!(
        !probed.is_empty(),
        "the probe found nothing; the test guards nothing"
    );
    assert!(!probed.contains(&3), "variable 4 is not defined");

    let mut clauses = f.clauses().to_vec();
    let mut fates = vec![DveFate::Kept; n];
    let round = dve_round(
        &mut clauses,
        n,
        &mut fates,
        10_000,
        false,
        &frozen,
        &mut wall_meter(),
    );
    assert_eq!(round.by_gate, 0, "the gate phase is off");
    assert!(round.probed >= 1);
    for (v, fate) in fates.iter().enumerate() {
        if *fate == DveFate::Defined {
            assert!(
                probed.contains(&(v as u32)),
                "variable {} was eliminated but is not in the probed set {probed:?}",
                v + 1
            );
        }
    }
    assert_eq!(fates[3], DveFate::Kept);
    assert_eq!(count_after(&mut fates, n, clauses), brute_force_mc(&f));
}

/// The gate phase takes a gate output the probe did not return, and only a
/// variable that has a gate.
#[test]
fn the_gate_phase_eliminates_gate_outputs_and_nothing_without_a_gate() {
    let f = make_formula(
        6,
        vec![
            vec![-3, 1],
            vec![-3, 2],
            vec![3, -1, -2],
            vec![1, 2, 6],
            vec![4, 5],
            vec![-4, 6],
        ],
    );
    let n = f.num_vars() as usize;
    let frozen = no_frozen();
    let mut clauses = f.clauses().to_vec();
    let mut fates = vec![DveFate::Kept; n];
    let round = dve_round(
        &mut clauses,
        n,
        &mut fates,
        10_000,
        true,
        &frozen,
        &mut wall_meter(),
    );
    assert_eq!(fates[2], DveFate::Defined, "the AND output goes");
    assert!(round.eliminated() >= 1);
    assert_eq!(
        fates[3],
        DveFate::Kept,
        "variable 4 has no gate and is not defined"
    );
    assert_eq!(count_after(&mut fates, n, clauses), brute_force_mc(&f));
}

#[test]
fn a_frozen_variable_is_never_eliminated() {
    let f = make_formula(
        4,
        vec![vec![-3, 1], vec![-3, 2], vec![3, -1, -2], vec![3, 4]],
    );
    let n = f.num_vars() as usize;
    for gate_definitions in [true, false] {
        // Unfrozen, the gate output goes: otherwise the frozen run proves nothing.
        let mut clauses = f.clauses().to_vec();
        let mut fates = vec![DveFate::Kept; n];
        dve_round(
            &mut clauses,
            n,
            &mut fates,
            10_000,
            gate_definitions,
            &no_frozen(),
            &mut wall_meter(),
        );
        assert_eq!(
            fates[2],
            DveFate::Defined,
            "gate_definitions={gate_definitions}"
        );

        let mut frozen = no_frozen();
        frozen.insert(VarId::from_dimacs(3));
        let mut clauses = f.clauses().to_vec();
        let mut fates = vec![DveFate::Kept; n];
        dve_round(
            &mut clauses,
            n,
            &mut fates,
            10_000,
            gate_definitions,
            &frozen,
            &mut wall_meter(),
        );
        assert_eq!(
            fates[2],
            DveFate::Kept,
            "gate_definitions={gate_definitions}"
        );

        let result = preprocess_dve(&f, config(&frozen, gate_definitions));
        assert_eq!(
            result.fates[2],
            DveFate::Kept,
            "gate_definitions={gate_definitions}"
        );
        assert_count_preserved(&f, &result);
    }
}

/// An elimination that a later one requeues is retried: eliminating `y` makes
/// `z` cheap enough to go.
#[test]
fn an_elimination_that_becomes_possible_after_another_is_found() {
    // z(4) = y(3) ∧ w(5); y = a ∧ b. Both are gates; the order must not matter.
    let f = make_formula(
        5,
        vec![
            vec![-3, 1],
            vec![-3, 2],
            vec![3, -1, -2],
            vec![-4, 3],
            vec![-4, 5],
            vec![4, -3, -5],
        ],
    );
    let n = f.num_vars() as usize;
    let mut clauses = f.clauses().to_vec();
    let mut fates = vec![DveFate::Kept; n];
    dve_round(
        &mut clauses,
        n,
        &mut fates,
        10_000,
        true,
        &no_frozen(),
        &mut wall_meter(),
    );
    assert_eq!(fates[2], DveFate::Defined);
    assert_eq!(fates[3], DveFate::Defined);
    assert_eq!(count_after(&mut fates, n, clauses), brute_force_mc(&f));
}
