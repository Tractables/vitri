use std::collections::HashMap;

use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use super::*;

/// The reading straight from its definition: the full k-by-k matrix, each
/// entry by evaluating the CNF on the joined assignment.
fn brute_force(
    n: usize,
    clauses: &[Vec<i32>],
    sample: &[Vec<bool>],
    exact: bool,
    inside: &[bool],
) -> f64 {
    let k = sample.len();
    let crossing = clauses.iter().any(|c| {
        let n_in = c
            .iter()
            .filter(|l| inside[l.unsigned_abs() as usize - 1])
            .count();
        n_in > 0 && n_in < c.len()
    });
    if k == 0 || clauses.is_empty() || !crossing {
        return 0.0;
    }
    let sat = |m: &[bool]| {
        clauses.iter().all(|c| {
            c.iter()
                .any(|&l| m[l.unsigned_abs() as usize - 1] == (l > 0))
        })
    };
    let joined = |i: usize, j: usize| -> Vec<bool> {
        (0..n)
            .map(|v| {
                if inside[v] {
                    sample[i][v]
                } else {
                    sample[j][v]
                }
            })
            .collect()
    };
    let m: Vec<Vec<bool>> = (0..k)
        .map(|i| (0..k).map(|j| sat(&joined(i, j))).collect())
        .collect();
    let classes = |row: &dyn Fn(usize) -> Vec<bool>| {
        let mut w: HashMap<Vec<bool>, usize> = HashMap::new();
        for i in 0..k {
            *w.entry(row(i)).or_default() += 1;
        }
        let f1 = w.values().filter(|&&c| c == 1).count();
        let f2 = w.values().filter(|&&c| c == 2).count();
        if exact {
            w.len() as f64
        } else {
            chao1(w.len(), f1, f2)
        }
    };
    let r = classes(&|i| m[i].clone());
    let c = classes(&|j| (0..k).map(|i| m[i][j]).collect());
    let total = m.iter().flatten().filter(|&&b| b).count() as f64;
    let diag = (0..k).filter(|&i| m[i][i]).count() as f64;
    let off = if k >= 2 {
        (total - diag) / (k * (k - 1)) as f64
    } else {
        1.0
    };
    let x = if off > 0.0 {
        -off.log2()
    } else {
        2.0 * (k.max(2) as f64).log2()
    };
    r.min(c).max(1.0).log2().max(x)
}

fn random_cnf(rng: &mut StdRng, n: usize, m: usize) -> Vec<Vec<i32>> {
    (0..m)
        .map(|_| {
            let len = rng.random_range(1..=4);
            (0..len)
                .map(|_| {
                    let v = rng.random_range(1..=n as i32);
                    if rng.random_bool(0.5) { v } else { -v }
                })
                .collect()
        })
        .collect()
}

#[test]
fn readings_match_the_definition() {
    let mut rng = StdRng::seed_from_u64(7);
    let mut checked = 0;
    for case in 0..300 {
        let n = rng.random_range(2..=9);
        let m = rng.random_range(1..=2 * n);
        let clauses = random_cnf(&mut rng, n, m);
        // A draw of random assignments, some repeated, some not models; up to
        // 130 so the sets span more than two words.
        let size = if case % 3 == 0 {
            rng.random_range(1..=8)
        } else {
            rng.random_range(60..=130)
        };
        let draw: Vec<Vec<bool>> = (0..size)
            .map(|_| (0..n).map(|_| rng.random_bool(0.5)).collect())
            .collect();
        let list = ClauseList::new(&clauses);
        let sample = ModelSample::new(n, &list, draw.clone());
        // The same preparation, spelled out, for the brute force.
        let mut x = draw;
        let d = distinct_rows(&x);
        if d.len() * 2 < x.len() {
            x = d;
        }
        x.retain(|m| list.satisfied_by(m));
        let d = distinct_rows(&x);
        let exact = !x.is_empty() && d.len() * 2 < x.len();
        if exact {
            x = d;
        }
        assert_eq!(sample.k, x.len());
        let mut reader = Reader::new(&list, &sample);
        for _ in 0..6 {
            let inside: Vec<bool> = (0..n).map(|_| rng.random_bool(0.5)).collect();
            let want = brute_force(n, &clauses, &x, exact, &inside);
            let got = reader.raw(&inside);
            assert!(
                (want - got).abs() < 1e-9,
                "case {case}: {got} against {want}"
            );
            let outside: Vec<bool> = inside.iter().map(|&x| !x).collect();
            assert_eq!(
                reader.raw(&outside),
                got,
                "case {case}: the complement reads the same"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 1800);
}

#[test]
fn blocks_follow_the_order_length() {
    assert_eq!(block_bounds(9), vec![0, 4, 9]);
    assert_eq!(block_bounds(8), vec![0, 4, 8]);
    assert_eq!(block_bounds(2), vec![0, 2]);
    let b = block_bounds(300);
    assert_eq!(b.len() - 1, 75);
    assert!(b.windows(2).all(|w| w[1] - w[0] >= 2));
}
