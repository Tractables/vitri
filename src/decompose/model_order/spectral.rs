//! Pair weights from repair sets and the Fiedler order of a weight.
//!
//! A weight is kept as the terms that make it up rather than as a matrix:
//! each term is one repair set, and it joins every pair of its members (a
//! clique term) or the flipped variable to each of the others (a star term)
//! with the same weight. A product with the weight then costs the total size
//! of the sets, and memory never grows with the square of the variables.
//!
//! The order is the one of the Fiedler vector of the normalised Laplacian of
//! `w / max w + eps` (every pair gets `eps`, so the graph is connected): the
//! eigenvector of the second smallest eigenvalue, divided entrywise by the
//! square root of the degrees. Lanczos iteration with full
//! reorthogonalisation finds it as the largest eigenvector of
//! `D^-1/2 W D^-1/2` once the known top eigenvector `sqrt(d)` is projected
//! out.

/// Added to every pair weight after scaling to a largest weight of 1.
const EPS: f64 = 1e-6;

/// Lanczos stops when the residual `|A y - theta y|` of its Ritz vector is
/// under this (the operator's norm is 1).
const RESIDUAL: f64 = 1e-12;

/// A symmetric weight over `n` variables, zero on the diagonal, as a sum of
/// terms.
pub(super) struct Weight {
    n: usize,
    /// Star terms join `members[start[t]]` to each later member of term `t`;
    /// clique terms join every pair of the term's members.
    star: bool,
    start: Vec<usize>,
    members: Vec<u32>,
    c: Vec<f64>,
}

impl Weight {
    pub(super) fn new(n: usize, star: bool) -> Self {
        Weight {
            n,
            star,
            start: vec![0],
            members: Vec::new(),
            c: Vec::new(),
        }
    }

    pub(super) fn len(&self) -> usize {
        self.n
    }

    /// Add a term over `members` (distinct, the star's centre first) with
    /// weight `c` on each of its pairs. A term with fewer than two members
    /// has no pair and is not kept.
    pub(super) fn push(&mut self, members: impl IntoIterator<Item = u32>, c: f64) {
        let at = self.members.len();
        self.members.extend(members);
        if self.members.len() - at < 2 {
            self.members.truncate(at);
            return;
        }
        self.start.push(self.members.len());
        self.c.push(c);
    }

    fn terms(&self) -> impl Iterator<Item = (&[u32], f64)> + '_ {
        self.start
            .windows(2)
            .zip(&self.c)
            .map(|(s, &c)| (&self.members[s[0]..s[1]], c))
    }

    /// `y = W z`.
    pub(super) fn apply(&self, z: &[f64], y: &mut [f64]) {
        y.iter_mut().for_each(|v| *v = 0.0);
        for (m, c) in self.terms() {
            if self.star {
                let centre = m[0] as usize;
                let mut s = 0.0;
                for &d in &m[1..] {
                    s += z[d as usize];
                    y[d as usize] += c * z[centre];
                }
                y[centre] += c * s;
            } else {
                let s: f64 = m.iter().map(|&b| z[b as usize]).sum();
                for &a in m {
                    y[a as usize] += c * (s - z[a as usize]);
                }
            }
        }
    }

    /// The largest pair weight, each pair's weight summed over its terms in
    /// term order.
    pub(super) fn max_entry(&self) -> f64 {
        let mut by_var: Vec<Vec<u32>> = vec![Vec::new(); self.n];
        for (t, (m, _)) in self.terms().enumerate() {
            for &a in m {
                by_var[a as usize].push(t as u32);
            }
        }
        let mut acc = vec![0.0f64; self.n];
        let mut touched = Vec::new();
        let mut best = 0.0f64;
        for (a, terms) in by_var.iter().enumerate() {
            for &t in terms {
                let (s, c) = (self.start[t as usize], self.c[t as usize]);
                let m = &self.members[s..self.start[t as usize + 1]];
                let others: &[u32] = if !self.star {
                    m
                } else if m[0] as usize == a {
                    &m[1..]
                } else {
                    &m[..1]
                };
                for &b in others {
                    if b as usize != a {
                        if acc[b as usize] == 0.0 {
                            touched.push(b);
                        }
                        acc[b as usize] += c;
                    }
                }
            }
            for &b in &touched {
                best = best.max(acc[b as usize]);
                acc[b as usize] = 0.0;
            }
            touched.clear();
        }
        best
    }

    /// The weight among the variables `keep` only, renumbered by their
    /// position in `keep`.
    pub(super) fn restrict(&self, keep: &[usize]) -> Weight {
        let mut local = vec![u32::MAX; self.n];
        for (i, &v) in keep.iter().enumerate() {
            local[v] = i as u32;
        }
        let mut out = Weight::new(keep.len(), self.star);
        for (m, c) in self.terms() {
            if self.star && local[m[0] as usize] == u32::MAX {
                continue;
            }
            out.push(
                m.iter()
                    .map(|&v| local[v as usize])
                    .filter(|&l| l != u32::MAX),
                c,
            );
        }
        out
    }
}

/// How a Lanczos run ended.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Solve {
    pub steps: usize,
    pub residual: f64,
}

/// The variables in the order of the weight's Fiedler vector, the vector
/// oriented so that it rises with the variable index on balance; `flip`
/// takes the other orientation.
pub(super) fn fiedler_order(w: &Weight, flip: bool) -> (Vec<usize>, Solve) {
    let n = w.len();
    if n < 2 {
        return ((0..n).collect(), Solve::default());
    }
    let scale = 1.0 / w.max_entry().max(1e-300);
    let mut deg = vec![0.0; n];
    w.apply(&vec![1.0; n], &mut deg);
    let root: Vec<f64> = deg
        .iter()
        .map(|&d| (d * scale + EPS * (n - 1) as f64).sqrt())
        .collect();
    let mut z = vec![0.0; n];
    let mut wz = vec![0.0; n];
    let op = |x: &[f64], y: &mut [f64]| {
        for ((zi, xi), r) in z.iter_mut().zip(x).zip(&root) {
            *zi = xi / r;
        }
        let sum: f64 = z.iter().sum();
        w.apply(&z, &mut wz);
        for (((yi, wi), zi), r) in y.iter_mut().zip(&wz).zip(&z).zip(&root) {
            *yi = (wi * scale + EPS * (sum - zi)) / r;
        }
    };
    let norm = root.iter().map(|r| r * r).sum::<f64>().sqrt();
    let top: Vec<f64> = root.iter().map(|r| r / norm).collect();
    let (vector, solve) = second_eigenvector(n, op, &top);
    let mut v: Vec<f64> = vector.iter().zip(&root).map(|(y, r)| y / r).collect();
    let mid = (n - 1) as f64 / 2.0;
    let rise: f64 = v
        .iter()
        .enumerate()
        .map(|(i, x)| x * (i as f64 - mid))
        .sum();
    if (rise < 0.0) != flip {
        v.iter_mut().for_each(|x| *x = -*x);
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| v[a].total_cmp(&v[b]));
    (order, solve)
}

/// The eigenvector of the largest eigenvalue of the symmetric operator `op`
/// on the complement of the unit vector `top`, by Lanczos iteration from a
/// fixed start vector with full reorthogonalisation.
fn second_eigenvector(
    n: usize,
    mut op: impl FnMut(&[f64], &mut [f64]),
    top: &[f64],
) -> (Vec<f64>, Solve) {
    let dim = n - 1;
    let mut basis: Vec<Vec<f64>> = Vec::new();
    let (mut alpha, mut beta): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut q = start_vector(n, &mut seed, top, &basis);
    let mut w = vec![0.0; n];
    let (ritz, solve) = loop {
        op(&q, &mut w);
        let a = dot(&q, &w);
        for (wi, qi) in w.iter_mut().zip(&q) {
            *wi -= a * qi;
        }
        if let (Some(prev), Some(&b)) = (basis.last(), beta.last()) {
            for (wi, pi) in w.iter_mut().zip(prev) {
                *wi -= b * pi;
            }
        }
        basis.push(q);
        alpha.push(a);
        for _ in 0..2 {
            orthogonalise(&mut w, top, &basis);
        }
        let b = norm2(&w);
        let s = top_eigenvector(&alpha, &beta);
        let solve = Solve {
            steps: basis.len(),
            residual: b * s.last().unwrap().abs(),
        };
        if solve.residual <= RESIDUAL || basis.len() == dim {
            break (s, solve);
        }
        if b <= 1e-12 {
            // An invariant subspace: carry on from a fresh direction, which
            // leaves the tridiagonal matrix split at this point.
            beta.push(0.0);
            q = start_vector(n, &mut seed, top, &basis);
        } else {
            beta.push(b);
            q = w.iter().map(|x| x / b).collect();
        }
    };
    let mut y = vec![0.0; n];
    for (s, qv) in ritz.iter().zip(&basis) {
        for (yi, qi) in y.iter_mut().zip(qv) {
            *yi += s * qi;
        }
    }
    (y, solve)
}

/// A unit vector orthogonal to `top` and `basis`, from a fixed sequence.
fn start_vector(n: usize, seed: &mut u64, top: &[f64], basis: &[Vec<f64>]) -> Vec<f64> {
    loop {
        let mut v: Vec<f64> = (0..n)
            .map(|_| {
                *seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut x = *seed;
                x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                x ^= x >> 31;
                (x >> 11) as f64 / (1u64 << 52) as f64 - 1.0
            })
            .collect();
        for _ in 0..2 {
            orthogonalise(&mut v, top, basis);
        }
        let len = norm2(&v);
        if len > 1e-8 {
            v.iter_mut().for_each(|x| *x /= len);
            return v;
        }
    }
}

fn orthogonalise(w: &mut [f64], top: &[f64], basis: &[Vec<f64>]) {
    for q in std::iter::once(top).chain(basis.iter().map(Vec::as_slice)) {
        let c = dot(w, q);
        for (wi, qi) in w.iter_mut().zip(q) {
            *wi -= c * qi;
        }
    }
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn norm2(a: &[f64]) -> f64 {
    dot(a, a).sqrt()
}

/// The unit eigenvector of the largest eigenvalue of the symmetric
/// tridiagonal matrix with diagonal `a` and off-diagonal `b`: the eigenvalue
/// by bisection on Sturm counts, the vector by inverse iteration.
fn top_eigenvector(a: &[f64], b: &[f64]) -> Vec<f64> {
    let m = a.len();
    if m == 1 {
        return vec![1.0];
    }
    let off = |i: usize| if i < m - 1 { b[i].abs() } else { 0.0 };
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for (i, &ai) in a.iter().enumerate() {
        let r = off(i) + if i > 0 { off(i - 1) } else { 0.0 };
        lo = lo.min(ai - r);
        hi = hi.max(ai + r);
    }
    // The number of eigenvalues below x.
    let below = |x: f64| {
        let (mut count, mut d) = (0, 1.0);
        for (i, &ai) in a.iter().enumerate() {
            let sub = if i > 0 { b[i - 1] * b[i - 1] / d } else { 0.0 };
            d = ai - x - sub;
            if d == 0.0 {
                d = -f64::MIN_POSITIVE;
            }
            if d < 0.0 {
                count += 1;
            }
        }
        count
    };
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if mid <= lo || mid >= hi {
            break;
        }
        if below(mid) == m {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let theta = hi;
    let mut s = vec![1.0; m];
    for _ in 0..3 {
        s = solve_shifted(a, b, theta, &s);
        let len = norm2(&s);
        s.iter_mut().for_each(|x| *x /= len);
    }
    s
}

/// Solve `(T - theta I) x = rhs` for tridiagonal T by Gaussian elimination
/// with partial pivoting; a zero pivot is replaced by a tiny one, as inverse
/// iteration wants.
fn solve_shifted(a: &[f64], b: &[f64], theta: f64, rhs: &[f64]) -> Vec<f64> {
    let m = a.len();
    let tiny = f64::EPSILON * a.iter().chain(b).fold(1e-300f64, |x, y| x.max(y.abs()));
    // Row i holds (diag, upper, upper2) after elimination.
    let (mut d, mut u, mut u2) = (vec![0.0; m], vec![0.0; m], vec![0.0; m]);
    let mut x = rhs.to_vec();
    let (mut cd, mut cu) = (a[0] - theta, if m > 1 { b[0] } else { 0.0 });
    for i in 0..m {
        if i == m - 1 {
            d[i] = if cd == 0.0 { tiny } else { cd };
            break;
        }
        let (low, nd, nu) = (
            b[i],
            a[i + 1] - theta,
            if i + 2 < m { b[i + 1] } else { 0.0 },
        );
        if cd.abs() >= low.abs() {
            let piv = if cd == 0.0 { tiny } else { cd };
            let f = low / piv;
            d[i] = piv;
            u[i] = cu;
            u2[i] = 0.0;
            x[i + 1] -= f * x[i];
            cd = nd - f * cu;
            cu = nu;
        } else {
            let f = cd / low;
            d[i] = low;
            u[i] = nd;
            u2[i] = nu;
            x.swap(i, i + 1);
            x[i + 1] -= f * x[i];
            cd = cu - f * nd;
            cu = -f * nu;
        }
    }
    for i in (0..m).rev() {
        let mut v = x[i];
        if i + 1 < m {
            v -= u[i] * x[i + 1];
        }
        if i + 2 < m {
            v -= u2[i] * x[i + 2];
        }
        x[i] = v / d[i];
    }
    x
}
