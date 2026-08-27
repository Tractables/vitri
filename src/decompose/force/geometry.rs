//! The geometry the layout runs on: an incidence view of the formula,
//! a deterministic RNG, and the eigen-decomposition that gives the
//! principal axes a layout is whitened onto.

use super::*;

// ---------------------------------------------------------------------------
// Deterministic RNG (xorshift64 + float / Gaussian helpers)
// ---------------------------------------------------------------------------

/// The layout's xorshift64 state and the float and Gaussian draws it needs.
pub(super) struct Rng(u64);

impl Rng {
    /// `+ 1` keeps seed 0 off the recurrence's fixed point. The offset is part
    /// of the stream every existing seed has always drawn.
    pub(super) fn new(seed: u64) -> Self {
        Rng(seed.wrapping_add(1))
    }

    pub(super) fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Uniform in `[0, 1)` from the top 53 bits.
    pub(super) fn next_f64(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64) / ((1u64 << 53) as f64)
    }

    /// Standard-normal sample (Box–Muller). Used only for degenerate-axis jitter, so
    /// a single transform per call (no caching) is fine. `u1` is nudged into `(0, 1]`
    /// to keep `ln` finite.
    fn normal(&mut self) -> f64 {
        let u1 = (((self.next_u64() >> 11) + 1) as f64) / ((1u64 << 53) as f64 + 1.0);
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

// ---------------------------------------------------------------------------
// Incidence (deduped variable membership per clause; polarity ignored)
// ---------------------------------------------------------------------------

/// Flat incidence for the centre-of-gravity math. `var_of[k]` and `clause_of[k]` are
/// parallel member lists with non-decreasing clause index; `sizes[c]` is clause `c`'s
/// member count. Per-variable weight sums are derived in the layout, since they
/// depend on the clause-weighting rule. Empty clauses and duplicate variables within
/// a clause are dropped.
pub(crate) struct Incidence {
    pub(super) var_of: Vec<u32>,
    pub(super) clause_of: Vec<u32>,
    pub(super) sizes: Vec<f64>,
    pub(super) nc: usize,
}

pub(crate) fn build_incidence(formula: &CnfFormula) -> Incidence {
    let mut var_of = Vec::new();
    let mut clause_of = Vec::new();
    let mut sizes = Vec::new();
    let mut members: Vec<u32> = Vec::new();
    let mut ci: u32 = 0;
    for clause in &formula.clauses {
        members.clear();
        for lit in &clause.literals {
            members.push(lit.var.0);
        }
        members.sort_unstable();
        members.dedup();
        if members.is_empty() {
            continue;
        }
        sizes.push(members.len() as f64);
        for &v in &members {
            var_of.push(v);
            clause_of.push(ci);
        }
        ci += 1;
    }
    Incidence {
        var_of,
        clause_of,
        sizes,
        nc: ci as usize,
    }
}

// ---------------------------------------------------------------------------
// Symmetric eigendecomposition
// ---------------------------------------------------------------------------

/// Eigenvectors of the symmetric matrix `[[a, b], [b, c]]` as an orthonormal basis.
/// Returns `(v0, v1)` where `v0` spans the SMALLER eigenvalue and `v1` the LARGER,
/// matching the ascending convention of the general solver below. The sign is
/// deterministic — a global flip only swaps cut halves or the whitening direction,
/// never validity.
pub(super) fn eig_axes(a: f64, b: f64, c: f64) -> ([f64; 2], [f64; 2]) {
    let tr = a + c;
    let d = a - c;
    let r = ((d * 0.5).powi(2) + b * b).sqrt();
    let lam1 = tr * 0.5 + r; // larger eigenvalue
    // Eigenvector of lam1: null space of (A − lam1·I). Pick the wider row for
    // numerical stability; fall back to the x-axis when the matrix is isotropic.
    let r1 = [a - lam1, b];
    let r2 = [b, c - lam1];
    let n1 = r1[0] * r1[0] + r1[1] * r1[1];
    let n2 = r2[0] * r2[0] + r2[1] * r2[1];
    let mut v1 = if n1 >= n2 {
        [-r1[1], r1[0]]
    } else {
        [-r2[1], r2[0]]
    };
    let norm = (v1[0] * v1[0] + v1[1] * v1[1]).sqrt();
    if norm < EPS {
        v1 = [1.0, 0.0];
    } else {
        v1[0] /= norm;
        v1[1] /= norm;
    }
    let v0 = [-v1[1], v1[0]];
    (v0, v1)
}

/// Euclidean distance between two `d`-dimensional points.
pub(super) fn dist(a: &[f64], b: &[f64]) -> f64 {
    let mut s = 0.0;
    for (x, y) in a.iter().zip(b.iter()) {
        let dk = x - y;
        s += dk * dk;
    }
    s.sqrt()
}

/// Inner product `Σ a[k]·b[k]`, accumulated LEFT to right starting from the `k = 0`
/// term rather than from a `0.0` seed, so a rotation onto a principal axis keeps the
/// sign of a zero result — which `total_cmp` distinguishes and downstream sorts
/// depend on.
pub(super) fn dot(a: &[f64], b: &[f64]) -> f64 {
    let mut it = a.iter().zip(b.iter());
    let mut s = match it.next() {
        Some((x, y)) => x * y,
        None => return 0.0,
    };
    for (x, y) in it {
        s += x * y;
    }
    s
}

/// Deterministic cyclic-Jacobi eigendecomposition of a symmetric `d × d` matrix
/// (`d ≤ `[`MAX_DIM`]). Returns `(eigenvalues, vecs)` where `vecs[r][c]` is row `r`
/// of the eigenvector matrix — eigenvector `c` is the COLUMN
/// `[vecs[0][c], …, vecs[d-1][c]]`. Fixed sweep budget ([`JACOBI_SWEEPS`]) with an
/// early exit once the off-diagonal mass is machine-negligible; no external
/// dependency and no time-based logic, so it is fully deterministic. Used only for
/// `d > 2`; `d = 2` keeps the closed-form [`eig_axes`].
pub(super) fn jacobi_eigen(mat: &[Vec<f64>]) -> (Vec<f64>, Vec<Vec<f64>>) {
    let d = mat.len();
    let mut a: Vec<Vec<f64>> = mat.to_vec();
    let mut v = vec![vec![0.0f64; d]; d];
    for (i, row) in v.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for _ in 0..JACOBI_SWEEPS {
        // Off-diagonal mass; stop when it is negligible.
        let mut off = 0.0;
        for (p, row) in a.iter().enumerate() {
            for &x in &row[(p + 1)..] {
                off += x * x;
            }
        }
        if off <= EPS * EPS {
            break;
        }
        for p in 0..d {
            for q in (p + 1)..d {
                if a[p][q].abs() <= f64::MIN_POSITIVE {
                    continue;
                }
                // Rotation angle that zeroes a[p][q] (Golub–Van Loan 8.4).
                let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                // A ← Jᵀ A J (update columns then rows p, q), V ← V J.
                for row in a.iter_mut() {
                    let aip = row[p];
                    let aiq = row[q];
                    row[p] = c * aip - s * aiq;
                    row[q] = s * aip + c * aiq;
                }
                // Rows p and q are distinct (`q > p`), so one split hands out
                // both at once and the pair of rows walks in lockstep.
                let (upper, lower) = a.split_at_mut(q);
                for (apj, aqj) in upper[p].iter_mut().zip(lower[0].iter_mut()) {
                    let (old_p, old_q) = (*apj, *aqj);
                    *apj = c * old_p - s * old_q;
                    *aqj = s * old_p + c * old_q;
                }
                for row in v.iter_mut() {
                    let vip = row[p];
                    let viq = row[q];
                    row[p] = c * vip - s * viq;
                    row[q] = s * vip + c * viq;
                }
            }
        }
    }
    let vals: Vec<f64> = a.iter().enumerate().map(|(i, row)| row[i]).collect();
    (vals, v)
}

/// Orthonormal principal axes of the already-scaled covariance `covn`, ordered by
/// ASCENDING eigenvalue (axis 0 = smallest variance, last axis = top principal
/// direction). For `d = 2` this is the closed-form [`eig_axes`]; above it runs
/// [`jacobi_eigen`] and sorts, with a deterministic sign convention (first
/// significant component positive) so the layout is reproducible.
pub(super) fn principal_axes(covn: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let d = covn.len();
    if d == 2 {
        let (v0, v1) = eig_axes(covn[0][0], covn[0][1], covn[1][1]);
        return vec![v0.to_vec(), v1.to_vec()];
    }
    let (vals, vecs) = jacobi_eigen(covn);
    let mut order: Vec<usize> = (0..d).collect();
    order.sort_by(|&i, &j| vals[i].total_cmp(&vals[j]).then(i.cmp(&j)));
    order
        .iter()
        .map(|&col| {
            let mut ev: Vec<f64> = vecs.iter().map(|row| row[col]).collect();
            if let Some(&first) = ev.iter().find(|x| x.abs() > EPS)
                && first < 0.0
            {
                for x in ev.iter_mut() {
                    *x = -*x;
                }
            }
            ev
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Whitening
// ---------------------------------------------------------------------------

/// Recentre, rotate into principal axes, rescale each axis to unit standard
/// deviation. Without this, repeated centre-of-gravity averaging is a linear
/// smoothing operator whose fixed point is the constant vector, so the point
/// cloud collapses onto one point every few rounds; whitening projects that mode
/// back out each round. A degenerate axis (std below [`EPS`]) gets seeded
/// Gaussian jitter and is then unit-normalized.
pub(super) fn whiten(p: &mut [Vec<f64>], d: usize, rng: &mut Rng) {
    let n = p.len();
    if n < 2 {
        return;
    }
    let nf = n as f64;
    // Recentre (per-coordinate mean, accumulated in point order).
    let mut mean = vec![0.0f64; d];
    for q in p.iter() {
        for (m, x) in mean.iter_mut().zip(q.iter()) {
            *m += x;
        }
    }
    for m in mean.iter_mut() {
        *m /= nf;
    }
    for q in p.iter_mut() {
        for (x, m) in q.iter_mut().zip(mean.iter()) {
            *x -= m;
        }
    }
    // Covariance (points already centred, divided by n − 1).
    let denom = (nf - 1.0).max(1.0);
    let mut covn = vec![vec![0.0f64; d]; d];
    for q in p.iter() {
        for (i, row) in covn.iter_mut().enumerate() {
            let qi = q[i];
            for (x, qj) in row.iter_mut().zip(q.iter()) {
                *x += qi * qj;
            }
        }
    }
    for row in covn.iter_mut() {
        for x in row.iter_mut() {
            *x /= denom;
        }
    }
    // Rotate into principal axes: axis j ← axes[j] (ascending eigenvalue order).
    let axes = principal_axes(&covn);
    let mut old = vec![0.0f64; d];
    for q in p.iter_mut() {
        old.copy_from_slice(q);
        for (x, ax) in q.iter_mut().zip(axes.iter()) {
            *x = dot(&old, ax);
        }
    }
    // Rescale each axis to unit population standard deviation.
    for axis in 0..d {
        let mut std = axis_std(p, axis, nf);
        if std < EPS {
            for q in p.iter_mut() {
                q[axis] += rng.normal() * 1e-3;
            }
            std = axis_std(p, axis, nf).max(EPS);
        }
        for q in p.iter_mut() {
            q[axis] /= std;
        }
    }
}

pub(super) fn axis_std(p: &[Vec<f64>], axis: usize, nf: f64) -> f64 {
    let mut mean = 0.0;
    for q in p {
        mean += q[axis];
    }
    mean /= nf;
    let mut var = 0.0;
    for q in p {
        let d = q[axis] - mean;
        var += d * d;
    }
    (var / nf).sqrt()
}
