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

/// The group of each model, one model and the size of each group, and the
/// row of each group.
type Grouped = (Vec<u32>, Vec<u32>, Vec<u32>, Vec<u64>);

/// The groups and rows of the reader's merged clauses built the plain way:
/// memberships one model at a time, eight clauses to a byte, and every union
/// of each byte's clauses.
fn plain_rows(r: &Reader<'_>) -> Grouped {
    let (k, words) = (r.sample.k, r.sample.words);
    let clauses = r.merged_in.len() / words;
    let groups = clauses.div_ceil(8);
    let mut bytes = vec![0u8; k * groups];
    for (j, set) in r.merged_in.chunks_exact(words).enumerate() {
        for i in 0..k {
            if (set[i / 64] >> (i % 64)) & 1 == 1 {
                bytes[i * groups + j / 8] |= 1 << (j % 8);
            }
        }
    }
    let (mut rep_of, mut rep_model, mut weight) = (Vec::new(), Vec::new(), Vec::new());
    let mut first: HashMap<&[u8], u32> = HashMap::new();
    for i in 0..k {
        let next = first.len() as u32;
        let g = *first
            .entry(&bytes[i * groups..(i + 1) * groups])
            .or_insert(next);
        if g == next {
            rep_model.push(i as u32);
            weight.push(0);
        }
        weight[g as usize] += 1;
        rep_of.push(g);
    }
    let mut rows = vec![0u64; rep_model.len() * words];
    for g in 0..groups {
        let q = (clauses - 8 * g).min(8);
        let mut table = vec![0u64; (1 << q) * words];
        for b in 1..1usize << q {
            for c in (0..q).filter(|c| (b >> c) & 1 == 1) {
                let set = &r.merged_out[(8 * g + c) * words..(8 * g + c + 1) * words];
                for (d, s) in table[b * words..(b + 1) * words].iter_mut().zip(set) {
                    *d |= s;
                }
            }
        }
        for (rep, &m) in rep_model.iter().enumerate() {
            let b = bytes[m as usize * groups + g] as usize;
            for (d, s) in rows[rep * words..(rep + 1) * words]
                .iter_mut()
                .zip(&table[b * words..(b + 1) * words])
            {
                *d |= s;
            }
        }
    }
    (rep_of, rep_model, weight, rows)
}

#[test]
fn groups_and_rows_match_the_plain_build() {
    let mut rng = StdRng::seed_from_u64(11);
    let (mut cuts, mut wide) = (0, 0);
    for _ in 0..60 {
        let n = rng.random_range(20..=80);
        let m = rng.random_range(50..=400);
        let clauses = random_cnf(&mut rng, n, m);
        let list = ClauseList::new(&clauses);
        // Assignments rather than models: groups and rows are defined for any
        // sample.
        let size = rng.random_range(1..=300);
        let draw: Vec<Vec<bool>> = (0..size)
            .map(|_| (0..n).map(|_| rng.random_bool(0.5)).collect())
            .collect();
        let sample = ModelSample::new(n, &ClauseList::new(&[]), draw);
        let mut reader = Reader::new(&list, &sample);
        for _ in 0..4 {
            let inside: Vec<bool> = (0..n).map(|_| rng.random_bool(0.5)).collect();
            if !reader.open_sets(&inside) {
                continue;
            }
            reader.merge_by_outside();
            reader.group_models();
            reader.form_rows();
            let (rep_of, rep_model, weight, rows) = plain_rows(&reader);
            assert_eq!(reader.rep_of, rep_of);
            assert_eq!(reader.rep_model, rep_model);
            assert_eq!(reader.weight, weight);
            assert_eq!(reader.rows, rows);
            cuts += 1;
            wide += usize::from(reader.merged_in.len() / sample.words > 64);
        }
    }
    assert!(
        cuts >= 200 && wide >= 50,
        "{cuts} cuts, {wide} past 64 clauses"
    );
}
