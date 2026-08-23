use super::*;

#[test]
fn a_variable_defined_by_an_and_gate_is_eliminated() {
    // y(3) = x1(1) ∧ x2(2)
    // Clauses: (¬3 ∨ 1), (¬3 ∨ 2), (3 ∨ ¬1 ∨ ¬2)
    // Plus a non-gate clause: (1 ∨ 2)
    let f = make_formula(
        3,
        vec![vec![-3, 1], vec![-3, 2], vec![3, -1, -2], vec![1, 2]],
    );
    let result = preprocess_dve(
        &f,
        10,
        10_000,
        false,
        &rustc_hash::FxHashSet::default(),
        &rustc_hash::FxHashSet::default(),
        FrozenEquiv::Ignore,
    );
    // Variable 3 (VarId(2)) should be eliminated as defined
    assert!(
        result.num_defined() >= 1,
        "Expected at least 1 defined var eliminated, got {}",
        result.num_defined()
    );
    assert!(
        result.formula.num_vars <= 2,
        "Expected at most 2 vars remaining, got {}",
        result.formula.num_vars
    );
}

#[test]
fn nothing_is_eliminated_when_no_variable_is_a_function_of_the_others() {
    // 5 models, no variable is a function of the others:
    // (1∨2∨3), (¬1∨2∨¬3), (1∨¬2∨¬3)
    let f = make_formula(3, vec![vec![1, 2, 3], vec![-1, 2, -3], vec![1, -2, -3]]);
    let result = preprocess_dve(
        &f,
        10,
        10_000,
        false,
        &rustc_hash::FxHashSet::default(),
        &rustc_hash::FxHashSet::default(),
        FrozenEquiv::Ignore,
    );
    assert_eq!(result.num_defined(), 0, "Expected no defined vars");
}

#[test]
fn a_gate_defined_in_terms_of_another_gate_is_eliminated_too() {
    // y(3) = x1(1) ∧ x2(2): (-3,1), (-3,2), (3,-1,-2)
    // z(4) = y(3) ∧ x3(5): (-4,3), (-4,5), (4,-3,-5)
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
    let result = preprocess_dve(
        &f,
        10,
        10_000,
        false,
        &rustc_hash::FxHashSet::default(),
        &rustc_hash::FxHashSet::default(),
        FrozenEquiv::Ignore,
    );
    assert!(
        result.num_defined() >= 2,
        "Expected at least 2 defined vars, got {}",
        result.num_defined()
    );
}

/// BVE mode: definition_clauses must have one entry per eliminated variable
/// (both DVE-eliminated and equivalence-merged within DVE).
#[test]
fn bve_definition_clauses_cover_all_eliminated() {
    // y(3) = x1(1) ∧ x2(2): (-3,1), (-3,2), (3,-1,-2)
    // z(4) = y(3) ∧ x3(5): (-4,3), (-4,5), (4,-3,-5)
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
    let result = preprocess_dve(
        &f,
        10,
        10_000,
        true,
        &rustc_hash::FxHashSet::default(),
        &rustc_hash::FxHashSet::default(),
        FrozenEquiv::Ignore,
    );
    // definition_clauses contains both defined and equiv variable clauses.
    let expected = result.num_defined() + result.num_equiv();
    assert_eq!(
        result.definition_clauses.len(),
        expected,
        "definition_clauses.len()={} but num_defined+num_equiv={}",
        result.definition_clauses.len(),
        expected,
    );
}

#[test]
fn bve_equiv_within_dve_folded() {
    // Create a formula where DVE will find equivalences internally:
    // x1 ≡ x2 (via binary clauses), plus a gate y = x1 ∧ x3.
    // Vars: 1=x1, 2=x2, 3=x3, 4=y
    // Equivalence: (1 ∨ ¬2) ∧ (¬1 ∨ 2)  →  x1 ≡ x2
    // Gate: (¬4 ∨ 1) ∧ (¬4 ∨ 3) ∧ (4 ∨ ¬1 ∨ ¬3)
    let f = make_formula(
        4,
        vec![
            vec![1, -2],
            vec![-1, 2], // x1 ≡ x2
            vec![-4, 1],
            vec![-4, 3],
            vec![4, -1, -3], // y = x1 ∧ x3
        ],
    );
    let result = preprocess_dve(
        &f,
        10,
        10_000,
        true,
        &rustc_hash::FxHashSet::default(),
        &rustc_hash::FxHashSet::default(),
        FrozenEquiv::Ignore,
    );
    assert!(
        result.num_equiv() >= 1,
        "Expected at least 1 equiv var, got {}",
        result.num_equiv()
    );
    assert_eq!(
        result.definition_clauses.len(),
        result.num_defined() + result.num_equiv(),
        "definition_clauses.len()={} but num_defined+num_equiv={}",
        result.definition_clauses.len(),
        result.num_defined() + result.num_equiv(),
    );
}

/// Regression for the gate-DVE shared-XOR bug: when two vars from the
/// same XOR cluster are both in `defined` (one preknown from gates, one
/// from the SAT probe) and processed in the same elim_vars call, resolving
/// the first removes all 4 XOR clauses, leaving the second clauseless.
/// The second must be credited as ×2 free, not ×1 defined, or MC is halved.
#[test]
fn dve_shared_xor_counts_second_var_as_free() {
    // XOR: v1 ⊕ v2 ⊕ v3 = 0 (4 models out of 8). Four clauses, each rules
    // out one odd-parity assignment.
    //   (v1 ∨ v2 ∨ ¬v3), (v1 ∨ ¬v2 ∨ v3), (¬v1 ∨ v2 ∨ v3), (¬v1 ∨ ¬v2 ∨ ¬v3)
    let f = make_formula(
        3,
        vec![
            vec![1, 2, -3],
            vec![1, -2, 3],
            vec![-1, 2, 3],
            vec![-1, -2, -3],
        ],
    );
    // Force both v1 (0) and v2 (1) into preknown — simulates the real
    // path where gate detection marks v1 and the SAT probe then picks v2.
    let mut known: rustc_hash::FxHashSet<VarId> = rustc_hash::FxHashSet::default();
    known.insert(VarId(0));
    known.insert(VarId(1));
    let result = preprocess_dve(
        &f,
        10,
        10_000,
        false,
        &known,
        &rustc_hash::FxHashSet::default(),
        FrozenEquiv::Ignore,
    );

    // Original MC on (v1,v2,v3) = 4 (even-parity assignments).
    // Reduced formula has 0 clauses → 1 model. So
    // 2^num_free * 1^num_defined must equal 4 → num_free == 2.
    let reduced_mc: u128 = if result.formula.clauses.is_empty() {
        1
    } else {
        0
    };
    let multiplier = 1u128 << result.num_free();
    assert_eq!(
        reduced_mc * multiplier,
        4,
        "MC mismatch: reduced={} * 2^{} = {}, expected 4 (stats: defined={}, free={})",
        reduced_mc,
        result.num_free(),
        reduced_mc * multiplier,
        result.num_defined(),
        result.num_free(),
    );
}

/// Regression for the skip-not-abort fix: when `elim_vars` receives a batch
/// where the FIRST variable is non-resolvent-bounded (pos*neg > pos+neg),
/// it must skip that variable and still process the remaining ones.
///
/// Old code: first var causes blowup → `break` → entire batch aborted.
/// New code: first var is non-R-bounded → `continue` → second var processed.
#[test]
fn elim_vars_eliminates_non_rb_when_formula_fits_after_prior_elims() {
    // a (var 0, DIMACS 1): pos=1, neg=1 → R-bounded
    // b (var 1, DIMACS 2): pos=2, neg=3 → pos*neg=6 > pos+neg=5: nominally NOT R-bounded
    // Other vars: x=2, y=3, z=4 (0-indexed)
    //
    // The blowup guard checks remaining.len() + resolvents > max_clauses, not pos*neg > pos+neg.
    // After eliminating a (2 clauses removed, 1 resolvent added), the formula has 6 clauses.
    // b's elimination: remaining=1, max resolvents=6, total=7 ≤ max_clauses=7 — no blowup.
    // So b IS eliminated even though it's nominally non-R-bounded.
    let f = make_formula(
        5,
        vec![
            vec![1, 5],  // a∨z
            vec![-1, 3], // ¬a∨x
            vec![2, 3],  // b∨x
            vec![2, 4],  // b∨y
            vec![-2, 3], // ¬b∨x
            vec![-2, 4], // ¬b∨y
            vec![-2, 5], // ¬b∨z
        ],
    );
    let mut clauses = f.clauses.clone();
    sort_clause_literals(&mut clauses);

    let orig_len = clauses.len();
    let (elim_ids, _, _) = elim_vars(&mut clauses, &[0u32, 1u32], orig_len, &Default::default());

    assert!(
        elim_ids.contains(&0u32),
        "a (var 0) should be eliminated (R-bounded); elim_ids={:?}",
        elim_ids,
    );
    assert!(
        elim_ids.contains(&1u32),
        "b (var 1) should also be eliminated (fits within max_clauses after a shrinks formula); elim_ids={:?}",
        elim_ids,
    );
}
