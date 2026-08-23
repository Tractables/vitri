//! Flow-based refinement (a simplified form of Heuer et al., JEA 2018): a
//! max-flow between the two sides of a corridor around the current cut finds
//! the best cut across that whole region at once, complementing the local
//! move-based FM pass. FM commits one vertex at a time and can only see the
//! gain of that one move; a min cut settles every boundary vertex together, so
//! it reaches partitions no single-move sequence with a non-negative prefix
//! would arrive at.
//!
//! This phase has no counterpart on the graph side, and runs only at the finest
//! level, from `hg_fm_refine_with_local` below.

use super::*;
use crate::decompose::fm_common::balance_bounds;

/// Max flow from `source` to `sink`, leaving `capacity` as the residual
/// capacities and `visited` as the source side of the resulting min cut.
///
/// `adj` and `capacity` must be built in pairs, forward edge then its reverse,
/// because the augmenting step finds a reverse edge as `ei ^ 1` — `add_edge` in
/// `hg_flow_refine` is what maintains that. Augmenting paths are found
/// breadth-first, which is what bounds the iteration count independently of the
/// capacity values.
pub(super) fn edmonds_karp(
    num_nodes: usize,
    adj: &[Vec<(usize, usize)>], // adj[u] = [(v, edge_idx), ...]
    capacity: &mut [i32],        // capacity[edge_idx] (residual)
    source: usize,
    sink: usize,
    visited: &mut [bool], // output: reachable from source in residual graph
) -> i32 {
    let mut total_flow: i32 = 0;
    let mut parent = vec![(usize::MAX, usize::MAX); num_nodes]; // (prev_node, edge_idx)

    loop {
        parent.fill((usize::MAX, usize::MAX));
        parent[source] = (source, usize::MAX);
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(source);

        while let Some(u) = queue.pop_front() {
            if u == sink {
                break;
            }
            for &(v, ei) in &adj[u] {
                if parent[v].0 == usize::MAX && capacity[ei] > 0 {
                    parent[v] = (u, ei);
                    queue.push_back(v);
                }
            }
        }

        if parent[sink].0 == usize::MAX {
            break;
        }

        let mut flow = i32::MAX;
        let mut v = sink;
        while v != source {
            let (u, ei) = parent[v];
            flow = flow.min(capacity[ei]);
            v = u;
        }

        v = sink;
        while v != source {
            let (_u, ei) = parent[v];
            capacity[ei] -= flow;
            capacity[ei ^ 1] += flow; // reverse edge
            v = parent[v].0;
        }

        total_flow += flow;
    }

    visited.fill(false);
    visited[source] = true;
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(source);
    while let Some(u) = queue.pop_front() {
        for &(v, ei) in &adj[u] {
            if !visited[v] && capacity[ei] > 0 {
                visited[v] = true;
                queue.push_back(v);
            }
        }
    }

    total_flow
}

/// Models cut hyperedges as flow-network nodes; the min-cut relocates
/// boundary vertices to improve the partition.
///
/// The min cut prices two different things in one number: cutting a
/// vertex-to-terminal edge costs `vwgt` and cutting a hyperedge edge costs
/// `hewgt`. The two are not commensurate, so the flow optimum is not the
/// hyperedge-cut optimum, and the result is adopted only where `hg_cut`
/// actually drops. Returns whether it did.
pub(super) fn hg_flow_refine(hg: &Hypergraph, part: &mut [u8], max_imbalance: f64) -> bool {
    let n = hg.num_vertices;
    if n < 10 {
        return false;
    }

    let (min_part_weight, max_part_weight) = balance_bounds(&hg.vwgt, max_imbalance);

    let num_he = hg.num_hyperedges();
    let he_count = hg.pin_counts(part);

    // Corridor: every pin of every cut hyperedge. Interior vertices are left
    // out of the network entirely, which is what keeps it small enough for a
    // dense max-flow to be worth running.
    let mut is_boundary = vec![false; n];
    let mut cut_hes: Vec<usize> = Vec::new();
    for (hei, counts) in he_count.iter().enumerate() {
        if counts[0] > 0 && counts[1] > 0 {
            cut_hes.push(hei);
            for &v in hg.hyperedge_pins(hei) {
                is_boundary[v as usize] = true;
            }
        }
    }

    if cut_hes.is_empty() {
        return false;
    }

    let boundary_count = is_boundary.iter().filter(|&&b| b).count();
    if boundary_count > 500 {
        // Tuned cap: max-flow cost grows with corridor size, so large
        // boundary regions skip flow refinement rather than pay for it.
        return false;
    }

    // Flow-network node-ID layout: source (0), sink (1), boundary vertices
    // (2..2+boundary_count), cut hyperedge nodes (2+boundary_count..).
    let mut vert_to_flow: Vec<usize> = vec![usize::MAX; n];
    let mut flow_verts: Vec<usize> = Vec::new(); // flow node → original vertex
    let mut next_node = 2usize;
    for v in 0..n {
        if is_boundary[v] {
            vert_to_flow[v] = next_node;
            flow_verts.push(v);
            next_node += 1;
        }
    }
    let he_node_start = next_node;
    let mut he_to_flow: Vec<usize> = vec![usize::MAX; num_he];
    for (i, &hei) in cut_hes.iter().enumerate() {
        he_to_flow[hei] = he_node_start + i;
    }
    let total_nodes = he_node_start + cut_hes.len();
    let source = 0;
    let sink = 1;

    let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); total_nodes];
    let mut capacity: Vec<i32> = Vec::new();

    let add_edge =
        |adj: &mut Vec<Vec<(usize, usize)>>, cap: &mut Vec<i32>, u: usize, v: usize, c: i32| {
            let ei = cap.len();
            adj[u].push((v, ei));
            cap.push(c);
            adj[v].push((u, ei + 1));
            cap.push(0); // reverse edge
        };

    // Source feeds partition-0 boundary vertices; partition-1 ones feed sink.
    // A vertex therefore ends on side 0 exactly when the residual graph still
    // reaches it from the source, and cutting its terminal edge is what the
    // network charges for relocating it.
    for &v in &flow_verts {
        let fv = vert_to_flow[v];
        if part[v] == 0 {
            add_edge(&mut adj, &mut capacity, source, fv, hg.vwgt[v] as i32);
        } else {
            add_edge(&mut adj, &mut capacity, fv, sink, hg.vwgt[v] as i32);
        }
    }

    // Boundary-vertex-to-cut-hyperedge capacity equals the hyperedge weight,
    // so the min-cut cost matches the hg cut it approximates.
    for &hei in &cut_hes {
        let he_node = he_to_flow[hei];
        let w = hg.hewgt[hei] as i32;
        for &v in hg.hyperedge_pins(hei) {
            let v = v as usize;
            if !is_boundary[v] {
                continue;
            }
            let fv = vert_to_flow[v];
            if part[v] == 0 {
                add_edge(&mut adj, &mut capacity, fv, he_node, w);
            } else {
                add_edge(&mut adj, &mut capacity, he_node, fv, w);
            }
        }
    }

    let mut visited = vec![false; total_nodes];
    let _flow = edmonds_karp(total_nodes, &adj, &mut capacity, source, sink, &mut visited);

    let mut new_part = part.to_vec();
    let mut weight = [0u32; 2];
    for v in 0..n {
        weight[part[v] as usize] += hg.vwgt[v];
    }

    let mut changed = false;
    for &v in &flow_verts {
        let fv = vert_to_flow[v];
        let new_side = if visited[fv] { 0u8 } else { 1u8 };
        if new_side != part[v] {
            let from = part[v] as usize;
            let to = new_side as usize;
            let nfw = weight[from] - hg.vwgt[v];
            let ntw = weight[to] + hg.vwgt[v];
            if nfw >= min_part_weight && ntw <= max_part_weight {
                new_part[v] = new_side;
                weight[from] = nfw;
                weight[to] = ntw;
                changed = true;
            }
        }
    }

    if changed {
        let old_cut = hg_cut(hg, part);
        let new_cut = hg_cut(hg, &new_part);
        if new_cut < old_cut {
            part.copy_from_slice(&new_part);
            return true;
        }
    }
    false
}

/// FM refinement with multi-try localized passes and flow-based refinement.
///
/// The finest level's entry point, and the only caller of `hg_flow_refine`:
/// global FM first, then localized passes seeded around the boundary, then one
/// flow pass over what is left. Each stage starts from the previous stage's
/// output, and the flow pass declines outright on a boundary wider than its own
/// cap, so the FM stages also decide whether it runs at all.
pub(super) fn hg_fm_refine_with_local(hg: &Hypergraph, part: &mut [u8], imbalance: f64) {
    hg_fm_refine(hg, part, imbalance);

    let n = hg.num_vertices;
    if n < 20 {
        return;
    }

    // 7919 below is prime, so successive tries land in unrelated stretches of
    // the boundary list rather than in one region's worth of adjacent vertices.
    let num_tries = 4.min(n);
    let mut boundary: Vec<usize> = Vec::new();
    {
        let he_count = hg.pin_counts(part);
        for v in 0..n {
            for &hei in hg.vertex_hyperedges(v) {
                if he_count[hei as usize][0] > 0 && he_count[hei as usize][1] > 0 {
                    boundary.push(v);
                    break;
                }
            }
        }
    }
    if !boundary.is_empty() {
        for i in 0..num_tries {
            let seed = boundary[(i * 7919) % boundary.len()];
            hg_localized_fm_pass(hg, part, seed, imbalance);
        }
    }

    hg_flow_refine(hg, part, imbalance);
}
