use super::*;
use crate::tests::common::{make_formula, rat};
use num_bigint::BigUint;
use num_rational::BigRational;

#[test]
fn mc_empty_formula_is_2_pow_n() {
    // No clauses over 3 vars → every assignment satisfies → 2^3 = 8.
    let f = make_formula(3, vec![]);
    assert_eq!(brute_force_mc(&f), BigUint::from(8u32));
}

#[test]
fn mc_single_unit() {
    // (x0) over 1 var → exactly 1 model.
    let f = make_formula(1, vec![vec![1]]);
    assert_eq!(brute_force_mc(&f), BigUint::from(1u32));
}

#[test]
fn pmc_full_show_equals_mc() {
    // With show = all vars, PMC == MC (no projection collapses anything).
    let f = make_formula(3, vec![vec![1, -2], vec![3]]);
    let mc = brute_force_mc(&f);
    let pmc = brute_force_pmc(&f, &[0, 1, 2]);
    assert_eq!(pmc, mc);
}

/// THE adversarial case (spec §5.2): `x ⊕ y`, show = {x}, project y.
/// True PMC = 2 (both x=0 and x=1 extend to a model). The unsound `/2^k`
/// shortcut would give MC/2^1 = 2/2 = 1 — WRONG. This pins that the
/// projected count is a real ∃ (OR of cofactors), not a flat power-of-two
/// division, exactly where saturating-marginalization must match ∃.
#[test]
fn pmc_xor_diverges_from_pow2_division() {
    // x ⊕ y  ≡  (x ∨ y) ∧ (¬x ∨ ¬y)
    let xor = make_formula(2, vec![vec![1, 2], vec![-1, -2]]);
    let mc = brute_force_mc(&xor);
    assert_eq!(mc, BigUint::from(2u32)); // models: (0,1) and (1,0)

    let pmc = brute_force_pmc(&xor, &[0]);
    assert_eq!(pmc, BigUint::from(2u32)); // both x=0 and x=1 are extendable
    // /2^k would claim mc/2 = 1 — the divergence the real-∃ path must avoid.
    assert_ne!(pmc, &mc / BigUint::from(2u32));
}

/// Contrast: when the projected var is genuinely FREE, `/2^k` happens to
/// agree with real ∃ — the only regime where the leaf-projection shortcut
/// is sound.
#[test]
fn pmc_free_var_matches_pow2_division() {
    // F = (x0); var 1 is free. show = {x0}, project x1.
    let f = make_formula(2, vec![vec![1]]);
    let mc = brute_force_mc(&f); // (1,0),(1,1) → 2
    assert_eq!(mc, BigUint::from(2u32));
    let pmc = brute_force_pmc(&f, &[0]); // x0 ∈ {1} → 1
    assert_eq!(pmc, BigUint::from(1u32));
    assert_eq!(pmc, &mc / BigUint::from(2u32)); // free var: /2^1 agrees
}

/// 3-var hand-checked PMC. F = (x0 ∨ x1) ∧ (x2). show = {x0, x1}.
/// Models: x2=1 and (x0∨x1). Show-pairs (x0,x1) ∈ {01,10,11} → PMC = 3.
#[test]
fn pmc_three_var_handchecked() {
    let f = make_formula(3, vec![vec![1, 2], vec![3]]);
    assert_eq!(brute_force_pmc(&f, &[0, 1]), BigUint::from(3u32));
    // Project x1 too (show = {x0}): x0 ∈ {0,1} both extendable → 2.
    assert_eq!(brute_force_pmc(&f, &[0]), BigUint::from(2u32));
}

/// FORCED-GATE case: a projected var shares a clause with a SHOW var, so it
/// cannot be projected independently up front — it "settles late", and the
/// count of extending projected-assignments differs per show-assignment.
/// This pins that PMC is a real ∃ over the projected vars (an OR of
/// cofactors evaluated in the right order), NOT a per-leaf `/2^k` factor.
///
/// F = (x0 ∨ x1) ∧ (x0 ∨ x2). Models (x0,x1,x2):
///   (1,0,0) (1,1,0) (1,0,1) (1,1,1) (0,1,1) → MC = 5.
/// The number of projected (x1,x2) completions is asymmetric: x0=1 extends
/// via 4 settings, x0=0 via exactly 1 — so no uniform power-of-two divides.
#[test]
fn pmc_forced_gate_projected_var_settles_late() {
    let f = make_formula(3, vec![vec![1, 2], vec![1, 3]]);
    // MC sanity: 5 models (asymmetric projected fan-out).
    assert_eq!(brute_force_mc(&f), BigUint::from(5u32));
    // show = {x0}, project {x1, x2}: both x0=0 and x0=1 are extendable → 2.
    // A `/2^k` shortcut would give 5/4 — not even an integer, so the gate
    // is provably a real ∃, not a division.
    assert_eq!(brute_force_pmc(&f, &[0]), BigUint::from(2u32));
    // show = {x0, x1}, project {x2}: show-pairs {(1,0),(1,1),(0,1)} → 3.
    assert_eq!(brute_force_pmc(&f, &[0, 1]), BigUint::from(3u32));
}

/// MULTI-COMPONENT INDEPENDENCE (the over-merge hazard): for two
/// var-DISJOINT components the
/// joint PMC must equal the PRODUCT of the per-component PMCs. An engine
/// that wrongly merges/threads the components across the gate would
/// double-count or collapse — this case is the differential tripwire.
///
/// F = (x0 ∨ x1) ∧ (x2 ∨ x3), two disjoint components. show = {x0,x1,x2}
/// (project x3). Component A = (x0∨x1), show {x0,x1} → PMC 3. Component
/// B = (x2∨x3), show {x2}, project x3 → PMC 2. Joint must be 3·2 = 6.
#[test]
fn pmc_two_disjoint_components_multiply() {
    let joint = make_formula(4, vec![vec![1, 2], vec![3, 4]]);
    let pmc_joint = brute_force_pmc(&joint, &[0, 1, 2]);
    assert_eq!(pmc_joint, BigUint::from(6u32)); // hand-checked: 3 × 2

    // The two components in isolation, with local var indices.
    let comp_a = make_formula(2, vec![vec![1, 2]]);
    let pmc_a = brute_force_pmc(&comp_a, &[0, 1]); // full show → 3
    let comp_b = make_formula(2, vec![vec![1, 2]]);
    let pmc_b = brute_force_pmc(&comp_b, &[0]); // project 2nd var → 2
    assert_eq!(pmc_a, BigUint::from(3u32));
    assert_eq!(pmc_b, BigUint::from(2u32));
    // The independence invariant the production gate must preserve.
    assert_eq!(pmc_joint, pmc_a * pmc_b);
}

/// PWMC with EVERY show literal weighted 1 must equal PMC (as a rational):
/// the weighted fold degenerates to a plain count of distinct feasible
/// projections. Pins the production weights=1 ↔ PMC identity at the oracle.
#[test]
fn pwmc_uniform_weight_one_equals_pmc() {
    let f = make_formula(3, vec![vec![1, 2], vec![3]]);
    let pmc = brute_force_pmc(&f, &[0, 1]); // 3 distinct feasible projections
    let pwmc = brute_force_pwmc(&f, &[0, 1], |_v, _val| rat(1, 1));
    assert_eq!(
        pwmc,
        BigRational::from_integer(num_bigint::BigInt::from(pmc))
    );
    assert_eq!(pwmc, BigRational::from_integer(3.into()));
}

/// Hand-checked weighted sum. F = (x0 ∨ x1), show = {x0, x1}, project nothing.
/// Feasible projections: (1,0),(0,1),(1,1). Weights w(v,true)=2, w(v,false)=3.
///   (1,0) → 2·3 = 6   (0,1) → 3·2 = 6   (1,1) → 2·2 = 4   ⇒ total 16.
#[test]
fn pwmc_handchecked_distinct_weights() {
    let f = make_formula(2, vec![vec![1, 2]]);
    let w = |_v: u32, val: bool| if val { rat(2, 1) } else { rat(3, 1) };
    let pwmc = brute_force_pwmc(&f, &[0, 1], w);
    assert_eq!(pwmc, BigRational::from_integer(16.into()));
}

/// Projected feasibility is counted ONCE per show-assignment, never per
/// extending model: x ⊕ y, show = {x}, project y. Both x=0 and x=1 are
/// feasible (one model each). With w(x,true)=1/2, w(x,false)=1/3 the result
/// is 1/2 + 1/3 = 5/6 — NOT scaled by the number of projected completions.
#[test]
fn pwmc_xor_dedupes_before_weighting() {
    let xor = make_formula(2, vec![vec![1, 2], vec![-1, -2]]);
    let w = |_v: u32, val: bool| if val { rat(1, 2) } else { rat(1, 3) };
    let pwmc = brute_force_pwmc(&xor, &[0], w);
    assert_eq!(pwmc, rat(5, 6));
}

/// Disjoint-component product holds in the weighted ring too: PWMC factorises
/// over var-disjoint components. F = (x0∨x1) ∧ (x2∨x3), show = {x0,x1,x2}.
/// Comp A weighted PWMC over {x0,x1} times comp B over {x2} (project x3).
#[test]
fn pwmc_disjoint_components_multiply() {
    let joint = make_formula(4, vec![vec![1, 2], vec![3, 4]]);
    let w = |_v: u32, val: bool| if val { rat(2, 1) } else { rat(3, 1) };
    let pwmc_joint = brute_force_pwmc(&joint, &[0, 1, 2], w);

    let comp_a = make_formula(2, vec![vec![1, 2]]);
    let pwmc_a = brute_force_pwmc(&comp_a, &[0, 1], w); // 16 (as above)
    // Comp B = (x2∨x3) with local idx (x0∨x1), show {x0}, project x1.
    // Feasible x0 ∈ {0,1} both → 2 (true) + 3 (false) = 5.
    let comp_b = make_formula(2, vec![vec![1, 2]]);
    let pwmc_b = brute_force_pwmc(&comp_b, &[0], w);
    assert_eq!(pwmc_b, BigRational::from_integer(5.into()));
    assert_eq!(pwmc_joint, pwmc_a * pwmc_b);
}
