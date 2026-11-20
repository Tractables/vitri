//! The two figures the pick between the portfolio's tree and the learned tree
//! reads off each tree.
//!
//! `s = S + W / 8`, on the held-out draw: with `w0(v)` the static width of node
//! v (the least of its variables that occur in a crossing clause, the outside
//! variables that do, and its crossing clauses) and `rho(v)` the unclipped
//! reading of v capped at `w0(v)`, `S` is log2 of the sum of `2^rho(v)` and `W`
//! the largest `w0(v)`, both over the internal nodes but the root.
//!
//! Omega, from the formula alone: the most orphans of an internal node, an
//! orphan of v being a variable of v that occurs in no clause lying wholly
//! inside v.

use super::Spend;
use super::bracket::Tree;
use crate::decompose::reading::{ClauseList, Readings};

/// Units of static width worth one bit of reading in `s`.
const WIDTH_PER_BIT: f64 = 8.0;

/// `s` of `tree` from the held-out `readings`, or none when the allowance runs
/// out first.
pub(super) fn static_score(
    tree: &Tree,
    clauses: &ClauseList,
    readings: &mut Readings<'_>,
    spend: &mut Spend,
) -> Option<f64> {
    let n = tree.order.len();
    let mut inside = vec![false; n];
    let mut side = vec![0u8; n];
    let root = tree.kids.len() - 1;
    let (mut total, mut widest) = (0.0f64, 0usize);
    for (v, kids) in tree.kids.iter().enumerate() {
        if kids.is_none() || v == root {
            continue;
        }
        let (lo, hi) = tree.run[v];
        inside.iter_mut().for_each(|b| *b = false);
        for &x in &tree.order[lo as usize..hi as usize] {
            inside[x] = true;
        }
        let w0 = static_width(clauses, &inside, &mut side);
        let rho = if w0 == 0 {
            0.0
        } else {
            let before = readings.reads();
            let r = readings.read(&inside);
            let new = readings.reads() - before;
            spend.charge_reads(new, clauses, readings.sample());
            spend.stages.reads[2] += new;
            if spend.out() {
                return None;
            }
            r.min(w0 as f64)
        };
        total += rho.exp2();
        widest = widest.max(w0);
    }
    Some(total.max(1.0).log2() + widest as f64 / WIDTH_PER_BIT)
}

/// `s` of `tree` with every reading at its cap and needing no draw,
/// `log2 Σ 2^w0(v) + W / 8` over the internal nodes but the root of nonzero
/// width; and `W`.
pub(super) fn static_bound(tree: &Tree, clauses: &ClauseList) -> (f64, usize) {
    let n = tree.order.len();
    let mut inside = vec![false; n];
    let mut side = vec![0u8; n];
    let root = tree.kids.len() - 1;
    let (mut total, mut widest) = (0.0f64, 0usize);
    for (v, kids) in tree.kids.iter().enumerate() {
        if kids.is_none() || v == root {
            continue;
        }
        let (lo, hi) = tree.run[v];
        inside.iter_mut().for_each(|b| *b = false);
        for &x in &tree.order[lo as usize..hi as usize] {
            inside[x] = true;
        }
        let w0 = static_width(clauses, &inside, &mut side);
        if w0 > 0 {
            total += (w0 as f64).exp2();
        }
        widest = widest.max(w0);
    }
    (
        total.max(1.0).log2() + widest as f64 / WIDTH_PER_BIT,
        widest,
    )
}

/// The static width of the cut with `inside` the inside variables; `side` is
/// scratch of one byte per variable, zero on entry and on return.
pub(super) fn static_width(clauses: &ClauseList, inside: &[bool], side: &mut [u8]) -> usize {
    let mut crossing = 0;
    let mut open = [0usize; 2];
    let mut touched = Vec::new();
    for c in 0..clauses.len() {
        let vars = clauses.vars(c);
        let ins = vars.iter().filter(|&&x| inside[x as usize]).count();
        if ins == 0 || ins == vars.len() {
            continue;
        }
        crossing += 1;
        for &x in vars {
            let out = usize::from(!inside[x as usize]);
            let s = &mut side[x as usize];
            if *s == 0 {
                touched.push(x);
            }
            if *s & (1 << out) == 0 {
                *s |= 1 << out;
                open[out] += 1;
            }
        }
    }
    for x in touched {
        side[x as usize] = 0;
    }
    crossing.min(open[0]).min(open[1])
}

/// Omega of `tree` over the formula's clauses.
pub(super) fn omega(tree: &Tree, clauses: &ClauseList) -> usize {
    let m = tree.kids.len();
    let n = tree.order.len();
    // Parents, and ancestors 2^j steps up, for the lowest node over a clause.
    let mut parent = vec![(m - 1) as u32; m];
    for (v, kids) in tree.kids.iter().enumerate() {
        if let Some((l, r)) = *kids {
            parent[l as usize] = v as u32;
            parent[r as usize] = v as u32;
        }
    }
    let mut up = vec![parent];
    while (1usize << up.len()) < m {
        let last = up.last().expect("one level");
        let next = last.iter().map(|&p| last[p as usize]).collect();
        up.push(next);
    }
    let mut leaf_at = vec![0u32; n];
    for (v, kids) in tree.kids.iter().enumerate() {
        if kids.is_none() {
            leaf_at[tree.run[v].0 as usize] = v as u32;
        }
    }
    let mut position = vec![0u32; n];
    for (p, &x) in tree.order.iter().enumerate() {
        position[x] = p as u32;
    }
    let covers = |v: u32, hi: u32| tree.run[v as usize].1 > hi;
    // Per variable, the smallest node over a clause that holds it.
    let mut lowest: Vec<Option<u32>> = vec![None; n];
    for c in 0..clauses.len() {
        let vars = clauses.vars(c);
        let Some(lo) = vars.iter().map(|&x| position[x as usize]).min() else {
            continue;
        };
        let hi = vars
            .iter()
            .map(|&x| position[x as usize])
            .max()
            .unwrap_or(lo);
        let mut v = leaf_at[lo as usize];
        if !covers(v, hi) {
            for level in up.iter().rev() {
                if !covers(level[v as usize], hi) {
                    v = level[v as usize];
                }
            }
            v = up[0][v as usize];
        }
        let size = |u: u32| tree.run[u as usize].1 - tree.run[u as usize].0;
        for &x in vars {
            let best = &mut lowest[x as usize];
            if best.is_none_or(|b| size(v) < size(b)) {
                *best = Some(v);
            }
        }
    }
    // A variable is an orphan of the nodes below its smallest clause node.
    let mut held = vec![0u32; m];
    for &v in lowest.iter().flatten() {
        held[v as usize] += 1;
    }
    let mut most = 0;
    for v in 0..m {
        if let Some((l, r)) = tree.kids[v] {
            held[v] += held[l as usize] + held[r as usize];
            let (lo, hi) = tree.run[v];
            most = most.max((hi - lo - held[v]) as usize);
        }
    }
    most
}
