//! The canonical form behind the per-build vtree cache: renamed copies agree,
//! and formulas that are not renamed copies do not.

use rand::rngs::SmallRng;
use rand::seq::SliceRandom;
use rand::{Rng, RngExt, SeedableRng};

use crate::cnf::{Clause, CnfFormula, Literal, ShowSet};
use crate::component::canonical::canonical_form;
use crate::vtree::VarId;

type RawClause = Vec<(usize, bool)>;

fn formula(num_vars: usize, clauses: &[RawClause]) -> CnfFormula {
    let clauses = clauses
        .iter()
        .map(|c| {
            Clause::new(
                c.iter()
                    .map(|&(v, p)| Literal::new(VarId::from_idx(v), p))
                    .collect(),
            )
        })
        .collect();
    CnfFormula::new(num_vars as u32, clauses).expect("the fixture stays inside its variable space")
}

fn random_clauses(rng: &mut impl Rng, num_vars: usize, count: usize) -> Vec<RawClause> {
    (0..count)
        .map(|_| {
            let len = rng.random_range(2..=3);
            let mut vars: Vec<usize> = (0..num_vars).collect();
            vars.shuffle(rng);
            vars.truncate(len);
            vars.into_iter()
                .map(|v| (v, rng.random_bool(0.5)))
                .collect()
        })
        .collect()
}

/// The same formula with its variables renamed by a random permutation and its
/// clauses and literals shuffled.
fn renamed_copy(
    rng: &mut impl Rng,
    num_vars: usize,
    clauses: &[RawClause],
) -> (Vec<usize>, Vec<RawClause>) {
    let mut perm: Vec<usize> = (0..num_vars).collect();
    perm.shuffle(rng);
    let mut copy: Vec<RawClause> = clauses
        .iter()
        .map(|c| {
            let mut c: RawClause = c.iter().map(|&(v, p)| (perm[v], p)).collect();
            c.shuffle(rng);
            c
        })
        .collect();
    copy.shuffle(rng);
    (perm, copy)
}

#[test]
fn a_renamed_and_reordered_copy_has_the_same_key() {
    for seed in 0..40 {
        let mut rng = SmallRng::seed_from_u64(seed);
        let n = 12;
        let clauses = random_clauses(&mut rng, n, 20);
        let (_, copy) = renamed_copy(&mut rng, n, &clauses);
        let a = canonical_form(&formula(n, &clauses), None);
        let b = canonical_form(&formula(n, &copy), None);
        assert_eq!(
            a.key, b.key,
            "seed {seed}: a renamed copy must share the key"
        );
    }
}

#[test]
fn a_renamed_copy_with_the_renamed_show_set_has_the_same_key() {
    let mut rng = SmallRng::seed_from_u64(7);
    let n = 12;
    let clauses = random_clauses(&mut rng, n, 20);
    let (perm, copy) = renamed_copy(&mut rng, n, &clauses);
    let shown = [0usize, 3, 4];
    let mask = |vars: &[usize]| {
        ShowSet::<crate::cnf::Local>::from_vars(vars.iter().map(|&v| VarId::from_idx(v)))
            .mask(n as u32)
    };
    let a = canonical_form(&formula(n, &clauses), Some(&mask(&shown)));
    let renamed: Vec<usize> = shown.iter().map(|&v| perm[v]).collect();
    let b = canonical_form(&formula(n, &copy), Some(&mask(&renamed)));
    assert_eq!(a.key, b.key);
    let other = canonical_form(&formula(n, &copy), Some(&mask(&[renamed[0], renamed[1]])));
    assert_ne!(a.key, other.key, "a different show set is a different key");
}

#[test]
fn a_flipped_literal_that_breaks_isomorphism_changes_the_key() {
    for seed in 0..40 {
        let mut rng = SmallRng::seed_from_u64(seed);
        let n = 12;
        let clauses = random_clauses(&mut rng, n, 20);
        let mut flipped = clauses.clone();
        // Turning a negative literal positive changes the number of positive
        // literals, which no renaming can do.
        let Some(lit) = flipped.iter_mut().flatten().find(|l| !l.1) else {
            continue;
        };
        lit.1 = true;
        let (_, copy) = renamed_copy(&mut rng, n, &flipped);
        let a = canonical_form(&formula(n, &clauses), None);
        let b = canonical_form(&formula(n, &copy), None);
        assert_ne!(a.key, b.key, "seed {seed}: the copies are not isomorphic");
    }
}

#[test]
fn the_renaming_is_a_permutation_that_maps_back() {
    let mut rng = SmallRng::seed_from_u64(3);
    let clauses = random_clauses(&mut rng, 15, 25);
    let form = canonical_form(&formula(15, &clauses), None);
    let back = form.canonical_to_local();
    for v in 0..15 {
        let local = VarId::from_idx(v);
        assert_eq!(back[form.to_canonical(local).idx()], local);
    }
}

#[test]
fn symmetric_variables_do_not_stop_copies_from_agreeing() {
    // Four interchangeable variables around one hub: refinement alone cannot
    // order them, individualisation must.
    let star: Vec<RawClause> = (1..5).map(|v| vec![(0, true), (v, false)]).collect();
    let mut rng = SmallRng::seed_from_u64(11);
    let (_, copy) = renamed_copy(&mut rng, 5, &star);
    assert_eq!(
        canonical_form(&formula(5, &star), None).key,
        canonical_form(&formula(5, &copy), None).key,
    );
}
