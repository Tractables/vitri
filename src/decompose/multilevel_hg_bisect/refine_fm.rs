//! Fiduccia-Mattheyses refinement of an existing partition: bucket-queue global
//! passes at every level, plus localized passes seeded on the boundary at the
//! finest one.
//!
//! A pass moves vertices one at a time and never moves the same vertex twice,
//! then keeps the best *prefix* of that move sequence and rolls the rest back.
//! The rollback is the whole point: a run of negative-gain moves is allowed to
//! happen, and survives only if something better follows, which is how FM
//! climbs out of a local minimum that single-move hill-climbing sits in.
//!
//! Hypergraph gains hinge on two pin counts per hyperedge rather than on a
//! single edge's endpoints, so `he_count` is maintained alongside the partition
//! and every rule below is a statement about a count reaching 0, 1, or 2.

use super::*;
use crate::decompose::fm_common::{FmBalance, GainBuckets, Stall, commit_best_prefix, fm_balance};

pub(super) fn hg_fm_refine_pass(hg: &Hypergraph, part: &mut [u8], max_imbalance: f64) -> bool {
    let n = hg.num_vertices;
    let Some(FmBalance {
        mut weight,
        min_part_weight,
        max_part_weight,
    }) = fm_balance(n, &hg.vwgt, part, max_imbalance)
    else {
        return false;
    };

    let mut he_count = hg.pin_counts(part);

    // A vertex can gain at most the weight of everything it is a pin of, which
    // is the widest the bucket range ever has to be.
    let mut max_gain: i32 = 1;
    for v in 0..n {
        let mut wsum: i32 = 0;
        for &hei in hg.vertex_hyperedges(v) {
            wsum += hg.hewgt[hei as usize] as i32;
        }
        if wsum > max_gain {
            max_gain = wsum;
        }
    }

    let mut gain: Vec<i32> = vec![0; n];
    let mut bq = [GainBuckets::new(n, max_gain), GainBuckets::new(n, max_gain)];

    for v in 0..n {
        let from = part[v] as usize;
        let to = 1 - from;
        // Gain of moving `v`: a hyperedge it is the last pin of on its own side
        // stops being cut (+w), and one with no pin yet on the far side starts
        // being cut (-w). A hyperedge with company on both sides is cut either
        // way and contributes nothing. Only vertices on a cut hyperedge are
        // queued — the rest can only add to the cut.
        let mut g: i32 = 0;
        let mut on_boundary = false;
        for &hei in hg.vertex_hyperedges(v) {
            let hei = hei as usize;
            let w = hg.hewgt[hei] as i32;
            if he_count[hei][from] == 1 {
                g += w;
            }
            if he_count[hei][to] == 0 {
                g -= w;
            }
            if he_count[hei][0] > 0 && he_count[hei][1] > 0 {
                on_boundary = true;
            }
        }
        gain[v] = g;
        if on_boundary {
            bq[from].insert(v, g);
        }
    }

    let mut locked = vec![false; n];
    let mut moves: Vec<(usize, i32)> = Vec::with_capacity(n);
    let mut cumulative_gain: Vec<i64> = Vec::with_capacity(n);
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
                let new_from_wt = weight[side] - hg.vwgt[top_v];
                let new_to_wt = weight[to] + hg.vwgt[top_v];
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
        weight[from] -= hg.vwgt[v];
        weight[to] += hg.vwgt[v];
        part[v] = to as u8;
        locked[v] = true;

        running_gain += best_gain as i64;
        moves.push((v, best_gain));
        cumulative_gain.push(running_gain);

        if stall.record(running_gain) {
            break;
        }

        // Incremental gain updates: O(hyperedge_size) per hyperedge instead of
        // O(hyperedge_size × avg_degree) for full recomputation.
        for &hei in hg.vertex_hyperedges(v) {
            let hei = hei as usize;
            let old_from = he_count[hei][from];
            let old_to = he_count[hei][to];

            he_count[hei][from] -= 1;
            he_count[hei][to] += 1;

            let new_from = old_from - 1;
            let _new_to = old_to + 1;

            for &u in hg.hyperedge_pins(hei) {
                let u = u as usize;
                if locked[u] {
                    continue;
                }

                // Four transitions, each a pin count crossing a critical
                // value. For `u` still on the side `v` left: `old_from == 2`
                // means `u` is now the last pin holding the hyperedge on that
                // side, so moving `u` would close it (+w); `old_to == 0` means
                // `v` just opened the far side, so `u` following no longer
                // opens it (+w). For `u` on the side `v` joined: `old_to == 1`
                // means `u` was the lone pin there and no longer is (-w);
                // `new_from == 0` means `v` was the last pin on the far side,
                // so `u` going back would reopen it (-w). The mirrored tests
                // are absent because each would require a side to hold no pins
                // while `v` or `u` is sitting on it.
                let u_side = part[u] as usize;
                let w = hg.hewgt[hei] as i32;
                let mut delta: i32 = 0;

                if u_side == from {
                    if old_from == 2 {
                        delta += w;
                    }
                    if old_to == 0 {
                        delta += w;
                    }
                } else {
                    if old_to == 1 {
                        delta -= w;
                    }
                    if new_from == 0 {
                        delta -= w;
                    }
                }

                if delta != 0 {
                    gain[u] += delta;
                }

                // Queue membership is per vertex, not per hyperedge: `u` leaves
                // only when none of its hyperedges is cut any more, so the
                // rescan below runs just on the step where this hyperedge
                // changed cut state.
                let he_is_cut = he_count[hei][0] > 0 && he_count[hei][1] > 0;
                let was_in_queue = bq[u_side].contains(u);
                let was_cut = old_from > 0 && old_to > 0;

                if he_is_cut != was_cut {
                    let on_boundary = if he_is_cut {
                        true
                    } else {
                        hg.vertex_hyperedges(u).iter().any(|&hej| {
                            let hej = hej as usize;
                            he_count[hej][0] > 0 && he_count[hej][1] > 0
                        })
                    };
                    if on_boundary {
                        if was_in_queue {
                            bq[u_side].update(u, gain[u]);
                        } else {
                            bq[u_side].insert(u, gain[u]);
                        }
                    } else if was_in_queue {
                        bq[u_side].remove(u);
                    }
                } else if was_in_queue && delta != 0 {
                    bq[u_side].update(u, gain[u]);
                }
            }
        }
    }

    commit_best_prefix(&moves, &cumulative_gain, part)
}

/// Returns true if the partition was improved.
///
/// FM confined to a region grown around `seed`, run at the finest level after
/// the global passes have stopped improving. The region is capped, so selection
/// is a linear scan over its vertices rather than a bucket queue, and a move
/// updates only region pins; a vertex outside the region is never a candidate,
/// so its gain entry stays at zero. On `false` the partition is restored to
/// exactly what was passed in.
pub(super) fn hg_localized_fm_pass(
    hg: &Hypergraph,
    part: &mut [u8],
    seed: usize,
    max_imbalance: f64,
) -> bool {
    let n = hg.num_vertices;
    let Some(FmBalance {
        mut weight,
        min_part_weight,
        max_part_weight,
    }) = fm_balance(n, &hg.vwgt, part, max_imbalance)
    else {
        return false;
    };

    let mut he_count = hg.pin_counts(part);

    let max_region = (n / 4).max(20).min(n);
    let mut in_region = vec![false; n];
    let mut region_queue = std::collections::VecDeque::new();
    in_region[seed] = true;
    region_queue.push_back(seed);
    let mut region_size = 1usize;

    // Growing along cut hyperedges only, and taking every pin of one, picks up
    // both sides of the cut in a single rule — a hyperedge is only cut if it
    // has pins on both. The graph sibling needs a second sweep to reach the
    // same-side vertices FM wants room to move.
    while let Some(v) = region_queue.pop_front() {
        if region_size >= max_region {
            break;
        }
        for &hei in hg.vertex_hyperedges(v) {
            let hei_idx = hei as usize;
            if he_count[hei_idx][0] == 0 || he_count[hei_idx][1] == 0 {
                continue;
            }
            for &u in hg.hyperedge_pins(hei_idx) {
                let u = u as usize;
                if !in_region[u] && region_size < max_region {
                    in_region[u] = true;
                    region_queue.push_back(u);
                    region_size += 1;
                }
            }
        }
    }

    let mut gain: Vec<i32> = vec![0; n];
    for v in 0..n {
        if !in_region[v] {
            continue;
        }
        let from = part[v] as usize;
        let to = 1 - from;
        for &hei in hg.vertex_hyperedges(v) {
            let hei = hei as usize;
            let w = hg.hewgt[hei] as i32;
            if he_count[hei][from] == 1 {
                gain[v] += w;
            }
            if he_count[hei][to] == 0 {
                gain[v] -= w;
            }
        }
    }

    let mut locked = vec![false; n];
    let mut moves: Vec<(usize, i32)> = Vec::new();
    let mut cumulative_gain: Vec<i64> = Vec::new();
    let mut running_gain: i64 = 0;
    let mut stall = Stall::new(region_size / 2);

    // O(region²), not O(region·n): region_list, not 0..n, is scanned per move.
    // Ascending index order (not BFS discovery order) preserves the lowest-index
    // tie-break: `gain[v] > best_gain` keeps the first vertex reaching the max.
    let region_list: Vec<usize> = (0..n).filter(|&v| in_region[v]).collect();

    for _ in 0..region_size {
        let mut best_v: i32 = -1;
        let mut best_gain: i32 = i32::MIN;
        for &v in &region_list {
            if locked[v] {
                continue;
            }
            let from = part[v] as usize;
            let to = 1 - from;
            let nfw = weight[from] - hg.vwgt[v];
            let ntw = weight[to] + hg.vwgt[v];
            if nfw < min_part_weight || ntw > max_part_weight {
                continue;
            }
            if gain[v] > best_gain {
                best_gain = gain[v];
                best_v = v as i32;
            }
        }
        if best_v < 0 {
            break;
        }

        let v = best_v as usize;
        let from = part[v] as usize;
        let to = 1 - from;

        weight[from] -= hg.vwgt[v];
        weight[to] += hg.vwgt[v];
        part[v] = to as u8;
        locked[v] = true;

        running_gain += best_gain as i64;
        moves.push((v, best_gain));
        cumulative_gain.push(running_gain);

        if stall.record(running_gain) {
            break;
        }

        for &hei in hg.vertex_hyperedges(v) {
            let hei = hei as usize;
            let w = hg.hewgt[hei] as i32;
            let old_from = he_count[hei][from];
            let old_to = he_count[hei][to];
            he_count[hei][from] -= 1;
            he_count[hei][to] += 1;
            let new_from = old_from - 1;

            // The same four transitions as `hg_fm_refine_pass`, applied to
            // region pins only.
            for &u in hg.hyperedge_pins(hei) {
                let u = u as usize;
                if locked[u] || !in_region[u] {
                    continue;
                }
                let u_side = part[u] as usize;
                let mut delta: i32 = 0;
                if u_side == from {
                    if old_from == 2 {
                        delta += w;
                    }
                    if old_to == 0 {
                        delta += w;
                    }
                } else {
                    if old_to == 1 {
                        delta -= w;
                    }
                    if new_from == 0 {
                        delta -= w;
                    }
                }
                if delta != 0 {
                    gain[u] += delta;
                }
            }
        }
    }

    commit_best_prefix(&moves, &cumulative_gain, part)
}

/// Standard FM refinement (global passes only).
///
/// Passes repeat until one fails to improve. The cap bounds the case where each
/// pass finds a single-move improvement and would otherwise keep going.
pub(super) fn hg_fm_refine(hg: &Hypergraph, part: &mut [u8], imbalance: f64) {
    let max_passes = 10;
    for _ in 0..max_passes {
        if !hg_fm_refine_pass(hg, part, imbalance) {
            break;
        }
    }
}
