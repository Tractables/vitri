//! Where the orders' pair weights come from without a solver call: mutual
//! information on the building draw and clause co-occurrence, blended.
//!
//! Every source gives weighted sets, the centre first, in the form the orders
//! read: a repair set with scale 1; a clause's variables with scale 1, so the
//! all-sets weight of a pair is the sum over clauses holding both of
//! `1 / (|c| - 1)`; and for each variable u the variables whose mutual
//! information with u is at least [`NEAR`] of u's largest, with that largest
//! as the scale.

use crate::preprocess::models::ModelMatrix;

/// Sets of variables, each with its scale.
pub(super) type Sets = Vec<(Vec<u32>, f64)>;

/// A variable's mutual-information set holds the variables within this share
/// of its largest mutual information.
const NEAR: f32 = 0.5;

/// At most this many variables join a variable's mutual-information set.
const NEIGHBOURS: usize = 64;

/// The mutual information in bits of every pair of variables over the rows,
/// row-major in an `n` by `n` matrix; zero for a variable constant over them.
pub(super) fn mutual(models: &ModelMatrix) -> Vec<f32> {
    let (n, k) = (models.vars(), models.len());
    let words = k.div_ceil(64);
    let mut cols = vec![0u64; n * words];
    for r in 0..k {
        for v in 0..n {
            if models.get(r, v) {
                cols[v * words + r / 64] |= 1 << (r % 64);
            }
        }
    }
    let col = |v: usize| &cols[v * words..(v + 1) * words];
    let ones: Vec<usize> = (0..n)
        .map(|v| col(v).iter().map(|w| w.count_ones() as usize).sum())
        .collect();
    let varies = |v: usize| ones[v] > 0 && ones[v] < k;
    let kf = k as f64;
    let mut mi = vec![0f32; n * n];
    for a in (0..n).filter(|&a| varies(a)) {
        for b in (a + 1..n).filter(|&b| varies(b)) {
            let both: usize = col(a)
                .iter()
                .zip(col(b))
                .map(|(x, y)| (x & y).count_ones() as usize)
                .sum();
            let (na, nb, n11) = (ones[a] as f64, ones[b] as f64, both as f64);
            let cells = [
                (n11, na, nb),
                (na - n11, na, kf - nb),
                (nb - n11, kf - na, nb),
                (kf - na - nb + n11, kf - na, kf - nb),
            ];
            let m: f64 = cells
                .iter()
                .filter(|c| c.0 > 0.0)
                .map(|&(c, x, y)| c / kf * (c * kf / (x * y)).log2())
                .sum();
            mi[a * n + b] = m as f32;
            mi[b * n + a] = m as f32;
        }
    }
    mi
}

/// For each variable with some mutual information, the set of it and the
/// variables within [`NEAR`] of its largest, strongest first and at most
/// [`NEIGHBOURS`] of them, scaled by that largest.
pub(super) fn mutual_sets(mi: &[f32], n: usize) -> Sets {
    let mut out = Vec::new();
    for u in 0..n {
        let row = &mi[u * n..(u + 1) * n];
        let top = row.iter().copied().fold(0f32, f32::max);
        if top < 1e-6 {
            continue;
        }
        let mut d: Vec<u32> = (0..n as u32)
            .filter(|&v| v as usize != u && row[v as usize] >= NEAR * top)
            .collect();
        d.sort_by(|&a, &b| row[b as usize].total_cmp(&row[a as usize]).then(a.cmp(&b)));
        d.truncate(NEIGHBOURS);
        let mut s = vec![u as u32];
        s.extend(d);
        out.push((s, f64::from(top)));
    }
    out
}

/// Each clause's distinct variables, its first literal's first.
pub(super) fn clause_sets(clauses: &[Vec<i32>]) -> Sets {
    clauses
        .iter()
        .map(|c| {
            let mut s: Vec<u32> = Vec::with_capacity(c.len());
            for l in c {
                let v = l.unsigned_abs() - 1;
                if !s.contains(&v) {
                    s.push(v);
                }
            }
            (s, 1.0)
        })
        .filter(|s| s.0.len() >= 2)
        .collect()
}

/// `sets` scaled so that their all-sets weight sums to `mass` over each
/// variable's pairs (the sum of scale times size is `mass`).
pub(super) fn normalised(mut sets: Sets, mass: f64) -> Sets {
    let total: f64 = sets
        .iter()
        .filter(|s| s.0.len() >= 2)
        .map(|(s, c)| c * s.len() as f64)
        .sum();
    if total > 0.0 {
        sets.iter_mut().for_each(|s| s.1 *= mass / total);
    }
    sets
}

/// Mutual information and clause sets, each scaled to a mass of `n`.
pub(super) fn blend(n: usize, mutual: Sets, clauses: Sets) -> Sets {
    let mut out = normalised(mutual, n as f64);
    out.extend(normalised(clauses, n as f64));
    out
}
