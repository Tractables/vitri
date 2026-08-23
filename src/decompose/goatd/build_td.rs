//! Elimination order → `TreeDecomposition`.
//!
//! Builds the bag tree with the elimination-order clique-tree rule (parent =
//! earliest-eliminated neighbour still in the bag) — not the junction-tree
//! construction the vendored C++ FlowCutter uses for the same job. Both yield
//! valid tree decompositions and are not expected to agree; this one does not
//! dedup non-maximal bags.

use super::super::{GraphKind, TdBag, TreeDecomposition};

/// Build a `TreeDecomposition` from a per-step elimination record.
///
/// `steps[s]` is the eliminated vertex followed by the live neighbours that
/// were in its bag. `rank[v]` is the step at which vertex `v` was eliminated,
/// or `u32::MAX` for a still-live vertex (only occurs in the residual
/// base-case fallback).
pub(super) fn build_td_from_steps(
    kind: GraphKind,
    num_vars: u32,
    steps: Vec<Vec<u32>>,
    rank: &[u32],
) -> TreeDecomposition {
    let n_bags = steps.len();
    let mut bags: Vec<TdBag> = Vec::with_capacity(n_bags);
    for (id, vertices) in steps.into_iter().enumerate() {
        bags.push(TdBag { id, vertices });
    }

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n_bags];
    let n_bags_u32 = n_bags as u32;
    for s in 0..n_bags {
        let bag = &bags[s];
        if bag.vertices.len() <= 1 {
            continue;
        }
        let mut best = u32::MAX;
        for &u in &bag.vertices[1..] {
            let r = rank[u as usize];
            if r < best {
                best = r;
            }
        }
        if best < n_bags_u32 {
            adj[s].push(best as usize);
            adj[best as usize].push(s);
        }
    }

    TreeDecomposition {
        kind,
        bags,
        adj,
        num_vars,
    }
}
