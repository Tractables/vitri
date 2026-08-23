//! Partitions the coarsest level from nothing, the one phase of the sweep with
//! no partition to start from.
//!
//! Two generators, several restarts each: growing side 0 out from a random
//! seed, and random bisections cleaned up by FM. The best hyperedge cut among
//! them is what uncoarsening starts from, so this is also the only place the
//! module picks between whole partitions rather than single moves.

use super::*;
use crate::decompose::fm_common::random_bisection;

/// Grows side 0 outward from `seed` until it holds half the vertex weight;
/// everything else lands on side 1.
///
/// The gain of a candidate is recomputed from `he_count0` on every step rather
/// than updated incrementally, which costs a scan of every unplaced vertex's
/// incidences per step but keeps the score exact. See "Where the two bisectors
/// differ" in [`fm_common`](crate::decompose::fm_common).
pub(super) fn hg_greedy_growing(hg: &Hypergraph, seed: usize) -> Vec<u8> {
    let n = hg.num_vertices;
    let total_weight: u32 = hg.vwgt.iter().sum();
    let target = total_weight / 2;

    let mut part = vec![1u8; n];
    let mut in_set = vec![false; n];
    let num_he = hg.num_hyperedges();
    let mut he_count0: Vec<u32> = vec![0; num_he];

    part[seed] = 0;
    in_set[seed] = true;
    let mut set_weight = hg.vwgt[seed];

    for &hei in hg.vertex_hyperedges(seed) {
        he_count0[hei as usize] += 1;
    }

    while set_weight < target {
        let mut best_v: i32 = -1;
        let mut best_gain: i32 = i32::MIN;

        for (v, &grown) in in_set.iter().enumerate() {
            if grown {
                continue;
            }
            // Only two transitions matter for a hyperedge when `v` joins side
            // 0: it had no pin there and now straddles the split (-w), or it
            // was one pin short of complete and is now wholly inside (+w).
            // Everything in between leaves the cut where it was.
            let mut gain: i32 = 0;
            for &hei in hg.vertex_hyperedges(v) {
                let hei = hei as usize;
                let w = hg.hewgt[hei] as i32;
                let total_pins = hg.he_start[hei + 1] - hg.he_start[hei];
                let count0 = he_count0[hei];
                if count0 == 0 {
                    gain -= w;
                }
                if count0 + 1 == total_pins {
                    gain += w;
                }
            }
            if gain > best_gain {
                best_gain = gain;
                best_v = v as i32;
            }
        }

        if best_v < 0 {
            break;
        }

        let v = best_v as usize;
        part[v] = 0;
        in_set[v] = true;
        set_weight += hg.vwgt[v];

        for &hei in hg.vertex_hyperedges(v) {
            he_count0[hei as usize] += 1;
        }
    }

    part
}

/// Summed weight of the hyperedges with pins on both sides.
///
/// A hyperedge is charged once however its pins are spread, so this is the
/// hyperedge-count metric rather than anything that grows with how badly a
/// clause is split.
pub(super) fn hg_cut(hg: &Hypergraph, part: &[u8]) -> u32 {
    let mut cut = 0u32;
    for hei in 0..hg.num_hyperedges() {
        let pins = hg.hyperedge_pins(hei);
        let first = part[pins[0] as usize];
        if pins.iter().any(|&v| part[v as usize] != first) {
            cut += hg.hewgt[hei];
        }
    }
    cut
}

pub(super) fn hg_initial_partition(
    hg: &Hypergraph,
    rng: &mut Xorshift64,
    imbalance: f64,
) -> Vec<u8> {
    let n = hg.num_vertices;
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![0];
    }

    let mut best_part = Vec::new();
    let mut best_cut = u32::MAX;

    // Restart counts: see "Where the two bisectors differ" in `fm_common`.
    let num_ggg = if n >= 30 { 6 } else { 4 };
    let num_rand = if n >= 30 { 6 } else { 4 };

    for _ in 0..num_ggg.min(n) {
        let seed = (rng.next_u64() as usize) % n;
        let part = hg_greedy_growing(hg, seed);
        let cut = hg_cut(hg, &part);
        if cut < best_cut {
            best_cut = cut;
            best_part = part;
        }
    }

    // Random starts get an FM pass before being scored; grown ones are scored
    // as produced.
    for _ in 0..num_rand.min(n) {
        let mut part = random_bisection(&hg.vwgt, rng);
        hg_fm_refine(hg, &mut part, imbalance);
        let cut = hg_cut(hg, &part);
        if cut < best_cut {
            best_cut = cut;
            best_part = part;
        }
    }

    best_part
}
