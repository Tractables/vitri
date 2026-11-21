use std::time::{Duration, Instant};

use rand::rngs::SmallRng;
use rand::{RngExt, SeedableRng};

use crate::cnf::{Clause, CnfFormula, Literal};
use crate::config::PreprocessClock;
use crate::preprocess::implicates::Allowance;
use crate::preprocess::models::{ModelMatrix, Repair, RepairSession, Repairs, draw, wall_stop};
use crate::tests::pmc_oracle::satisfies;

/// The repairs of `pairs` in one session over `formula` and its `models`.
fn repairs(
    formula: &CnfFormula,
    models: &ModelMatrix,
    pairs: &[(usize, u32)],
    conflicts: i32,
    seed: u64,
    allowance: &Allowance,
) -> Repairs {
    RepairSession::new(formula, models, conflicts, seed)
        .expect("a solver")
        .run(pairs, allowance)
}

fn cnf(num_vars: u32, clauses: &[Vec<i32>]) -> CnfFormula {
    CnfFormula::new(
        num_vars,
        clauses
            .iter()
            .map(|c| Clause::new(c.iter().map(|&l| Literal::from(l)).collect()))
            .collect(),
    )
    .expect("valid clauses")
}

/// A random 3-CNF over `n` variables with `m` clauses of distinct variables.
fn random_3cnf(n: u32, m: usize, seed: u64) -> CnfFormula {
    let mut rng = SmallRng::seed_from_u64(seed);
    let clauses: Vec<Vec<i32>> = (0..m)
        .map(|_| {
            let mut c: Vec<i32> = Vec::new();
            while c.len() < 3 {
                let v = rng.random_range(1..=n as i32);
                if c.iter().all(|l| l.abs() != v) {
                    c.push(if rng.random::<bool>() { v } else { -v });
                }
            }
            c
        })
        .collect();
    cnf(n, &clauses)
}

fn models_of(formula: &CnfFormula) -> Vec<u64> {
    (0u64..1 << formula.num_vars())
        .filter(|&a| satisfies(formula, a))
        .collect()
}

/// Satisfiable random 3-CNFs near the threshold, so that repairs need work.
fn cases() -> Vec<CnfFormula> {
    (1..60)
        .map(|seed| random_3cnf(8 + (seed % 7) as u32, 30 + (seed % 20) as usize, seed))
        .filter(|f| !models_of(f).is_empty())
        .collect()
}

fn unbounded() -> Allowance {
    let nominal = Duration::from_secs(600);
    Allowance::of_run(PreprocessClock::WallClock, nominal, nominal, None)
}

fn charged(ms: u64) -> Allowance {
    let clock = PreprocessClock::Deterministic {
        configured_wall_ms: None,
    };
    let nominal = Duration::from_millis(ms);
    Allowance::of_run(clock, nominal, nominal, None)
}

fn word(models: &ModelMatrix, row: usize) -> u64 {
    models.row(row)[0]
}

#[test]
fn every_drawn_model_satisfies_the_formula() {
    for f in cases() {
        let d = draw(&f, 40, 0, 7, &unbounded());
        assert_eq!(d.models.len(), 40);
        assert!(!d.ran_out && !d.unsatisfiable);
        for r in 0..d.models.len() {
            assert!(satisfies(&f, word(&d.models, r)));
        }
    }
}

#[test]
fn random_phases_draw_more_than_one_model_of_a_loose_formula() {
    let f = cnf(12, &[vec![1, 2], vec![-3, 4, 5]]);
    let d = draw(&f, 30, 0, 1, &unbounded());
    let mut seen: Vec<u64> = (0..d.models.len()).map(|r| word(&d.models, r)).collect();
    seen.sort_unstable();
    seen.dedup();
    assert!(seen.len() > 20, "{} distinct of 30", seen.len());
}

#[test]
fn a_draw_repeats_for_the_same_seed_and_differs_for_another() {
    let f = cnf(16, &[vec![1, 2, 3], vec![-1, 4]]);
    let a = draw(&f, 20, 0, 5, &unbounded()).models;
    assert_eq!(a, draw(&f, 20, 0, 5, &unbounded()).models);
    assert_ne!(a, draw(&f, 20, 0, 6, &unbounded()).models);
}

#[test]
fn a_draw_of_an_unsatisfiable_formula_is_empty_and_says_so() {
    let f = cnf(2, &[vec![1], vec![-1, 2], vec![-2]]);
    let d = draw(&f, 10, 0, 1, &unbounded());
    assert!(d.models.is_empty() && d.unsatisfiable && !d.ran_out);
}

#[test]
fn a_draw_projected_past_its_allowance_ends_early() {
    let f = random_3cnf(150, 570, 5);
    let full = draw(&f, 200, 0, 1, &charged(600_000));
    assert!(full.models.len() == 200 && !full.ran_out);
    let allowance = Allowance {
        nominal: full.spent * 4,
        ..charged(0)
    };
    let fits = draw(&f, 200, 0, 1, &allowance);
    assert!(fits.models.len() == 200 && !fits.ran_out);
    let d = draw(&f, 200, 2000, 1, &allowance);
    assert!(d.ran_out && d.models.len() < 200);
    assert!(d.spent < full.spent, "{:?} of {:?}", d.spent, full.spent);
}

#[test]
fn a_spent_allowance_ends_the_draw_with_what_it_has() {
    let f = random_3cnf(40, 120, 3);
    let d = draw(&f, 1000, 0, 1, &charged(0));
    assert!(d.models.is_empty() && d.ran_out);
}

#[test]
fn a_draw_under_charged_work_ends_at_the_deadline() {
    let f = random_3cnf(40, 120, 3);
    let passed = Allowance {
        deadline: Some(Instant::now()),
        ..charged(60_000)
    };
    let d = draw(&f, 1000, 0, 1, &passed);
    assert!(d.models.is_empty() && d.ran_out);
}

#[test]
fn the_wall_stop_is_the_allowance_or_a_runaway_wall_under_charged_work() {
    let t = Instant::now();
    let nominal = Duration::from_secs(1);
    let wall = Allowance::of_run(PreprocessClock::WallClock, nominal, nominal, None);
    assert_eq!(wall_stop(&wall, t), Some(t + nominal));
    let runaway = wall_stop(&charged(1000), t).expect("a wall stop");
    assert!(runaway > t + nominal);
    let sooner = Allowance {
        deadline: Some(t + nominal / 2),
        ..charged(1000)
    };
    assert_eq!(wall_stop(&sooner, t), Some(t + nominal / 2));
}

/// Whether some model differs from `m` in `u` and otherwise only inside `within`.
fn flip_within(models: &[u64], m: u64, u: usize, within: u64) -> bool {
    models
        .iter()
        .any(|&x| (x ^ m) >> u & 1 == 1 && (x ^ m) & !(within | 1 << u) == 0)
}

#[test]
fn every_repair_is_inclusion_minimal_and_every_backbone_has_no_repair() {
    let (mut minimal, mut backbone) = (0, 0);
    for (i, f) in cases().into_iter().enumerate() {
        let all = models_of(&f);
        let n = f.num_vars() as usize;
        let mut models = ModelMatrix::new(n);
        let rows: Vec<u64> = all.iter().copied().step_by(all.len().div_ceil(5)).collect();
        for &m in &rows {
            models.push((0..n).map(|v| m >> v & 1 == 1));
        }
        let pairs: Vec<(usize, u32)> = (0..rows.len())
            .flat_map(|r| (0..n as u32).map(move |u| (r, u)))
            .collect();
        let got = repairs(&f, &models, &pairs, 10_000, i as u64, &unbounded());
        assert!(!got.ran_out);
        assert_eq!(got.found.len(), pairs.len());
        for (&(r, u), repair) in pairs.iter().zip(&got.found) {
            let (m, u) = (rows[r], u as usize);
            match repair {
                Repair::Backbone => {
                    assert!(!flip_within(&all, m, u, !0));
                    backbone += 1;
                }
                Repair::Minimal(d) => {
                    let set: u64 = d.iter().map(|&v| 1u64 << v).sum();
                    assert!(set >> u & 1 == 0);
                    assert!(satisfies(&f, m ^ set ^ 1 << u));
                    for &v in d {
                        assert!(!flip_within(&all, m, u, set & !(1 << v)));
                    }
                    minimal += 1;
                }
                other => panic!("{other:?} with no conflict cap reached"),
            }
        }
    }
    assert!(
        minimal > 1000 && backbone > 0,
        "{minimal} minimal, {backbone} backbone"
    );
}

#[test]
fn a_session_given_the_pairs_in_parts_repairs_them_as_one_call_does() {
    let f = random_3cnf(40, 150, 5);
    let d = draw(&f, 8, 0, 3, &unbounded());
    let pairs: Vec<(usize, u32)> = (0..d.models.len())
        .flat_map(|r| (0..40u32).step_by(3).map(move |u| (r, u)))
        .collect();
    let whole = repairs(&f, &d.models, &pairs, 1000, 7, &unbounded());
    let mut session = RepairSession::new(&f, &d.models, 1000, 7).expect("solver");
    let cut = pairs.len() / 3;
    let mut parts = session.run(&pairs[..cut], &unbounded()).found;
    parts.extend(session.run(&pairs[cut..], &unbounded()).found);
    assert_eq!(parts, whole.found);
}

#[test]
fn a_variable_free_in_the_model_is_repaired_by_flipping_it_alone() {
    let f = cnf(3, &[vec![1, 2]]);
    let mut models = ModelMatrix::new(3);
    models.push([true, true, false]);
    let got = repairs(&f, &models, &[(0, 2), (0, 0)], 1000, 1, &unbounded());
    assert_eq!(
        got.found,
        vec![Repair::Minimal(vec![]), Repair::Minimal(vec![])]
    );
}

#[test]
fn a_spent_allowance_ends_the_repairs_with_what_they_have() {
    let f = random_3cnf(30, 100, 9);
    let d = draw(&f, 2, 0, 1, &unbounded());
    let pairs: Vec<(usize, u32)> = (0..30).map(|u| (0, u)).collect();
    let got = repairs(&f, &d.models, &pairs, 1000, 1, &charged(0));
    assert!(got.found.is_empty() && got.ran_out);
}
