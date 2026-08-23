//! Nested-dissection elimination ordering via `multilevel_bisect`.
//!
//! Recurse:
//!   1. Run multilevel bisection to partition vertices into two sides.
//!   2. Extract a minimum vertex cover of the bipartite cross-edge graph via
//!      König-Egerváry — this is the smallest vertex separator derivable from
//!      the given partition.
//!   3. Recurse on each side of `V \ separator`.
//!   4. Concatenate `order(A) ++ order(B) ++ order(separator)`.
//!
//! Base case (small subgraph, or [`MAX_RECURSION_DEPTH`] levels down): run
//! min-fill on the induced subgraph. The returned vector is a full elimination
//! order over `active` in global IDs; the caller applies it as a forced-order
//! elimination on the reduced graph.

use std::time::Instant;

use rustc_hash::FxHashSet;

use super::super::local_index;
use super::super::multilevel_bisect::multilevel_bisect;
use super::flow_cut;
use super::graph::Graph;
use super::minfill_core::{ElimSteps, ElimStop, eliminate_minfill};
use crate::budget::expired;

/// Default cutoff: once the induced subgraph has ≤ this many vertices, fall
/// back to local min-fill. Keeps recursion cost bounded while still letting
/// min-fill handle the dense tail where it does best.
pub(super) const DEFAULT_BASE_THRESH: usize = 32;

/// Safety valve against unbounded recursion, in the shape `refine` uses. Past
/// this depth `nd_order` stops splitting and finishes the subgraph with
/// min-fill, so the recursion cannot run the thread stack out no matter what
/// the bisector returns.
///
/// Not a tuning knob, and no real instance reaches it: the bisector holds each
/// side to `0.5 + max_imbalance` of the vertices, so at the 0.2 every caller
/// passes, a level keeps at most 70% of what it was given and `n` falls to
/// `DEFAULT_BASE_THRESH` inside 30 levels even for a residual of a million
/// vertices. The valve exists for the case where the bisector degenerates and
/// peels off a handful of vertices per level instead.
const MAX_RECURSION_DEPTH: u32 = 64;

/// What a whole `nd_order` recursion runs under. Every field is the same at
/// every level, so they travel as one reference rather than being re-threaded
/// through each call.
pub(super) struct NdParams<'a> {
    /// `salt[v]` is the RNG-salt for global vertex `v`, used by the base-case
    /// min-fill for tie-breaking.
    pub(super) salt: &'a [u32],
    /// Subgraph size at or below which a level stops splitting and runs
    /// min-fill instead.
    pub(super) base_thresh: usize,
    /// Balance tolerance handed to the bisector at every level.
    pub(super) max_imbalance: f64,
    /// Soft bound (checked at entry and at each recursion level) on the total
    /// time spent in `nd_order`. On huge residuals the multilevel-bisect
    /// recursion alone can take many seconds — if the deadline fires partway
    /// through, we bail and return `active` sorted by salt, which is a valid
    /// elimination order (not a good one, but the caller will still produce a
    /// complete TD). Without this, the deadline overshoots inside `nd_order`
    /// before the caller's elim loop ever gets a chance to bail.
    pub(super) hard_deadline: Option<Instant>,
    /// The schedule slot's seed, carried unchanged down the whole recursion to
    /// `multilevel_bisect`. Without it the bisector runs on a fixed RNG stream
    /// and the schedule's two `NestedDissMinCover` slots produce identical
    /// separators — see [`super::super::bisect_seed`].
    pub(super) base_seed: u64,
}

/// Compute a nested-dissection elimination order for the active vertex set
/// `active` (global IDs) whose internal edges are `edges` (global IDs).
///
/// `depth` counts recursion levels; the top-level caller passes 0. At
/// [`MAX_RECURSION_DEPTH`] the split stops and the subgraph goes to min-fill.
pub(super) fn nd_order(
    active: &[u32],
    edges: &[(u32, u32)],
    params: &NdParams<'_>,
    depth: u32,
) -> Vec<u32> {
    let salt = params.salt;
    let n = active.len();
    if n == 0 {
        return Vec::new();
    }
    if expired(params.hard_deadline) {
        // Bail with a salt-sorted permutation: valid order, poor width.
        let mut salt_sorted: Vec<u32> = active.to_vec();
        salt_sorted.sort_by_key(|&v| salt[v as usize]);
        return salt_sorted;
    }
    if n <= params.base_thresh || depth >= MAX_RECURSION_DEPTH {
        return base_minfill_order(active, edges, salt);
    }

    // Relabel active into dense 0..n so multilevel_bisect / flow_cut can use
    // vec-indexed adjacency without sparse maps.
    let local_edges = local_edges_for(active, edges);

    let part = multilevel_bisect(n, &local_edges, params.max_imbalance, params.base_seed);
    let sep = flow_cut::min_cover_separator(n, &local_edges, &part);

    // Degenerate partition — nothing to recurse on. Fall back to local min-fill.
    if sep.side_a.is_empty() || sep.side_b.is_empty() || sep.separator.len() >= n {
        return base_minfill_order(active, edges, salt);
    }

    let side_a_global = local_to_global(&sep.side_a, active);
    let side_b_global = local_to_global(&sep.side_b, active);
    let sep_global = local_to_global(&sep.separator, active);

    let edges_a = edges_induced_on(edges, &side_a_global);
    let edges_b = edges_induced_on(edges, &side_b_global);

    let mut order = nd_order(&side_a_global, &edges_a, params, depth + 1);
    order.extend(nd_order(&side_b_global, &edges_b, params, depth + 1));

    let mut sep_sorted = sep_global;
    sep_sorted.sort_by_key(|&v| salt[v as usize]);
    order.extend(sep_sorted);
    order
}

/// Min-fill on the induced subgraph of `active`, returning the resulting order
/// translated back to global IDs.
fn base_minfill_order(active: &[u32], edges: &[(u32, u32)], salt: &[u32]) -> Vec<u32> {
    let n = active.len();
    let local_edges = local_edges_for(active, edges);
    let mut local_graph = Graph::from_edges(n as u32, &local_edges);
    let local_salt: Vec<u32> = active.iter().map(|&v| salt[v as usize]).collect();
    let mut steps = ElimSteps::default();
    eliminate_minfill(
        &mut local_graph,
        &local_salt,
        steps.sink(),
        ElimStop::default(),
    );
    steps
        .rank_pairs
        .into_iter()
        .map(|(l, _)| active[l as usize])
        .collect()
}

/// Translate `edges` (global IDs) into dense 0..n local IDs where position `i`
/// in `active` becomes local ID `i`. Every endpoint must be in `active`.
///
/// Deliberately NOT
/// [`restrict_to_subset`](super::super::td_parse::restrict_to_subset), which
/// renumbers the same way but hands back a canonical list. The order and
/// orientation of what comes out here reach [`Graph::from_edges`], which fills
/// each adjacency list in the order it is handed, and the base case's min-fill
/// emits each bag in adjacency order — so sorting this list would change the
/// elimination orders this function exists to produce.
fn local_edges_for(active: &[u32], edges: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let to_local = local_index(active);
    edges
        .iter()
        .map(|&(u, v)| (to_local[&u], to_local[&v]))
        .collect()
}

/// Translate a list of local indices (positions into `active`) back to their
/// original global IDs.
fn local_to_global(locals: &[u32], active: &[u32]) -> Vec<u32> {
    locals.iter().map(|&l| active[l as usize]).collect()
}

fn edges_induced_on(edges: &[(u32, u32)], vertex_set: &[u32]) -> Vec<(u32, u32)> {
    let set: FxHashSet<u32> = vertex_set.iter().copied().collect();
    edges
        .iter()
        .copied()
        .filter(|&(u, v)| set.contains(&u) && set.contains(&v))
        .collect()
}
