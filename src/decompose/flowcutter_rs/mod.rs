//! Pure-Rust port of FlowCutter's anytime balanced-separator search: the
//! algorithm in `vendor/treedecomp/upstream/flow-cutter-pace17/src/` plus the
//! `IFlowCutter::computeSeparator` driver in `vendor/treedecomp/upstream/IFlowCutter.cpp`.
//!
//! Structural port only — it does **not** match the C++ output bit-for-bit:
//! the inner RNG (mt19937 → `SmallRng`) and BFS tie-breaking don't reproduce
//! C++ STL behaviour, and `cutter_count` is deliberately pinned to 1 rather
//! than incremented every 16 iterations as upstream does. Parity is verified
//! empirically, not by construction.
//!
//! **Scope — this is the separator subroutine only, not a tree decomposer.**
//! Entry point [`compute_separator`] returns ONE balanced vertex separator, and
//! [`separator`] wraps it with the two sides a caller splitting a graph needs.
//! Full tree-decomposition construction still crosses into C++ (multilevel
//! recursion, cell→bag conversion, greedy elimination orders) via FFI in
//! [`super::flowcutter`]; the two are separate halves of FlowCutter, not two
//! implementations of one thing.
//!
//! The RNG difference above is why the port's separators are not the C++
//! separators bit-for-bit; quality is at parity on the benchmark suite.
//!

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use std::time::Instant;

mod cutter;
mod expanded;
mod graph;
mod separator;
use cutter::*;
use expanded::*;
use graph::*;

pub(super) use separator::flowcutter_compute_separator;

/// Milliseconds of construction work since `start`, on whichever clock the
/// meter is serving.
///
/// Outside a metered construction that is the real wall, and every caller
/// behaves exactly as it did before the meter existed. Inside one it is charged
/// work, so a loop cannot outrun a budget that was measured against the same
/// clock.
#[inline]
fn elapsed_since(start: Instant) -> u128 {
    crate::decompose::meter::now()
        .saturating_duration_since(start)
        .as_millis()
}

/// Compute one balanced vertex separator using FlowCutter's anytime
/// max-flow / Pareto-balance search.
///
/// `edges` is a list of undirected edges as (u, v) pairs (0-indexed,
/// u < n, v < n, u != v).  Self-loops and duplicates are tolerated but not
/// recommended.  The returned separator is a sorted list of vertex IDs
/// in the input space.
///
/// `steps` is the soft step budget (subtracted by `sqrt(n)*sqrt(2m)/50` per
/// iteration).  `iters` caps the number of outer iterations.
/// `timeout_ms = 0` disables the wall-clock deadline.
///
/// Returns `None` if the input is degenerate (n < 3, disconnected, or no
/// non-trivial separator found within budget).
pub(super) fn compute_separator(
    n: usize,
    edges: &[(u32, u32)],
    steps: i64,
    iters: i32,
    timeout_ms: i64,
) -> Option<Vec<u32>> {
    if n < 3 {
        return None;
    }

    let g = OrigGraph::build(n as u32, edges)?;
    if !is_connected(&g) {
        return None;
    }

    // The construction clock, not the wall — and here the cap it feeds is a
    // decision rather than a safety bound. It sits INSIDE the search instead of
    // around a finished build: the iteration it stops is an iteration that might
    // have found a smaller separator, so wherever the cap binds it picks which
    // separator comes back, hence the decomposition assembled from that
    // separator and the vtree read off the decomposition. A cap that chooses the
    // answer has to be measured on the same clock as the budget it was derived
    // from, or the answer moves with how fast and how loaded the machine was.
    let start = crate::decompose::meter::now();
    let has_deadline = timeout_ms > 0;
    let deadline_ms = timeout_ms as u128;

    // Seeded with 0, matching C++ minstd_rand's default; only need deterministic,
    // well-mixed seeds for SmallRng, not bit-exact parity with C++.
    let mut outer_rng = MinstdRand::new(0);

    let mut best: Option<Vec<u32>> = None;
    let mut best_size = i32::MAX;

    let n_orig = g.n as i64;
    let m_arc = (g.tail.len() as i64).max(1);
    let step_cost = (((n_orig as f64).sqrt() * (m_arc as f64).sqrt()) / 50.0).max(1.0) as i64;

    let iter_cap = iters.max(1);
    let mut steps_left = steps;

    // C++ schedule increments cutter_count every 16 iters; we pin it to 1
    // because in differential tests on small graphs MultiCutter's switching
    // rule discards good single-cutter trajectories.
    for i in 0..iter_cap {
        if has_deadline && elapsed_since(start) >= deadline_ms {
            break;
        }
        if steps_left <= 0 {
            break;
        }
        steps_left -= step_cost;
        // ONE cost model for both FlowCutter implementations. An outer iteration
        // here does what an iteration of the vendored restart loop does — a pass
        // over the whole graph — so it is charged the same way, from this
        // graph's own counts: `n_orig` vertices and `m_arc / 2` undirected edges
        // (`m_arc` counts every edge once per direction). There is no second
        // constant and no second model to keep in step.
        let iter_units = super::flowcutter::fc_iter_units(n_orig as u64, (m_arc / 2) as u64);
        crate::decompose::meter::charge(iter_units);

        let min_small_side = match i % 3 {
            2 => 0.2_f32,
            1 => 0.1_f32,
            _ => 0.0_f32,
        };

        let cfg = Config {
            cutter_count: 1,
            random_seed: outer_rng.next() as u64,
            max_cut_size: 10_000,
            min_small_side_size: min_small_side,
        };

        if let Some(sep) = compute_separator_one(&g, &cfg, deadline_ms, start, has_deadline)
            && !sep.is_empty()
            && (sep.len() as i32) < best_size
        {
            best_size = sep.len() as i32;
            best = Some(sep);
        }
    }

    best
}

#[derive(Clone)]
struct Config {
    cutter_count: u32,
    random_seed: u64,
    max_cut_size: i32,
    min_small_side_size: f32,
}

// Ports `flow_cutter::ComputeSeparator`'s node_min_expansion branch only —
// the only one IFlowCutter uses; other branches aren't implemented here.
fn compute_separator_one(
    g: &OrigGraph,
    cfg: &Config,
    deadline_ms: u128,
    start: Instant,
    has_deadline: bool,
) -> Option<Vec<u32>> {
    let n_orig = g.n;
    let a_orig = g.tail.len() as u32;
    let n_exp_v = n_exp(n_orig);
    let a_exp_v = a_exp(n_orig, a_orig);
    let exp = Exp { g, a_orig };

    let pairs = select_random_st_pairs(n_orig, cfg.cutter_count, cfg.random_seed);
    if pairs.is_empty() {
        return None;
    }

    let exp_pairs: Vec<(u32, u32)> = pairs
        .iter()
        .map(|&(s, t)| (orig_node_to_exp(s, false), orig_node_to_exp(t, true)))
        .collect();

    let mut multi = MultiCutter::new(n_exp_v, a_exp_v, exp_pairs.len() as u32);
    multi.init(&exp, a_orig, &exp_pairs);

    let mut best: Option<Vec<u32>> = None;
    let mut best_score = f64::INFINITY;
    let exp_node_count_f = n_exp_v as f64;

    let min_balance_threshold = cfg.min_small_side_size as f64 * exp_node_count_f;

    let mut iter_guard: u32 = 0;
    loop {
        if has_deadline && elapsed_since(start) >= deadline_ms {
            break;
        }
        iter_guard += 1;
        if iter_guard > 10_000_000 {
            break;
        }

        let cut_size = multi.current_cut_size() as f64;
        let small_side = multi.current_smaller_size() as f64;
        let mut score = if small_side > 0.0 {
            cut_size / small_side
        } else {
            f64::INFINITY
        };
        if multi.current_smaller_size() < min_balance_threshold as u32 {
            score += 1_000_000.0;
        }

        if score < best_score {
            best_score = score;
            let sep = extract_original_separator(g, a_orig, &multi);
            if (sep.len() as i32) > cfg.max_cut_size {
                best = Some(sep);
                break;
            }
            best = Some(sep);
        }

        let potential_best_next = (cut_size + 1.0) / (exp_node_count_f / 2.0);
        if potential_best_next >= best_score {
            break;
        }
        if !multi.advance(&exp, a_orig) {
            break;
        }
    }

    best
}

/// Recover the original-space vertex separator (not expanded-space) from the
/// current cut.
fn extract_original_separator(g: &OrigGraph, a_orig: u32, multi: &MultiCutter) -> Vec<u32> {
    let mut sep: Vec<u32> = Vec::new();
    let cur_cut = multi.current_cut();

    for &xy in cur_cut {
        if is_intra(xy, a_orig) {
            sep.push(intra_to_orig_node(xy, a_orig));
        }
    }

    let n_orig = g.n as usize;
    // Expanded-space smaller-side count double-counts each vertex (in + out);
    // `sep` here is single-counted, hence -sep.len() then /2 for vertex count.
    let cur_small = multi.current_smaller_size() as i64;
    let mut left_size = (cur_small - sep.len() as i64) / 2;
    let mut right_size = n_orig as i64 - sep.len() as i64 - left_size;

    let is_orig_left = |x: u32| -> bool {
        // Tests via the OUT node: the expanded graph's "left" (smaller) side
        // holds u_out for u in the original left set, not u_in.
        multi.is_on_smaller_side(orig_node_to_exp(x, true))
    };

    for &xy in cur_cut {
        if !is_intra(xy, a_orig) {
            let lr = inter_to_orig_arc(xy);
            let mut l = g.tail[lr as usize];
            let mut r = g.head[lr as usize];
            if is_orig_left(r) {
                std::mem::swap(&mut l, &mut r);
            }
            if left_size > right_size {
                sep.push(l);
                left_size -= 1;
            } else {
                sep.push(r);
                right_size -= 1;
            }
        }
    }

    sep.sort_unstable();
    sep.dedup();
    sep
}

fn select_random_st_pairs(n: u32, count: u32, seed: u64) -> Vec<(u32, u32)> {
    let mut rng = SmallRng::seed_from_u64(seed);
    let mut out = Vec::with_capacity(count as usize);
    if n < 2 {
        return out;
    }
    for _ in 0..count {
        let mut s;
        let mut t;
        loop {
            s = rng.next_u32() % n;
            t = rng.next_u32() % n;
            if s != t {
                break;
            }
        }
        out.push((s, t));
    }
    out
}

/// LCG params (a=48271, m=2^31-1) match C++ std::minstd_rand.
struct MinstdRand {
    state: u64,
}

impl MinstdRand {
    fn new(seed: u32) -> Self {
        // libstdc++'s linear_congruential_engine treats seed 0 as seed 1;
        // match that so the sequence lines up with minstd_rand.
        let s = if seed == 0 { 1 } else { seed };
        MinstdRand { state: s as u64 }
    }
    fn next(&mut self) -> u32 {
        self.state = (self.state * 48271) % ((1u64 << 31) - 1);
        self.state as u32
    }
}

#[cfg(test)]
mod tests;
