//! Partitions the coarsest level from nothing, the one phase of the sweep with
//! no partition to start from.
//!
//! Two generators, several restarts each: graph growing from a random seed, and
//! random bisections cleaned up by FM. The best edge cut among them is what
//! uncoarsening starts from, so this is also the only place the module picks
//! between whole partitions rather than single moves.

use super::*;
use crate::decompose::fm_common::random_bisection;

/// Grows side 0 outward from `seed` until it holds half the vertex weight;
/// everything else lands on side 1.
pub(super) fn greedy_graph_growing(graph: &CsrGraph, seed: usize) -> Vec<u8> {
    let n = graph.num_vertices();
    let total_weight: u32 = graph.vwgt.iter().sum();
    let target = total_weight / 2;

    let mut part = vec![1u8; n];
    let mut in_set = vec![false; n];
    let mut gain: Vec<i64> = vec![0; n];

    part[seed] = 0;
    in_set[seed] = true;
    let mut set_weight = graph.vwgt[seed];

    let s_start = graph.xadj[seed] as usize;
    let s_end = graph.xadj[seed + 1] as usize;
    for (&nb, &w) in graph.adjncy[s_start..s_end]
        .iter()
        .zip(&graph.adjwgt[s_start..s_end])
    {
        gain[nb as usize] += w as i64;
    }

    while set_weight < target {
        let mut best_v: i32 = -1;
        let mut best_gain: i64 = i64::MIN;
        for v in 0..n {
            if !in_set[v] && gain[v] > best_gain {
                best_gain = gain[v];
                best_v = v as i32;
            }
        }

        if best_v < 0 {
            break;
        }

        let v = best_v as usize;
        part[v] = 0;
        in_set[v] = true;
        set_weight += graph.vwgt[v];

        let start = graph.xadj[v] as usize;
        let end = graph.xadj[v + 1] as usize;
        for (&nb, &w) in graph.adjncy[start..end]
            .iter()
            .zip(&graph.adjwgt[start..end])
        {
            let nb = nb as usize;
            if !in_set[nb] {
                // `2 * w`: the edge leaves the not-in-set sum and enters the
                // in-set sum, so the difference between them moves by twice its
                // weight. The seed's own edges above were added at `w`, so an
                // edge to the seed counts half of what every later edge does,
                // and a candidate's edges to vertices still outside the set are
                // never subtracted at all — the score ranks candidates but is
                // not the cut reduction the textbook version tracks. See "Where
                // the two bisectors differ" in `fm_common`.
                gain[nb] += 2 * w as i64;
            }
        }
    }

    part
}

/// Summed weight of the cut edges, each counted once: the scan visits only
/// side-0 vertices, so an edge is reached from its side-0 endpoint alone.
pub(super) fn edge_cut(graph: &CsrGraph, part: &[u8]) -> u64 {
    // A full pass, charged as one: the scan skips side-1 vertices, so what it
    // touches is bounded by a pass rather than equal to it.
    crate::decompose::meter::charge(graph.pass_units());
    let mut cut: u64 = 0;
    for v in 0..graph.num_vertices() {
        if part[v] == 0 {
            let start = graph.xadj[v] as usize;
            let end = graph.xadj[v + 1] as usize;
            for (&nb, &w) in graph.adjncy[start..end]
                .iter()
                .zip(&graph.adjwgt[start..end])
            {
                if part[nb as usize] != 0 {
                    cut += w as u64;
                }
            }
        }
    }
    cut
}

pub(super) fn initial_partition(
    graph: &CsrGraph,
    rng: &mut Xorshift64,
    max_imbalance: f64,
    scratch: &mut FmScratch,
) -> Vec<u8> {
    let n = graph.num_vertices();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![0];
    }

    let mut best_part = Vec::new();
    let mut best_cut = u64::MAX;

    // Restart count: see "Where the two bisectors differ" in `fm_common`.
    for _ in 0..4.min(n) {
        let seed = (rng.next_u64() as usize) % n;
        let part = greedy_graph_growing(graph, seed);
        let cut = edge_cut(graph, &part);
        if cut < best_cut {
            best_cut = cut;
            best_part = part;
        }
    }

    // Random starts get an FM pass before being scored; grown ones are scored
    // as produced.
    for _ in 0..4.min(n) {
        let mut part = random_bisection(&graph.vwgt, rng);
        fm_refine(graph, &mut part, max_imbalance, scratch);
        let cut = edge_cut(graph, &part);
        if cut < best_cut {
            best_cut = cut;
            best_part = part;
        }
    }

    best_part
}
