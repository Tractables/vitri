//! Every case enumerates both counts exhaustively with [`brute_force_mc`]
//! rather than comparing against a second implementation, so a bug shared by
//! both sides cannot hide.

use super::*;
use crate::tests::pmc_oracle::brute_force_mc;
use num_bigint::BigUint;

/// Diagnostic: eliminating ONLY the gate DAG preknown must preserve MC.
/// Each gate output is individually functional in the original, so BVE on
/// each is count-preserving. This isolates the fix question: if this
/// passes but the full pipeline loses MC, the leak is in the SAT-probe /
/// aggressive-round interaction, not in preknown resolution itself.
#[test]
fn dve_preknown_only_preserves_mc() {
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

    let preknown: Vec<u32> = vec![2, 4, 9, 10, 11];
    let mut clauses = f.clauses.clone();
    let mut fates = vec![DveFate::Kept; 12];
    let orig_len = clauses.len();
    let _ = apply_elimination(
        &mut clauses,
        &preknown,
        &mut fates,
        orig_len,
        &Default::default(),
    );

    let appears = appearance_mask(&clauses, 12);
    let mut num_free = 0;
    for v in 0..12 {
        if !fates[v].eliminated() && !appears[v] {
            fates[v] = DveFate::Free;
            num_free += 1;
        }
    }

    let (reduced, _) = renumber_formula(&fates, 12, clauses);
    let mc = brute_force_mc(&reduced);
    let total = mc.clone() * BigUint::from(1u128 << num_free);
    assert_eq!(
        total,
        BigUint::from(64u32),
        "preknown-only elim should preserve MC: reduced_mc={} * 2^{} = {}",
        mc,
        num_free,
        total
    );
}

/// Diagnostic: preknown elimination followed by SAT-probe-detected
/// var elimination (split into two `apply_elimination` calls, no CaDiCaL
/// between). If this preserves MC, the two-phase DVE fix is sound in
/// isolation — any remaining bug comes from the pipeline (strengthening
/// or aggressive final round).
#[test]
fn dve_preknown_then_sat_defined_preserves_mc() {
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

    let preknown: Vec<u32> = vec![2, 4, 9, 10, 11];
    let mut clauses = f.clauses.clone();
    let mut fates = vec![DveFate::Kept; 12];
    let orig_len = clauses.len();
    let _ = apply_elimination(
        &mut clauses,
        &preknown,
        &mut fates,
        orig_len,
        &Default::default(),
    );

    // Now test the remaining SAT probe candidate (var 3 = 1-indexed 4).
    // Without preknown-pinning, re-probe to see if still deemed defined.
    let sat_candidates: Vec<u32> = vec![3];
    let defined = pick_def_vars(&clauses, 12, &sat_candidates, 10_000);
    if !defined.is_empty() {
        let orig_len2 = clauses.len();
        let _ = apply_elimination(
            &mut clauses,
            &defined,
            &mut fates,
            orig_len2,
            &Default::default(),
        );
    }

    let appears = appearance_mask(&clauses, 12);
    let mut num_free = 0;
    for v in 0..12 {
        if !fates[v].eliminated() && !appears[v] {
            fates[v] = DveFate::Free;
            num_free += 1;
        }
    }
    let (reduced, _) = renumber_formula(&fates, 12, clauses);
    let mc = brute_force_mc(&reduced);
    let total = mc.clone() * BigUint::from(1u128 << num_free);
    assert_eq!(
        total,
        BigUint::from(64u32),
        "two-phase preknown+sat elim should preserve MC: reduced_mc={} * 2^{} = {} (defined_set={:?})",
        mc,
        num_free,
        total,
        defined
    );
}

/// Regression: SAT-probe shares preknown vars between dual-CNF copies, so
/// candidates can appear "defined" only because preknown gate clauses are
/// still present to pin the shared state. If preknown and those spurious
/// candidates are eliminated together, the resulting BVE doesn't preserve
/// model count (CNF can't encode the projected multiplicities).
///
/// Fix: eliminate preknown first, then re-probe candidates on the reduced
/// formula. A candidate that was only defined "through preknown" will
/// correctly fail the second probe and remain in the formula.
///
/// Minimal scenario (from benchmark bug hunt): 12 vars, 19 clauses. Var 4
/// has the hidden pattern 4 ↔ 1 ∧ ¬2 (AND-with-negation, not syntactically
/// detected because binary clauses have negative inputs). Gates 3, 5, 10,
/// 11, 12 form a detectable DAG. Original MC = 64; combined elimination
/// before the fix produced 48 (factor 4/3 lost).
#[test]
fn dve_preknown_first_preserves_mc() {
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

    // Gate preknown (0-indexed): vars 3, 5, 10, 11, 12 one-indexed.
    let mut known: rustc_hash::FxHashSet<VarId> = rustc_hash::FxHashSet::default();
    for v in [2u32, 4, 9, 10, 11] {
        known.insert(VarId(v));
    }

    let result = preprocess_dve(
        &f,
        10,
        10_000,
        false,
        &known,
        &rustc_hash::FxHashSet::default(),
        FrozenEquiv::Ignore,
    );

    let reduced_mc = brute_force_mc(&result.formula);
    let total = reduced_mc.clone() * BigUint::from(1u128 << result.num_free());
    assert_eq!(
        total,
        BigUint::from(64u32),
        "MC not preserved: reduced_mc={} * 2^{} = {}, expected 64 (defined={}, equiv={}, free={}, reduced_vars={})",
        reduced_mc,
        result.num_free(),
        total,
        result.num_defined(),
        result.num_equiv(),
        result.num_free(),
        result.formula.num_vars,
    );
}

/// Regression: pure-literal elimination is sound for SAT but UNSOUND
/// for #SAT. When a var in `known_defined` has only one polarity
/// present in the residual clauses (either originally, or after
/// earlier in-batch resolutions strip the other), `elim_vars` must
/// NOT force it and drop its clauses — that undercounts models.
///
/// Direct scenario: φ = (V ∨ A). V is preknown-defined; V appears
/// only positive. Buggy code: pure-literal branch forces V=true,
/// drops (V ∨ A), eliminates V (counted as ×1 defined), leaves A
/// as a free var (×2). Total = 2. True count = 3 (the V=0, A=1
/// model is lost). Sound behavior: skip V entirely (one polarity
/// missing → resolution can't preserve its multiplicity).
#[test]
fn dve_pure_literal_on_defined_var_preserves_mc() {
    // 1-indexed: V=1, A=2. φ = (V ∨ A). V only positive.
    let f = make_formula(2, vec![vec![1, 2]]);
    // Models (V,A): (0,1), (1,0), (1,1) → count = 3.
    assert_eq!(brute_force_mc(&f), BigUint::from(3u32));

    let mut known: rustc_hash::FxHashSet<VarId> = rustc_hash::FxHashSet::default();
    known.insert(VarId(0)); // V

    let result = preprocess_dve(
        &f,
        10,
        10_000,
        false,
        &known,
        &rustc_hash::FxHashSet::default(),
        FrozenEquiv::Ignore,
    );
    let reduced_mc = brute_force_mc(&result.formula);
    let total = reduced_mc.clone() * BigUint::from(1u128 << result.num_free());
    assert_eq!(
        total,
        BigUint::from(3u32),
        "Pure-literal corrupted MC: reduced={} * 2^{} = {}, \
         expected 3 (defined={}, equiv={}, free={}, reduced_vars={})",
        reduced_mc,
        result.num_free(),
        total,
        result.num_defined(),
        result.num_equiv(),
        result.num_free(),
        result.formula.num_vars,
    );
}

#[test]
fn dve_equiv_followed_by_gate_elim_preserves_mc() {
    // 1-indexed: Y=1, X=2, A=3. OR gate Y = A ∨ X + extra (¬Y ∨ X).
    let f = make_formula(
        3,
        vec![
            vec![1, -3],    // Y ∨ ¬A       (A → Y)
            vec![1, -2],    // Y ∨ ¬X       (X → Y, gate)
            vec![-1, 3, 2], // ¬Y ∨ A ∨ X   (Y → A ∨ X, gate)
            vec![-1, 2],    // ¬Y ∨ X       (Y → X, creates Y ≡ X)
        ],
    );
    // Ground-truth: satisfying (Y,X,A) are (0,0,0), (1,1,0), (1,1,1).
    assert_eq!(brute_force_mc(&f), BigUint::from(3u32));

    // Preknown: Y is detected as a gate output on the original formula.
    let mut known: rustc_hash::FxHashSet<VarId> = rustc_hash::FxHashSet::default();
    known.insert(VarId(0));

    let result = preprocess_dve(
        &f,
        10,
        10_000,
        false,
        &known,
        &rustc_hash::FxHashSet::default(),
        FrozenEquiv::Ignore,
    );
    let reduced_mc = brute_force_mc(&result.formula);
    let total = reduced_mc.clone() * BigUint::from(1u128 << result.num_free());
    assert_eq!(
        total,
        BigUint::from(3u32),
        "MC not preserved: reduced_mc={} * 2^{} = {}, expected 3 (defined={}, equiv={}, free={}, reduced_vars={})",
        reduced_mc,
        result.num_free(),
        total,
        result.num_defined(),
        result.num_equiv(),
        result.num_free(),
        result.formula.num_vars,
    );
}
