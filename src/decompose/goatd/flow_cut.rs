//! Flow-based node-separator extraction via König-Egerváry: the minimum
//! vertex cover of the bipartite boundary graph induced by a partition is a
//! provably minimum separator derivable from that partition. goatd's
//! nested-dissection separator step.

use rustc_hash::FxHashSet;

/// Extract a minimum vertex cover of the bipartite graph induced by the
/// cross-edges of `part` over vertex set `0..n`, and return it as a list of
/// local vertex IDs (a valid node separator).
///
/// `edges` are undirected edges in local IDs; `part[v] ∈ {0,1}` is the side of
/// vertex `v`. Edges with both endpoints on the same side are ignored.
///
/// Also returns the resulting bipartition sides (the two components of
/// `V \ separator`, each restricted to its original `part` side).
pub(super) fn min_cover_separator(n: usize, edges: &[(u32, u32)], part: &[u8]) -> SeparatorResult {
    let mut is_boundary_a = vec![false; n];
    let mut is_boundary_b = vec![false; n];
    let mut cross_edges: Vec<(u32, u32)> = Vec::new();
    for &(u, v) in edges {
        if part[u as usize] != part[v as usize] {
            let (a, b) = if part[u as usize] == 0 {
                (u, v)
            } else {
                (v, u)
            };
            is_boundary_a[a as usize] = true;
            is_boundary_b[b as usize] = true;
            cross_edges.push((a, b));
        }
    }
    let boundary_a: Vec<u32> = (0..n as u32)
        .filter(|&i| is_boundary_a[i as usize])
        .collect();
    let boundary_b: Vec<u32> = (0..n as u32)
        .filter(|&i| is_boundary_b[i as usize])
        .collect();

    let la = boundary_a.len();
    let lb = boundary_b.len();
    let mut idx_a: Vec<i32> = vec![-1; n];
    let mut idx_b: Vec<i32> = vec![-1; n];
    for (i, &v) in boundary_a.iter().enumerate() {
        idx_a[v as usize] = i as i32;
    }
    for (i, &v) in boundary_b.iter().enumerate() {
        idx_b[v as usize] = i as i32;
    }

    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); la];
    for &(a, b) in &cross_edges {
        let ai = idx_a[a as usize];
        let bi = idx_b[b as usize];
        debug_assert!(ai >= 0 && bi >= 0);
        adj[ai as usize].push(bi as u32);
    }
    for list in &mut adj {
        list.sort_unstable();
        list.dedup();
    }

    // Max matching via Kuhn's algorithm.
    let mut match_l: Vec<i32> = vec![-1; la];
    let mut match_r: Vec<i32> = vec![-1; lb];
    let mut visited_r: Vec<bool> = vec![false; lb];
    for u in 0..la {
        for v in visited_r.iter_mut() {
            *v = false;
        }
        try_kuhn(u, &adj, &mut visited_r, &mut match_l, &mut match_r);
    }

    // König's construction: Z = vertices reachable from unmatched L via
    // alternating paths. Min vertex cover = (L \ Z) ∪ (R ∩ Z).
    let mut in_z_l = vec![false; la];
    let mut in_z_r = vec![false; lb];
    let mut queue: Vec<usize> = Vec::new();
    for u in 0..la {
        if match_l[u] < 0 {
            in_z_l[u] = true;
            queue.push(u);
        }
    }
    while let Some(u) = queue.pop() {
        for &v in &adj[u] {
            let vi = v as usize;
            if match_l[u] == v as i32 {
                continue;
            }
            if !in_z_r[vi] {
                in_z_r[vi] = true;
                if match_r[vi] >= 0 && !in_z_l[match_r[vi] as usize] {
                    in_z_l[match_r[vi] as usize] = true;
                    queue.push(match_r[vi] as usize);
                }
            }
        }
    }

    // Separator = (L \ Z) ∪ (R ∩ Z).
    let mut separator: Vec<u32> = Vec::new();
    for (i, &v) in boundary_a.iter().enumerate() {
        if !in_z_l[i] {
            separator.push(v);
        }
    }
    for (i, &v) in boundary_b.iter().enumerate() {
        if in_z_r[i] {
            separator.push(v);
        }
    }

    let sep_set: FxHashSet<u32> = separator.iter().copied().collect();
    let mut side_a: Vec<u32> = Vec::new();
    let mut side_b: Vec<u32> = Vec::new();
    for v in 0..n as u32 {
        if sep_set.contains(&v) {
            continue;
        }
        if part[v as usize] == 0 {
            side_a.push(v);
        } else {
            side_b.push(v);
        }
    }

    SeparatorResult {
        side_a,
        side_b,
        separator,
    }
}

pub(super) struct SeparatorResult {
    pub side_a: Vec<u32>,
    pub side_b: Vec<u32>,
    pub separator: Vec<u32>,
}

/// One level of the augmenting-path search: the left vertex being matched, and
/// how far its adjacency list has been scanned.
struct KuhnFrame {
    u: usize,
    cursor: usize,
}

/// Kuhn's augmenting-path matcher. Returns true iff `start` was (newly) matched.
///
/// Depth-first over alternating paths, with the search levels held on an
/// explicit stack rather than the call stack. Each level scans `adj[u]` in
/// order, marks a right vertex the moment it is first tried, and records its
/// own pairing as control returns through it once a level below reports
/// success — so pairings are written deepest-first, exactly as the recursion
/// would unwind.
fn try_kuhn(
    start: usize,
    adj: &[Vec<u32>],
    visited_r: &mut [bool],
    match_l: &mut [i32],
    match_r: &mut [i32],
) -> bool {
    let mut stack = vec![KuhnFrame {
        u: start,
        cursor: 0,
    }];
    // What the level below reported, waiting to be acted on by the level above:
    // `Some(true)` means the right vertex that level was tried with is now free
    // to take, `Some(false)` means keep scanning. `None` means nothing to
    // report — the current level has not descended yet.
    let mut reported: Option<bool> = None;

    while !stack.is_empty() {
        // `take` always: a report is consumed whether or not it succeeded. On
        // failure the level below found no augmenting path and this one resumes
        // scanning where it left off.
        if reported.take() == Some(true) {
            let level = stack.last().unwrap();
            // The edge this level was on: the one its cursor just passed.
            let (u, v) = (level.u, adj[level.u][level.cursor - 1]);
            match_l[u] = v as i32;
            match_r[v as usize] = u as i32;
            stack.pop();
            reported = Some(true);
            continue;
        }

        let top = stack.len() - 1;
        let u = stack[top].u;
        let mut stepped = false;
        while stack[top].cursor < adj[u].len() {
            let v = adj[u][stack[top].cursor];
            stack[top].cursor += 1;
            let vi = v as usize;
            if visited_r[vi] {
                continue;
            }
            visited_r[vi] = true;
            if match_r[vi] >= 0 {
                stack.push(KuhnFrame {
                    u: match_r[vi] as usize,
                    cursor: 0,
                });
            } else {
                // The pairing itself is recorded on the next iteration's
                // `reported.take()` branch above, not here.
                reported = Some(true);
            }
            stepped = true;
            break;
        }
        if !stepped {
            stack.pop();
            reported = Some(false);
        }
    }

    reported == Some(true)
}
