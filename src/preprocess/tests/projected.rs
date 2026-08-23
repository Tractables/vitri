//! Projection-aware reduction: what it may eliminate, and when it stops.
//!
//! The projected and projected-weighted driver paths run the full pipeline
//! FROZEN on the show vars before `bve_project`. Every variable eliminated
//! there must be a HIDDEN one. The soundness cases below pin the projected
//! count through the strengthen pass against the brute-force oracle on the
//! ORIGINAL formula — across the cases that would be unsound without the
//! show-var protection, especially an equivalence merge picking a hidden
//! representative for a class that contains a show variable.

use crate::cnf::{Clause, CnfFormula, Literal, Reduced, ShowSet, VarId};
use crate::preprocess::projected::{strengthen_and_bve, strengthen_projected_hidden};
use crate::tests::common::{Lcg, make_formula};
use crate::tests::pmc_oracle::{brute_force_pmc, brute_force_pwmc};
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// A random 3-CNF at the ratio where most variables are neither definable nor
/// equivalent to another: the strengthening stage pays for a definability check
/// per candidate and gets nothing back, so it runs until its budget stops it.
fn rand3(num_vars: u32, num_clauses: usize, seed: u64) -> CnfFormula {
    let mut rng = Lcg::new(seed);
    let mut clauses = Vec::new();
    for _ in 0..num_clauses {
        let mut lits: Vec<Literal> = Vec::new();
        while lits.len() < 3 {
            let v = rng.below(num_vars as u64) as u32;
            if lits.iter().any(|l| l.var.0 == v) {
                continue;
            }
            lits.push(Literal::new(VarId(v), rng.below(2) == 1));
        }
        clauses.push(Clause::new(lits));
    }
    CnfFormula { num_vars, clauses }
}

/// The projected reduction stops at the run's deadline, not at its own ceiling.
///
/// This input spends the whole three-second ceiling when nothing else bounds it.
/// An already-expired deadline leaves nothing of it, so the strengthening stage
/// must not open a round and the call comes back in the time the two unbudgeted
/// passes around it take.
#[test]
fn expired_deadline_returns_before_the_ceiling() {
    let formula = rand3(200, 840, 7);
    let show = ShowSet::<Reduced>::from_zero_based(0..20);
    let started = Instant::now();
    let out = strengthen_and_bve(&formula, show, Some(Instant::now()));
    let elapsed = started.elapsed();
    assert_eq!(out.formula.num_vars, formula.num_vars);
    assert!(
        elapsed < Duration::from_millis(1_000),
        "the projected reduction ran {elapsed:?} past an expired deadline"
    );
}

/// `show_1based`: show ids as a file would write them.
fn assert_pmc_preserved(f: &CnfFormula, show_1based: &[u32]) {
    let show_set = ShowSet::<Reduced>::from_dimacs_ids(show_1based).expect("valid ids");
    let show_vec: Vec<u32> = show_set.as_zero_based().to_vec();
    let want = brute_force_pmc(f, &show_vec);
    let (strong, determined) = strengthen_projected_hidden(f, &show_set, 8, 5_000);
    // num_vars preserved so the oracle keys are aligned.
    assert_eq!(
        strong.num_vars, f.num_vars,
        "strengthen must preserve num_vars"
    );
    // Mirror the projected chain: show vars merged away as equivalent are
    // dropped from the show set (counted ×1 via their survivor). The oracle
    // must count over
    // the REDUCED show set, exactly as the projected chain does.
    let drop: HashSet<u32> = determined.iter().map(|f| f.eliminated.0).collect();
    let reduced_show: Vec<u32> = show_vec
        .iter()
        .copied()
        .filter(|v| !drop.contains(v))
        .collect();
    let got = brute_force_pmc(&strong, &reduced_show);
    assert_eq!(
        got,
        want,
        "projected count changed through strengthen pass\n  in:  {:?}\n  show:{:?}\n  out: {:?}",
        f.clauses
            .iter()
            .map(|c| c.literals.iter().map(|l| l.to_dimacs()).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        show_vec,
        strong
            .clauses
            .iter()
            .map(|c| c.literals.iter().map(|l| l.to_dimacs()).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
    );
}

/// Two equivalent SHOW vars: x0 ≡ x1 (both show). With ForceShowRep the merge
/// keeps a show var as the survivor and drops the other as *determined* (×1),
/// so the projected count stays = 2 (NOT the ×2=4 a naive free-marking would
/// give). This is the case `ForceShowRep` exists to enable.
#[test]
fn two_equivalent_show_vars_merge_preserves_count() {
    // x0 ≡ x1 : (x0 ∨ ¬x1) ∧ (¬x0 ∨ x1)
    let f = make_formula(2, vec![vec![1, -2], vec![-1, 2]]);
    assert_eq!(
        brute_force_pmc(&f, &[0, 1]),
        num_bigint::BigUint::from(2u32)
    );
    assert_pmc_preserved(&f, &[1, 2]); // show = {x0, x1}
}

/// WEIGHTED analogue: two equivalent SHOW vars with ASYMMETRIC, distinct
/// per-literal weights. The merge keeps a show survivor and the projected chain
/// folds the eliminated show var's weights into it; the projected-WEIGHTED count
/// must
/// be preserved exactly. This is the soundness guard for the weighted show-equiv
/// fold (`strengthen_projected_hidden` returning `(elim, survivor)`).
/// `w[v] = (w_neg, w_pos)` per 0-based var.
fn assert_pwmc_fold_preserved(f: &CnfFormula, show_1based: &[u32], w: &[(i64, i64)]) {
    use num_bigint::BigInt;
    use num_rational::BigRational;
    let rat = |x: i64| BigRational::from_integer(BigInt::from(x));
    let show_set = ShowSet::<Reduced>::from_dimacs_ids(show_1based).expect("valid ids");
    let show_vec: Vec<u32> = show_set.as_zero_based().to_vec();
    let orig_w = |v: u32, val: bool| {
        if val {
            rat(w[v as usize].1)
        } else {
            rat(w[v as usize].0)
        }
    };
    let want = brute_force_pwmc(f, &show_vec, orig_w);

    let (strong, folds) = strengthen_projected_hidden(f, &show_set, 8, 5_000);
    assert_eq!(strong.num_vars, f.num_vars);
    let mut wt: Vec<(BigRational, BigRational)> = (0..f.num_vars as usize)
        .map(|v| (rat(w[v].0), rat(w[v].1)))
        .collect();
    let mut drop: HashSet<u32> = HashSet::new();
    for f in &folds {
        let elim = f.eliminated.idx();
        let surv = f.survivor;
        let (en, ep) = (wt[elim].0.clone(), wt[elim].1.clone());
        if surv.positive {
            wt[surv.var.idx()].0 *= en;
            wt[surv.var.idx()].1 *= ep;
        } else {
            wt[surv.var.idx()].0 *= ep;
            wt[surv.var.idx()].1 *= en;
        }
        drop.insert(f.eliminated.0);
    }
    let reduced_show: Vec<u32> = show_vec
        .iter()
        .copied()
        .filter(|v| !drop.contains(v))
        .collect();
    let folded_w = |v: u32, val: bool| {
        if val {
            wt[v as usize].1.clone()
        } else {
            wt[v as usize].0.clone()
        }
    };
    let got = brute_force_pwmc(&strong, &reduced_show, folded_w);
    assert_eq!(
        got, want,
        "PWMC changed through weighted show-equiv fold (folds={folds:?})"
    );
}

#[test]
fn two_equivalent_show_vars_weighted_fold_preserves_pwmc() {
    // x0 ≡ x1, asymmetric weights x0=(neg3,pos2), x1=(neg7,pos5).
    // Feasible show: (T,T)=2·5=10, (F,F)=3·7=21 → 31.
    let f = make_formula(2, vec![vec![1, -2], vec![-1, 2]]);
    assert_pwmc_fold_preserved(&f, &[1, 2], &[(3, 2), (7, 5)]);
}

#[test]
fn anti_equivalent_show_vars_weighted_fold_preserves_pwmc() {
    // x0 ≡ ¬x1 : (x0 ∨ x1) ∧ (¬x0 ∨ ¬x1). Feasible show: (T,F)=2·7=14,
    // (F,T)=3·5=15 → 29. Exercises the polarity-swap branch of the fold.
    let f = make_formula(2, vec![vec![1, 2], vec![-1, -2]]);
    assert_pwmc_fold_preserved(&f, &[1, 2], &[(3, 2), (7, 5)]);
}

/// Backbone-forced show var, forced ONLY through hidden vars (the case the
/// fuzz originally caught). show = {x1}; x1 is forced TRUE (x1=F is UNSAT) but
/// only via hidden x0,x2,x3. DVE ∃-eliminates those hidden vars; resolution
/// derives the unit (x1). The bug: that unit was propagated away, erasing x1
/// from the residual → mis-counted free (×2 → got 2). The fix keeps the derived
/// unit on the frozen var as a clause, so x1 stays forced and pmc = 1.
#[test]
fn dve_frozen_unit_resolvent_preserves_pmc() {
    let f = make_formula(
        4,
        vec![
            vec![3, 2],
            vec![1, -2],
            vec![-1, -2, -4],
            vec![2, -4],
            vec![4, -3],
            vec![-3, 1],
        ],
    );
    assert_eq!(brute_force_pmc(&f, &[1]), num_bigint::BigUint::from(1u32)); // x1 forced T
    assert_pmc_preserved(&f, &[2]); // show = {x1}
}

/// show ≡ hidden: x0 (show) ≡ x1 (hidden). The hidden var MAY be eliminated
/// (∃-absorbed, ×1), but the projected count over {x0} must stay 2.
#[test]
fn show_equiv_hidden_preserves_count() {
    let f = make_formula(2, vec![vec![1, -2], vec![-1, 2]]);
    assert_pmc_preserved(&f, &[1]); // show = {x0}, project x1
}

/// Definable hidden var: x2 = x0 ∧ x1, x2 hidden. DVE eliminates x2 (defined,
/// ×1). Projected count over {x0, x1} unchanged.
#[test]
fn defined_hidden_var_eliminated_soundly() {
    // x2 ↔ (x0 ∧ x1): (¬x2 ∨ x0)(¬x2 ∨ x1)(x2 ∨ ¬x0 ∨ ¬x1), plus a use of x2.
    let f = make_formula(3, vec![vec![-3, 1], vec![-3, 2], vec![3, -1, -2], vec![3]]);
    assert_pmc_preserved(&f, &[1, 2]); // show = {x0, x1}, project x2
}

/// Hidden-only equivalence still merges (×1). x1 ≡ x2 both hidden; show {x0}.
#[test]
fn hidden_only_equiv_merges_soundly() {
    // x0 ∨ x1 ; x1 ≡ x2
    let f = make_formula(3, vec![vec![1, 2], vec![2, -3], vec![-2, 3]]);
    assert_pmc_preserved(&f, &[1]); // show = {x0}, project x1, x2
}

/// Differential fuzz: random small CNFs, random show subsets. A deterministic
/// xorshift keeps it reproducible without Math.random. Any divergence between
/// the strengthened projected count and the oracle is a soundness bug.
#[test]
fn fuzz_random_small_cnfs() {
    let mut state: u64 = 0x9E3779B97F4A7C15;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..20000 {
        let nv = 3 + (next() % 5) as u32;
        let nc = 2 + (next() % 8) as usize;
        let mut raw = Vec::new();
        for _ in 0..nc {
            let k = 1 + (next() % 3) as usize;
            let mut lits: Vec<i32> = Vec::new();
            let mut used_vars: Vec<i32> = Vec::new();
            for _ in 0..k {
                let v = 1 + (next() % nv as u64) as i32;
                if used_vars.contains(&v) {
                    continue;
                }
                used_vars.push(v);
                let s = if next() & 1 == 0 { 1 } else { -1 };
                lits.push(v * s);
            }
            if !lits.is_empty() {
                raw.push(lits);
            }
        }
        if raw.is_empty() {
            continue;
        }
        let f = make_formula(nv, raw);
        let mut show: Vec<u32> = (1..=nv).filter(|_| next() & 1 == 0).collect();
        if show.is_empty() {
            show.push(1);
        }
        assert_pmc_preserved(&f, &show);
    }
}

/// The two stages composed, which is how the projected chain runs them: hidden
/// variables are strengthened away and then existentially eliminated, and the
/// projected count over what is left is the one the original formula had. The
/// stages are individually sound above; this is the statement that running them
/// back to back is too.
#[test]
fn the_projected_chain_preserves_the_projected_count() {
    // Hidden 3 links the two show variables, hidden 4 ≡ hidden 5, and the last
    // clause keeps 4 from being merely redundant.
    let f = make_formula(
        5,
        vec![
            vec![1, 3],
            vec![-3, 2],
            vec![4, -5],
            vec![-4, 5],
            vec![1, 2, 4],
        ],
    );
    let show = ShowSet::<Reduced>::from_dimacs_ids(&[1, 2]).expect("valid ids");
    let want = brute_force_pmc(&f, show.as_zero_based());

    let reduced = strengthen_and_bve(&f, show.clone(), /*deadline=*/ None);

    assert!(
        reduced.folds.is_empty(),
        "no show variable is equivalent to another here, so none may be folded away",
    );
    assert_eq!(
        brute_force_pmc(&reduced.formula, reduced.show_set.as_zero_based()),
        want,
        "eliminating the hidden variables changed the projected count",
    );
}

/// A show variable proved equivalent to ANOTHER show variable is determined, so
/// the chain counts the survivor alone and drops the eliminated one from the
/// show set. Leaving it in would count the pair twice over one value.
#[test]
fn merging_two_show_variables_drops_the_eliminated_one_from_the_show_set() {
    // 1 ≡ 2, both shown; 3 is hidden and keeps the formula from collapsing.
    let f = make_formula(3, vec![vec![1, -2], vec![-1, 2], vec![1, 2, 3]]);
    let show = ShowSet::<Reduced>::from_dimacs_ids(&[1, 2]).expect("valid ids");
    let want = brute_force_pmc(&f, show.as_zero_based());

    let reduced = strengthen_and_bve(&f, show.clone(), /*deadline=*/ None);

    assert_eq!(
        reduced.folds.len(),
        1,
        "one of the two equivalent show variables must be merged away",
    );
    let fold = &reduced.folds[0];
    assert!(
        !reduced.show_set.contains(fold.eliminated),
        "the merged-away show variable must leave the show set",
    );
    assert!(
        reduced.show_set.contains(fold.survivor.var),
        "the survivor must stay in the show set — it is what carries the value",
    );
    assert_eq!(
        brute_force_pmc(&reduced.formula, reduced.show_set.as_zero_based()),
        want,
        "counting the survivor alone must give the projected count of the pair",
    );
}
