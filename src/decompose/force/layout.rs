//! The layout itself: the 1D pre-pass that seeds dimension zero, the
//! iterated force placement, and the cost it is measured by.

use super::*;

// ---------------------------------------------------------------------------
// 1D FORCE pre-pass (the `init=force1d` seed for dimension 0)
// ---------------------------------------------------------------------------

/// Classic 1D FORCE pre-pass. Always uses uniform clause weighting, independent of
/// `cfg.clause_weight` — the main layout iteration honors that axis, this pre-pass
/// does not. Returns each variable's final integer rank as an unscaled `f64`.
pub(super) fn force1d_ranks(n: usize, inc: &Incidence, rng: &mut Rng) -> Vec<f64> {
    // Random permutation (Fisher–Yates) → initial integer positions per variable.
    let mut perm: Vec<u32> = (0..n as u32).collect();
    for i in (1..n).rev() {
        let j = (rng.next_u64() % (i as u64 + 1)) as usize;
        perm.swap(i, j);
    }
    let mut pos = vec![0.0f64; n];
    for (rank, &v) in perm.iter().enumerate() {
        pos[v as usize] = rank as f64;
    }
    if inc.var_of.is_empty() || inc.nc == 0 {
        return pos; // no clauses → the random permutation is the layout
    }
    // Per-variable clause count (uniform weighting).
    let mut deg = vec![0.0f64; n];
    for &v in &inc.var_of {
        deg[v as usize] += 1.0;
    }
    let mut prev_order = perm; // current rank order (perm is order-by-rank)
    for _ in 0..FORCE1D_ROUNDS {
        // Clause centre of gravity (mean member position).
        let mut cog = vec![0.0f64; inc.nc];
        for (&v, &c) in inc.var_of.iter().zip(inc.clause_of.iter()) {
            cog[c as usize] += pos[v as usize];
        }
        for (g, &sz) in cog.iter_mut().zip(inc.sizes.iter()) {
            *g /= sz;
        }
        // New variable position = mean centre of gravity over its clauses; a
        // clause-free variable keeps its position.
        let mut npos = vec![0.0f64; n];
        for (&v, &c) in inc.var_of.iter().zip(inc.clause_of.iter()) {
            npos[v as usize] += cog[c as usize];
        }
        for (v, pv) in pos.iter_mut().enumerate() {
            if deg[v] > 0.0 {
                *pv = npos[v] / deg[v];
            }
        }
        // Re-rank to integer positions 0..n-1 (stable tie-break on variable index).
        let mut order: Vec<u32> = (0..n as u32).collect();
        order.sort_by(|&a, &b| pos[a as usize].total_cmp(&pos[b as usize]).then(a.cmp(&b)));
        for (rank, &v) in order.iter().enumerate() {
            pos[v as usize] = rank as f64;
        }
        if order == prev_order {
            break; // ranking converged
        }
        prev_order = order;
    }
    pos
}

// ---------------------------------------------------------------------------
// The layout
// ---------------------------------------------------------------------------

/// `d`-dimensional FORCE embedding with per-round whitening. Returns the best-cost
/// layout (`n` points, each with `cfg.dim` coordinates).
///
/// `extra_w` is an optional per-clause multiplier folded into the layout weight (the
/// `fb` metric feedback); `None` leaves the weights at the base [`ClauseWeight`]
/// rule. `warm` optionally seeds the point cloud from a previous round's positions
/// (the `fb` warm start) instead of the fresh initialization `cfg.init` otherwise
/// selects.
pub(super) fn force_layout(
    n: usize,
    inc: &Incidence,
    seed: u64,
    cfg: &ForceConfig,
    extra_w: Option<&[f64]>,
    warm: Option<&[Vec<f64>]>,
) -> Vec<Vec<f64>> {
    let d = cfg.dim;
    let mut rng = Rng::new(seed);
    // Fresh init (d coordinates per point, in rng order) unless warm-started.
    let mut p: Vec<Vec<f64>> = match warm {
        Some(prev) => prev.to_vec(),
        None => match cfg.init {
            InitMode::Rand => (0..n)
                .map(|_| (0..d).map(|_| rng.next_f64()).collect())
                .collect(),
            InitMode::Force1d => {
                // The 1D FORCE pre-pass consumes rng first (the permutation), then
                // dimensions 1.. draw uniform random. Dimension 0 is the rank scaled
                // to unit variance.
                let ranks = force1d_ranks(n, inc, &mut rng);
                let nf = n as f64;
                let mean = ranks.iter().sum::<f64>() / nf;
                let var = ranks.iter().map(|r| (r - mean) * (r - mean)).sum::<f64>() / nf;
                let std = var.sqrt().max(EPS);
                (0..n)
                    .map(|v| {
                        let mut coords = vec![0.0f64; d];
                        coords[0] = (ranks[v] - mean) / std;
                        for coord in coords.iter_mut().skip(1) {
                            *coord = rng.next_f64();
                        }
                        coords
                    })
                    .collect()
            }
        },
    };

    let has_edges = !inc.var_of.is_empty() && inc.nc > 0;
    if !has_edges {
        whiten(&mut p, d, &mut rng);
        return p;
    }

    let nc = inc.nc;
    // Base per-clause layout weight, before `extra_w` (the fb feedback) multiplies
    // in.
    let base_w = |c: usize| -> f64 {
        match cfg.clause_weight {
            ClauseWeight::Uniform => 1.0,
            ClauseWeight::Short => 1.0 / (inc.sizes[c] - 1.0).max(1.0),
        }
    };
    let eff_w: Vec<f64> = match extra_w {
        None => (0..nc).map(base_w).collect(),
        Some(ew) => (0..nc).map(|c| base_w(c) * ew[c]).collect(),
    };
    // The per-variable weight sum is fixed across rounds (membership and weights are).
    let mut wsum = vec![0.0f64; n];
    for (&v, &c) in inc.var_of.iter().zip(inc.clause_of.iter()) {
        wsum[v as usize] += eff_w[c as usize];
    }
    let mut best_p = p.clone();
    let mut best_cost = f64::INFINITY;
    let mut no_improve = 0;
    // Scratch reused across rounds (kept O(d·nc)): the clause COG per axis.
    let mut g = vec![vec![0.0f64; nc]; d];

    for _ in 0..200 {
        clause_cogs(&p, inc, &mut g);
        // New variable position = clause-weighted mean COG over its clauses; a
        // variable of weight 0 keeps its position.
        let mut npos = vec![vec![0.0f64; n]; d];
        for (&v, &c) in inc.var_of.iter().zip(inc.clause_of.iter()) {
            let (v, c) = (v as usize, c as usize);
            let w = eff_w[c];
            for (row, gr) in npos.iter_mut().zip(g.iter()) {
                row[v] += w * gr[c];
            }
        }
        let mut next = p.clone();
        for (v, nv) in next.iter_mut().enumerate() {
            if wsum[v] > 0.0 {
                for (x, row) in nv.iter_mut().zip(npos.iter()) {
                    *x = row[v] / wsum[v];
                }
            }
        }
        whiten(&mut next, d, &mut rng);
        p = next;

        let c = layout_cost(&p, inc, &mut g);
        if c < best_cost - 1e-12 {
            best_cost = c;
            best_p.clone_from(&p);
            no_improve = 0;
        } else {
            no_improve += 1;
            if no_improve >= 10 {
                break;
            }
        }
    }
    best_p
}

/// Fill `g[axis][c]` with clause `c`'s centre of gravity along `axis`. `g` is the
/// caller's reused `[d][nc]` scratch buffer.
pub(super) fn clause_cogs(p: &[Vec<f64>], inc: &Incidence, g: &mut [Vec<f64>]) {
    for row in g.iter_mut() {
        row.iter_mut().for_each(|x| *x = 0.0);
    }
    for (&v, &c) in inc.var_of.iter().zip(inc.clause_of.iter()) {
        let (v, c) = (v as usize, c as usize);
        for (row, x) in g.iter_mut().zip(p[v].iter()) {
            row[c] += x;
        }
    }
    for row in g.iter_mut() {
        for (x, &sz) in row.iter_mut().zip(inc.sizes.iter()) {
            *x /= sz;
        }
    }
}

/// Sum over clauses of the mean member-to-COG distance. `g` is a reused scratch
/// buffer of shape `[d][nc]`.
pub(super) fn layout_cost(p: &[Vec<f64>], inc: &Incidence, g: &mut [Vec<f64>]) -> f64 {
    clause_cogs(p, inc, g);
    let mut per_clause = vec![0.0f64; inc.nc];
    for (&v, &c) in inc.var_of.iter().zip(inc.clause_of.iter()) {
        let (v, c) = (v as usize, c as usize);
        let mut dd = 0.0;
        for (row, x) in g.iter().zip(p[v].iter()) {
            let e = x - row[c];
            dd += e * e;
        }
        per_clause[c] += dd.sqrt();
    }
    let mut total = 0.0;
    for (&pc, &sz) in per_clause.iter().zip(inc.sizes.iter()) {
        total += pc / sz;
    }
    total
}
