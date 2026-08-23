//! Fiduccia-Mattheyses refinement of an existing partition: bucket-queue global
//! passes at every level, plus localized passes seeded on the boundary at the
//! finest one.
//!
//! A pass moves vertices one at a time and never moves the same vertex twice,
//! then keeps the best *prefix* of that move sequence and rolls the rest back.
//! The rollback is the whole point: a run of negative-gain moves is allowed to
//! happen, and survives only if something better follows, which is how FM
//! climbs out of a local minimum that single-move hill-climbing sits in.

use super::*;
use crate::decompose::fm_common::{FmBalance, GainBuckets, Stall, commit_best_prefix, fm_balance};

pub(super) struct FmScratch {
    gain: Vec<i32>,
    cut_edges: Vec<i32>,
    locked: Vec<bool>,
    moves: Vec<(usize, i32)>,
    cumulative_gain: Vec<i64>,
    bq: [GainBuckets; 2],
}

impl FmScratch {
    pub(super) fn new() -> Self {
        FmScratch {
            gain: Vec::new(),
            cut_edges: Vec::new(),
            locked: Vec::new(),
            moves: Vec::new(),
            cumulative_gain: Vec::new(),
            bq: [GainBuckets::empty(), GainBuckets::empty()],
        }
    }

    fn prepare(&mut self, n: usize, max_gain: i32) {
        self.gain.clear();
        self.gain.resize(n, 0);
        self.cut_edges.clear();
        self.cut_edges.resize(n, 0);
        self.locked.clear();
        self.locked.resize(n, false);
        self.moves.clear();
        self.cumulative_gain.clear();
        self.bq[0].reset(n, max_gain);
        self.bq[1].reset(n, max_gain);
    }
}

/// Returns true if the partition was improved.
///
/// On `false` the partition is restored to exactly what was passed in, so a
/// caller can loop on this until it stops paying off without keeping a copy.
pub(super) fn fm_refine_pass(
    graph: &CsrGraph,
    part: &mut [u8],
    max_imbalance: f64,
    scratch: &mut FmScratch,
) -> bool {
    let n = graph.num_vertices();
    let Some(FmBalance {
        mut weight,
        min_part_weight,
        max_part_weight,
    }) = fm_balance(n, &graph.vwgt, part, max_imbalance)
    else {
        return false;
    };

    // max_gain must be known before scratch.prepare() sizes the bucket
    // queues, so gains/cut_edges are computed in a second pass, into scratch.
    let mut max_gain: i32 = 1;
    for v in 0..n {
        let start = graph.xadj[v] as usize;
        let end = graph.xadj[v + 1] as usize;
        let wgts = &graph.adjwgt[start..end];
        let mut total: i32 = 0;
        for &w in wgts {
            total += w as i32;
        }
        if total > max_gain {
            max_gain = total;
        }
    }

    scratch.prepare(n, max_gain);
    let gain = scratch.gain.as_mut_slice();
    let cut_edges = scratch.cut_edges.as_mut_slice();
    for v in 0..n {
        let my_part = part[v];
        let start = graph.xadj[v] as usize;
        let end = graph.xadj[v + 1] as usize;
        let nbrs = &graph.adjncy[start..end];
        let wgts = &graph.adjwgt[start..end];
        let mut g: i32 = 0;
        let mut cut: i32 = 0;
        for (&nb, &w) in nbrs.iter().zip(wgts) {
            let w = w as i32;
            if part[nb as usize] != my_part {
                g += w;
                cut += w;
            } else {
                g -= w;
            }
        }
        gain[v] = g;
        cut_edges[v] = cut;
    }

    // Only boundary vertices are queued: an interior vertex has every edge on
    // its own side, so moving it can only add to the cut.
    let bq = &mut scratch.bq;
    for v in 0..n {
        if cut_edges[v] > 0 {
            bq[part[v] as usize].insert(v, gain[v]);
        }
    }

    let locked = scratch.locked.as_mut_slice();
    let moves = &mut scratch.moves;
    let cumulative_gain = &mut scratch.cumulative_gain;
    let mut running_gain: i64 = 0;
    let mut stall = Stall::new((n / 2).max(20));

    for _ in 0..n {
        let mut best_v: Option<usize> = None;
        let mut best_gain: i32 = i32::MIN;
        let mut best_from: usize = 0;

        for side in 0..2 {
            while let Some(top_v) = bq[side].top() {
                if locked[top_v] {
                    bq[side].remove(top_v);
                    continue;
                }
                let to = 1 - side;
                let new_from_wt = weight[side] - graph.vwgt[top_v];
                let new_to_wt = weight[to] + graph.vwgt[top_v];
                if new_from_wt < min_part_weight || new_to_wt > max_part_weight {
                    // Evicted, not skipped: side weights keep changing as the
                    // pass runs, so a vertex rejected on balance here can become
                    // movable later, but it only comes back if a gain update
                    // re-inserts it.
                    bq[side].remove(top_v);
                    continue;
                }
                let g = gain[top_v];
                if g > best_gain {
                    best_gain = g;
                    best_v = Some(top_v);
                    best_from = side;
                }
                break;
            }
        }

        let v = match best_v {
            Some(v) => v,
            None => break,
        };

        let from = best_from;
        let to = 1 - from;

        bq[from].remove(v);
        weight[from] -= graph.vwgt[v];
        weight[to] += graph.vwgt[v];
        part[v] = to as u8;
        locked[v] = true;

        running_gain += best_gain as i64;
        moves.push((v, best_gain));
        cumulative_gain.push(running_gain);

        if stall.record(running_gain) {
            break;
        }

        let v_start = graph.xadj[v] as usize;
        let v_end = graph.xadj[v + 1] as usize;
        let v_nbrs = &graph.adjncy[v_start..v_end];
        let v_wgts = &graph.adjwgt[v_start..v_end];
        for (&nb_raw, &w_raw) in v_nbrs.iter().zip(v_wgts) {
            let nb = nb_raw as usize;
            if locked[nb] {
                continue;
            }
            let w = w_raw as i32;
            let nb_part = part[nb] as usize;
            let was_in_queue = bq[nb_part].contains(nb);
            // `2 * w`, not `w`: `gain` is external weight minus internal
            // weight, and this edge crossed from one of those sums to the
            // other, so their difference moves by twice the edge's weight.
            // `cut_edges` is the external sum on its own, so it moves by `w`.
            if nb_part == to {
                gain[nb] -= 2 * w;
                cut_edges[nb] -= w;
            } else {
                gain[nb] += 2 * w;
                cut_edges[nb] += w;
            }
            let on_boundary = cut_edges[nb] > 0;
            if on_boundary {
                if was_in_queue {
                    bq[nb_part].update(nb, gain[nb]);
                } else {
                    bq[nb_part].insert(nb, gain[nb]);
                }
            } else if was_in_queue {
                bq[nb_part].remove(nb);
            }
        }
    }

    commit_best_prefix(moves, cumulative_gain, part)
}

/// FM confined to a region grown around `seed`, run at the finest level after
/// the global passes have stopped improving.
///
/// The region is capped, so selection is a linear scan over its vertices rather
/// than a bucket queue. Gains exist only for region vertices and a move updates
/// only region neighbours; a vertex outside the region is never a candidate, so
/// its gain entry stays at zero.
pub(super) fn localized_fm_pass(
    graph: &CsrGraph,
    part: &mut [u8],
    seed: usize,
    max_imbalance: f64,
) -> bool {
    let n = graph.num_vertices();
    let Some(FmBalance {
        mut weight,
        min_part_weight,
        max_part_weight,
    }) = fm_balance(n, &graph.vwgt, part, max_imbalance)
    else {
        return false;
    };

    // region_list is collected alongside in_region to avoid a separate O(n)
    // scan in the selection loop below.
    let max_region = (n / 4).max(20).min(n);
    let mut in_region = vec![false; n];
    let mut region_list: Vec<usize> = Vec::with_capacity(max_region);
    let mut queue = std::collections::VecDeque::new();
    in_region[seed] = true;
    queue.push_back(seed);
    region_list.push(seed);

    // Cross-partition neighbours first, so the region hugs the cut even when
    // the cap cuts the growth short.
    while let Some(v) = queue.pop_front() {
        if region_list.len() >= max_region {
            break;
        }
        for &nb in graph.neighbors(v) {
            let nb = nb as usize;
            if !in_region[nb] && part[nb] != part[v] && region_list.len() < max_region {
                in_region[nb] = true;
                queue.push_back(nb);
                region_list.push(nb);
            }
        }
        // Region also absorbs same-side neighbors of boundary vertices, not
        // just cross-partition ones — gives FM room to move on both sides.
        for &nb in graph.neighbors(v) {
            let nb = nb as usize;
            if !in_region[nb] && region_list.len() < max_region {
                let nb_part = part[nb];
                let is_boundary = graph
                    .neighbors(nb)
                    .iter()
                    .any(|&nnb| part[nnb as usize] != nb_part);
                if is_boundary {
                    in_region[nb] = true;
                    queue.push_back(nb);
                    region_list.push(nb);
                }
            }
        }
    }

    // Gains only for region vertices: O(region × deg), not O(n × deg).
    let mut gain: Vec<i32> = vec![0; n];
    for &v in &region_list {
        let my_part = part[v];
        let start = graph.xadj[v] as usize;
        let end = graph.xadj[v + 1] as usize;
        let nbrs = &graph.adjncy[start..end];
        let wgts = &graph.adjwgt[start..end];
        let mut g: i32 = 0;
        for (&nb, &w) in nbrs.iter().zip(wgts) {
            let w = w as i32;
            if part[nb as usize] != my_part {
                g += w;
            } else {
                g -= w;
            }
        }
        gain[v] = g;
    }

    let mut locked = vec![false; n];
    let mut moves: Vec<(usize, i32)> = Vec::new();
    let mut cumulative_gain: Vec<i64> = Vec::new();
    let mut running_gain: i64 = 0;
    let mut stall = Stall::new(region_list.len() / 2);

    // O(region²), not O(n × region): only region_list is scanned per move.
    // Ties go to whichever vertex BFS reached first, since `gain[v] > best_g`
    // is strict; the hypergraph sibling scans its region in ascending index
    // order instead, and nothing in the tree records why the two differ.
    for _ in 0..region_list.len() {
        let mut best_v: i32 = -1;
        let mut best_g: i32 = i32::MIN;
        for &v in &region_list {
            if locked[v] {
                continue;
            }
            let from = part[v] as usize;
            let to = 1 - from;
            let nfw = weight[from] - graph.vwgt[v];
            let ntw = weight[to] + graph.vwgt[v];
            if nfw < min_part_weight || ntw > max_part_weight {
                continue;
            }
            if gain[v] > best_g {
                best_g = gain[v];
                best_v = v as i32;
            }
        }
        if best_v < 0 {
            break;
        }

        let v = best_v as usize;
        let from = part[v] as usize;
        let to = 1 - from;
        weight[from] -= graph.vwgt[v];
        weight[to] += graph.vwgt[v];
        part[v] = to as u8;
        locked[v] = true;

        running_gain += best_g as i64;
        moves.push((v, best_g));
        cumulative_gain.push(running_gain);

        if stall.record(running_gain) {
            break;
        }

        let v_start = graph.xadj[v] as usize;
        let v_end = graph.xadj[v + 1] as usize;
        let v_nbrs = &graph.adjncy[v_start..v_end];
        let v_wgts = &graph.adjwgt[v_start..v_end];
        for (&nb_raw, &w_raw) in v_nbrs.iter().zip(v_wgts) {
            let nb = nb_raw as usize;
            if locked[nb] || !in_region[nb] {
                continue;
            }
            let w = w_raw as i32;
            if part[nb] == to as u8 {
                gain[nb] -= 2 * w;
            } else {
                gain[nb] += 2 * w;
            }
        }
    }

    commit_best_prefix(&moves, &cumulative_gain, part)
}

/// Standard FM refinement (global passes only).
///
/// Passes repeat until one fails to improve. The cap bounds the case where each
/// pass finds a single-move improvement and would otherwise keep going.
pub(super) fn fm_refine(
    graph: &CsrGraph,
    part: &mut [u8],
    max_imbalance: f64,
    scratch: &mut FmScratch,
) {
    let max_passes = 10;
    for _ in 0..max_passes {
        if !fm_refine_pass(graph, part, max_imbalance, scratch) {
            break;
        }
    }
}

/// FM refinement with multi-try localized passes after global FM.
pub(super) fn fm_refine_with_local(
    graph: &CsrGraph,
    part: &mut [u8],
    max_imbalance: f64,
    scratch: &mut FmScratch,
) {
    fm_refine(graph, part, max_imbalance, scratch);

    let n = graph.num_vertices();
    if n < 20 {
        return;
    }
    let num_tries = 4.min(n);
    let mut boundary: Vec<usize> = Vec::new();
    for v in 0..n {
        let my_part = part[v];
        if graph
            .neighbors(v)
            .iter()
            .any(|&nb| part[nb as usize] != my_part)
        {
            boundary.push(v);
        }
    }
    if boundary.is_empty() {
        return;
    }

    // 7919 is prime, so successive tries land in unrelated stretches of the
    // boundary list rather than in one region's worth of adjacent vertices.
    for i in 0..num_tries {
        let seed = boundary[(i * 7919) % boundary.len()];
        localized_fm_pass(graph, part, seed, max_imbalance);
    }
}
