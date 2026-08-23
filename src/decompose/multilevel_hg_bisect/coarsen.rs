//! One coarsening level: match vertices in pairs, contract each pair into a
//! coarse vertex, rebuild the hypergraph.
//!
//! Contraction is by matching, so a level can at best halve the vertex count;
//! `hg_multilevel_pass` calls this in a loop until it declines. Coarsening also
//! shrinks the hyperedges themselves — pins that land in the same coarse vertex
//! merge, a hyperedge left with one pin disappears, and hyperedges that end up
//! with identical pin sets become one with their weights summed.

use super::*;

pub(super) struct HgCoarseLevel {
    pub(super) hg: Hypergraph,
    pub(super) mapping: Vec<u32>,
}

/// If `part` is provided, preferentially match vertices in the same partition (V-cycle).
pub(super) fn hg_coarsen_one_level(
    hg: &Hypergraph,
    min_vertices: usize,
    rng: &mut Xorshift64,
    part: Option<&[u8]>,
) -> Option<HgCoarseLevel> {
    let n = hg.num_vertices;
    if n <= min_vertices {
        return None;
    }

    // SHEM analog for hypergraphs: degree-ascending order leaves high-degree
    // hubs to match last, with genuinely connected partners.
    let mut perm: Vec<usize> = (0..n).collect();
    perm.sort_by_key(|&v| {
        let deg = hg.v_he_start[v + 1] - hg.v_he_start[v];
        (deg, hg.vwgt[v])
    });
    // Shuffle within each equal-degree run: the degree order itself is what
    // this wants, but leaving ties in vertex-index order makes every level of
    // every restart match the same pairs first.
    {
        let mut i = 0;
        while i < n {
            let mut j = i + 1;
            let deg_i = hg.v_he_start[perm[i] + 1] - hg.v_he_start[perm[i]];
            while j < n && (hg.v_he_start[perm[j] + 1] - hg.v_he_start[perm[j]]) == deg_i {
                j += 1;
            }
            for k in (i + 1..j).rev() {
                let l = i + (rng.next_u64() as usize) % (k - i + 1);
                perm.swap(k, l);
            }
            i = j;
        }
    }

    let mut match_of: Vec<i32> = vec![-1; n]; // -1 = unmatched
    let mut coarse_id: Vec<u32> = vec![0; n];
    let mut num_coarse: u32 = 0;

    for &v in &perm {
        if match_of[v] != -1 {
            continue;
        }

        // Candidates are ranked by how many hyperedges they share with `v`,
        // counted here. The count ignores `hewgt`, so a pair sharing several
        // light hyperedges outranks one sharing a single heavy one — the graph
        // sibling matches on edge weight instead, and nothing in the tree
        // records why the two differ.
        use std::collections::HashMap;
        let mut connectivity: HashMap<u32, u32> = HashMap::new();

        for &hei in hg.vertex_hyperedges(v) {
            for &u in hg.hyperedge_pins(hei as usize) {
                let u = u as usize;
                if u != v && match_of[u] == -1 {
                    *connectivity.entry(u as u32).or_insert(0) += 1;
                }
            }
        }

        // `HashMap` iteration order varies from run to run, so the lowest-index
        // tie-break below is what makes the winner reproducible: a
        // same-partition candidate always displaces a cross-partition one, and
        // within a tier replacement is strict improvement under (connectivity
        // descending, index ascending) — a maximum under a total order does not
        // depend on the order the candidates arrive in. The graph sibling gets
        // the same tie-break for free by scanning a sorted adjacency slice.
        let mut best_nb: i32 = -1;
        let mut best_conn: u32 = 0;
        let mut best_same_part = false;
        for (&nb, &conn) in &connectivity {
            let same_part = part.is_some_and(|p| p[nb as usize] == p[v]);
            if same_part && !best_same_part {
                best_conn = conn;
                best_nb = nb as i32;
                best_same_part = true;
            } else if same_part == best_same_part
                && (conn > best_conn || (conn == best_conn && (nb as i32) < best_nb))
            {
                best_conn = conn;
                best_nb = nb as i32;
            }
        }

        if best_nb >= 0 {
            match_of[v] = best_nb;
            match_of[best_nb as usize] = v as i32;
            coarse_id[v] = num_coarse;
            coarse_id[best_nb as usize] = num_coarse;
            num_coarse += 1;
        } else {
            // Every neighbour already matched: the vertex crosses the level
            // alone, which is why a level can shrink by less than half and why
            // the floor below is needed at all.
            match_of[v] = v as i32;
            coarse_id[v] = num_coarse;
            num_coarse += 1;
        }
    }

    let nc = num_coarse as usize;
    if nc >= n * 9 / 10 {
        return None; // tuned 10% floor: stop once a level barely shrinks
    }

    let mut coarse_vwgt = vec![0u32; nc];
    for v in 0..n {
        coarse_vwgt[coarse_id[v] as usize] += hg.vwgt[v];
    }

    let num_he = hg.num_hyperedges();
    let mut coarse_he_with_wgt: Vec<(Vec<u32>, u32)> = Vec::with_capacity(num_he);
    for hei in 0..num_he {
        let pins = hg.hyperedge_pins(hei);
        let mut coarse_pins: Vec<u32> = pins.iter().map(|&v| coarse_id[v as usize]).collect();
        coarse_pins.sort_unstable();
        coarse_pins.dedup();
        if coarse_pins.len() >= 2 {
            coarse_he_with_wgt.push((coarse_pins, hg.hewgt[hei]));
        }
    }

    // Sorting by pin set brings identical coarse hyperedges together so the
    // scan below can merge them by summing weights: two hyperedges that this
    // contraction made indistinguishable are one hyperedge of their combined
    // weight from here up.
    coarse_he_with_wgt.sort_by(|a, b| a.0.cmp(&b.0));
    let mut coarse_hyperedges: Vec<Vec<u32>> = Vec::new();
    let mut coarse_hewgt: Vec<u32> = Vec::new();
    for (pins, w) in coarse_he_with_wgt {
        if !coarse_hyperedges.is_empty() && *coarse_hyperedges.last().unwrap() == pins {
            *coarse_hewgt.last_mut().unwrap() += w;
        } else {
            coarse_hyperedges.push(pins);
            coarse_hewgt.push(w);
        }
    }

    let mut coarse_hg = Hypergraph::from_hyperedges(nc, &coarse_hyperedges, Some(&coarse_hewgt));
    coarse_hg.vwgt = coarse_vwgt;

    Some(HgCoarseLevel {
        hg: coarse_hg,
        mapping: coarse_id,
    })
}
