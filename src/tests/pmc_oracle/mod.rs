//! Brute-force reference oracles for projected / weighted / algebraic model
//! counting — the differential soundness gate for the projected, weighted and
//! algebraic production paths.
//!
//! These enumerate all `2^n` assignments, so they are valid only for small
//! `n` (≤ ~22). They define GROUND TRUTH: the compile-path counts must match
//! these exactly on small CNFs.
//!
//! ## Exact semantics (the definitions the production paths must satisfy)
//!
//! Let `F` be a CNF over variables `0..n`, `S ⊆ vars` the SHOW set (the kept /
//! counted vars; `c p show`), `P = vars \ S` the PROJECTED-OUT set.
//!
//! - **MC** = `|{ σ ∈ {0,1}^n : σ ⊨ F }|`. (`S = vars`, no projection.)
//! - **PMC** = `|{ τ ∈ {0,1}^S : ∃ ρ ∈ {0,1}^P, (τ,ρ) ⊨ F }|` — the number of
//!   DISTINCT show-assignments that extend to ≥1 model. Projected vars are
//!   counted as mere satisfiability, never enumerated. (`brute_force_pmc`.)
//! - **WMC** (literal weights `w(v,·)`, every var weighted, `S = vars`) =
//!   `Σ_{σ ⊨ F}  ∏_v w(v, σ(v))`.
//! - **PWMC** (show vars weighted, projected vars unweighted) =
//!   `Σ_{τ ∈ {0,1}^S : τ extendable}  ∏_{v∈S} w(v, τ(v))` — each extendable
//!   show-assignment counted ONCE, weighted by its show-var weights; projected
//!   vars contribute existence (0/1), not weight.
//! - **AMC** = PWMC with weights in a field (ℂ for Track 5B): same sum, complex
//!   `+`/`×`.
//!
//! The two weighted oracles (`brute_force_wmc`, `brute_force_pwmc`) are exact
//! over `num_rational::BigRational`. AMC (complex field, Track 5B) remains
//! specified-but-unimplemented: it needs a complex-rational type.

use crate::cnf::CnfFormula;
use num_bigint::BigUint;
use num_rational::BigRational;
use std::collections::HashSet;

/// True iff assignment `a` (bit `i` = value of var `i`) satisfies every clause.
fn satisfies(formula: &CnfFormula, a: u64) -> bool {
    formula.clauses.iter().all(|clause| {
        clause.literals.iter().any(|lit| {
            let val = (a >> lit.var.0) & 1 == 1;
            val == lit.positive
        })
    })
}

/// Brute-force model count. Valid for `num_vars ≤ 63`; intended for ≤ ~22.
pub(crate) fn brute_force_mc(formula: &CnfFormula) -> BigUint {
    let n = formula.num_vars;
    assert!(
        n <= 22,
        "brute_force_mc is exponential; keep test CNFs tiny"
    );
    let mut count = BigUint::ZERO;
    let one = BigUint::from(1u32);
    for a in 0u64..(1u64 << n) {
        if satisfies(formula, a) {
            count += &one;
        }
    }
    count
}

/// Brute-force projected model count over the SHOW set `show` (a slice of raw
/// var ids). Counts DISTINCT show-projections that extend to a model.
pub(crate) fn brute_force_pmc(formula: &CnfFormula, show: &[u32]) -> BigUint {
    let n = formula.num_vars;
    assert!(
        n <= 22,
        "brute_force_pmc is exponential; keep test CNFs tiny"
    );
    assert!(show.len() <= 63);
    assert!(
        show.iter().all(|&v| v < n),
        "show var out of range for this formula"
    );
    let mut seen: HashSet<u64> = HashSet::new();
    for a in 0u64..(1u64 << n) {
        if satisfies(formula, a) {
            let mut key = 0u64;
            for (i, &v) in show.iter().enumerate() {
                if (a >> v) & 1 == 1 {
                    key |= 1u64 << i;
                }
            }
            seen.insert(key);
        }
    }
    BigUint::from(seen.len())
}

/// Brute-force PROJECTED WEIGHTED model count (`pwmc`). Each DISTINCT show-
/// assignment that extends to ≥1 model is counted ONCE, weighted by the product
/// of its show-var literal weights; projected vars contribute existence only.
///
/// `weight(v, val)` returns the exact `BigRational` weight of show var `v` taking
/// boolean value `val`. Mirrors the production semantics the compile-path fold
/// (`RationalSemiring` + projected `/2^k` correction + free-show factor) must
/// match exactly on small CNFs. With `weight ≡ 1` this equals `brute_force_pmc`.
pub(crate) fn brute_force_pwmc(
    formula: &CnfFormula,
    show: &[u32],
    weight: impl Fn(u32, bool) -> BigRational,
) -> BigRational {
    let n = formula.num_vars;
    assert!(
        n <= 22,
        "brute_force_pwmc is exponential; keep test CNFs tiny"
    );
    assert!(show.len() <= 63);
    assert!(
        show.iter().all(|&v| v < n),
        "show var out of range for this formula"
    );
    let mut seen: HashSet<u64> = HashSet::new();
    for a in 0u64..(1u64 << n) {
        if satisfies(formula, a) {
            let mut key = 0u64;
            for (i, &v) in show.iter().enumerate() {
                if (a >> v) & 1 == 1 {
                    key |= 1u64 << i;
                }
            }
            seen.insert(key);
        }
    }
    let mut total = BigRational::from_integer(0.into());
    for key in seen {
        let mut prod = BigRational::from_integer(1.into());
        for (i, &v) in show.iter().enumerate() {
            let val = (key >> i) & 1 == 1;
            prod *= weight(v, val);
        }
        total += prod;
    }
    total
}

/// Brute-force WEIGHTED model count: each model contributes the product of its
/// literal weights over EVERY variable, so — unlike [`brute_force_pwmc`] —
/// nothing is projected away and two models agreeing on a subset still count
/// twice.
///
/// `weight(v, val)` returns the exact `BigRational` weight of var `v` taking
/// boolean value `val`. With `weight` constant 1 this equals [`brute_force_mc`].
pub(crate) fn brute_force_wmc(
    formula: &CnfFormula,
    weight: impl Fn(u32, bool) -> BigRational,
) -> BigRational {
    let n = formula.num_vars;
    assert!(
        n <= 22,
        "brute_force_wmc is exponential; keep test CNFs tiny"
    );
    let mut total = BigRational::from_integer(0.into());
    for a in 0u64..(1u64 << n) {
        if satisfies(formula, a) {
            let mut prod = BigRational::from_integer(1.into());
            for v in 0..n {
                prod *= weight(v, (a >> v) & 1 == 1);
            }
            total += prod;
        }
    }
    total
}

mod soundness;
