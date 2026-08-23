//! One coarsening level: match vertices in pairs, contract each pair into a
//! coarse vertex, rebuild the CSR.
//!
//! Contraction is by matching, so a level can at best halve the vertex count;
//! `multilevel_pass` calls this in a loop until it declines. Only the coarse
//! graph and the fine-to-coarse mapping survive a level — the fine graph is
//! kept by the caller, since uncoarsening needs it back.

use super::*;

pub(super) struct CoarseningLevel {
    pub(super) graph: CsrGraph,
    /// mapping[fine_vertex] = coarse_vertex it was contracted into.
    pub(super) mapping: Vec<u32>,
}

/// If `part` is provided, preferentially match same-side vertices — the key
/// idea in V-cycle refinement (Karypis & Kumar 1998, Section 5.4): the coarse
/// graph then respects the partition being refined instead of cutting across
/// it. `None` is plain heavy-edge matching.
pub(super) fn coarsen_one_level(
    graph: &CsrGraph,
    min_vertices: usize,
    rng: &mut Xorshift64,
    part: Option<&[u8]>,
) -> Option<CoarseningLevel> {
    let n = graph.num_vertices();
    if n <= min_vertices {
        return None;
    }

    // Sorted Heavy-Edge Matching (SHEM): degree-ascending order leaves
    // high-degree hubs to match last, with genuinely connected partners
    // (Karypis & Kumar 1998).
    let mut perm: Vec<usize> = (0..n).collect();
    perm.sort_by_key(|&v| {
        let deg = graph.xadj[v + 1] - graph.xadj[v];
        (deg, graph.vwgt[v])
    });
    // Shuffle within each equal-degree run: the degree order itself is what
    // SHEM wants, but leaving ties in vertex-index order makes every level of
    // every restart match the same pairs first.
    let mut i = 0;
    while i < n {
        let mut j = i + 1;
        let deg_i = graph.xadj[perm[i] + 1] - graph.xadj[perm[i]];
        while j < n && (graph.xadj[perm[j] + 1] - graph.xadj[perm[j]]) == deg_i {
            j += 1;
        }
        for k in (i + 1..j).rev() {
            let l = i + (rng.next_u64() as usize) % (k - i + 1);
            perm.swap(k, l);
        }
        i = j;
    }
    let mut match_of: Vec<i32> = vec![-1; n]; // -1 = unmatched
    let mut coarse_id: Vec<u32> = vec![0; n];
    let mut num_coarse: u32 = 0;

    for &vtx in &perm {
        if match_of[vtx] != -1 {
            continue;
        }

        let start = graph.xadj[vtx] as usize;
        let end = graph.xadj[vtx + 1] as usize;
        let neighbors = &graph.adjncy[start..end];
        let weights = &graph.adjwgt[start..end];
        let mut best_nb: i32 = -1;
        let mut best_wt: u32 = 0;
        let mut best_same_part = false;

        for (&nb, &w) in neighbors.iter().zip(weights) {
            if match_of[nb as usize] != -1 {
                continue;
            }
            // Same-partition neighbours are preferred over cross-partition ones;
            // within that tier, heaviest edge wins. With no partition,
            // `best_same_part` stays false for every neighbour, degenerating to
            // plain heaviest-unmatched-neighbour.
            // `w > best_wt` is strict and `adjncy` is sorted ascending, so a
            // weight tie goes to the lowest-numbered neighbour. The hypergraph
            // sibling ranks candidates out of a `HashMap` and has to spell that
            // tie-break out to get the same determinism.
            let same_part = part.is_some_and(|p| p[nb as usize] == p[vtx]);
            if same_part && !best_same_part {
                best_wt = w;
                best_nb = nb as i32;
                best_same_part = true;
            } else if same_part == best_same_part && w > best_wt {
                best_wt = w;
                best_nb = nb as i32;
            }
        }

        if best_nb >= 0 {
            match_of[vtx] = best_nb;
            match_of[best_nb as usize] = vtx as i32;
            coarse_id[vtx] = num_coarse;
            coarse_id[best_nb as usize] = num_coarse;
            num_coarse += 1;
        } else {
            // Every neighbour already matched: the vertex crosses the level
            // alone, which is why a level can shrink by less than half and why
            // the floor below is needed at all.
            match_of[vtx] = vtx as i32;
            coarse_id[vtx] = num_coarse;
            num_coarse += 1;
        }
    }

    let nc = num_coarse as usize;
    if nc >= n * 9 / 10 {
        return None; // tuned 10% floor: stop once a level barely shrinks
    }

    let mut coarse_vwgt = vec![0u32; nc];
    for v in 0..n {
        coarse_vwgt[coarse_id[v] as usize] += graph.vwgt[v];
    }

    // Count then fill, with `counts` reused as the per-row cursor: the coarse
    // adjacency lands contiguously in one flat buffer instead of a Vec of Vecs.
    // Entries are duplicated here (one per fine edge) and merged below.
    let mut counts: Vec<u32> = vec![0; nc];
    for v in 0..n {
        let cv = coarse_id[v] as usize;
        for &nb in graph.neighbors(v) {
            let cnb = coarse_id[nb as usize];
            if cnb as usize != cv {
                counts[cv] += 1;
            }
        }
    }
    let mut offsets: Vec<u32> = Vec::with_capacity(nc + 1);
    offsets.push(0);
    let mut acc = 0u32;
    for &c in &counts {
        acc += c;
        offsets.push(acc);
    }
    let total = acc as usize;
    let mut flat: Vec<(u32, u32)> = vec![(0, 0); total];
    let mut cur = counts; // reuse as cursor; already sized nc
    for c in cur.iter_mut() {
        *c = 0;
    }
    for v in 0..n {
        let cv = coarse_id[v] as usize;
        let start = graph.xadj[v] as usize;
        let end = graph.xadj[v + 1] as usize;
        let neighbors = &graph.adjncy[start..end];
        let weights = &graph.adjwgt[start..end];
        for (&nb, &w) in neighbors.iter().zip(weights) {
            let cnb = coarse_id[nb as usize];
            if cnb as usize != cv {
                let pos = (offsets[cv] + cur[cv]) as usize;
                flat[pos] = (cnb, w);
                cur[cv] += 1;
            }
        }
    }

    // Sorting each cv's slice keeps CSR order deterministic. Merging the
    // parallel entries sums their weights, which is what makes heavy-edge
    // matching meaningful one level up: a coarse edge's weight is the number of
    // fine edges that would be cut by separating its two endpoints.
    let mut xadj = Vec::with_capacity(nc + 1);
    let mut adjncy = Vec::new();
    let mut adjwgt = Vec::new();
    xadj.push(0u32);
    for v in 0..nc {
        let start = offsets[v] as usize;
        let end = offsets[v + 1] as usize;
        let slice = &mut flat[start..end];
        slice.sort_unstable_by_key(|&(nb, _)| nb);
        let mut i = 0;
        while i < slice.len() {
            let nb = slice[i].0;
            let mut wt = 0u32;
            while i < slice.len() && slice[i].0 == nb {
                wt = wt.saturating_add(slice[i].1);
                i += 1;
            }
            adjncy.push(nb);
            adjwgt.push(wt);
        }
        xadj.push(adjncy.len() as u32);
    }

    Some(CoarseningLevel {
        graph: CsrGraph {
            xadj,
            adjncy,
            vwgt: coarse_vwgt,
            adjwgt,
        },
        mapping: coarse_id,
    })
}
