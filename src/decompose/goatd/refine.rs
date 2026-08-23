//! Width-gated global-cut refinement of an existing tree decomposition.
//!
//! Motivation: goatd's `MinFillSampleJW` elimination is strong locally — each
//! pivot is optimal given a neighbourhood — but it has no notion of a global
//! balanced separator.  FlowCutter's anytime max-flow / Pareto balance-vs-cut
//! search does find good high-level cuts and tends to dominate min-fill on
//! structured CNFs whose primal graph has a small top-level separator.
//!
//! This module injects FlowCutter's notion of global structure into a goatd
//! TD by (a) picking one balanced separator `S` on the active subgraph, (b)
//! projecting the existing TD onto each side (preserving RIP), (c) gluing the
//! two projections at a new bag = `S`, and (d) recursing on each side while
//! the refinement strictly reduces `(width, total_bag_size)`.

use std::time::Instant;

use rustc_hash::FxHashSet;

use super::super::TreeDecomposition;
use super::super::flowcutter_rs::flowcutter_compute_separator;
use super::super::td_ops::{glue_at_separator, project_td_keeping_global_ids};
use super::super::td_parse::restrict_to_subset;
use crate::budget::expired;

/// FlowCutter restart breadth for one separator search — how many source/sink
/// pairs the anytime search tries per level.
const FC_REFINE_ITERS: i32 = 50;

/// Per-level FlowCutter step budget = `all_vars.len()` clamped to
/// `[REFINE_STEPS_MIN, REFINE_STEPS_MAX]`. A COMPUTATION-STEP budget rather
/// than a wall-clock one is what makes the refined decomposition — and hence
/// the vtree built from it — a pure function of the graph, identical however
/// loaded the machine is. A large top-level subgraph gets the full budget;
/// deeper (smaller) subgraphs get proportionally fewer steps but, because
/// FlowCutter's internal per-iteration `step_cost` (~sqrt(n·m)) also shrinks,
/// still run enough iterations.
const REFINE_STEPS_PER_VERTEX: i64 = 1;
const REFINE_STEPS_MIN: i64 = 2_000;
const REFINE_STEPS_MAX: i64 = 20_000;

/// Below this number of active variables, do not attempt to cut and refine.
const DEFAULT_MIN_SIDE_SIZE: usize = 16;
/// Safety valve against unbounded recursion.  A real CNF should terminate
/// well before this, driven by the dominance guard.
const MAX_RECURSION_DEPTH: u32 = 20;
/// Above this number of active vertices, a single FlowCutter iteration can
/// take seconds and is uninterruptible mid-iteration (the FC deadline check
/// only fires between iterations).  Incidence graphs of the hardest CNFs
/// blow past 300k vertices, at which point post-process refinement reliably
/// overruns the deadline it was given.  Above this gate, skip refinement
/// and return the input TD unchanged — the dominance guard would make the
/// call a no-op anyway in most cases, so the only cost is losing a
/// occasional win on a very large graph.
const MAX_VERTICES_FOR_REFINE: usize = 100_000;

/// Refine `td` by finding a FlowCutter-induced balanced separator on `all_vars`,
/// projecting `td` onto each side of the separator, and recursing while the
/// refinement strictly improves `(width, total_bag_size)`.
///
/// `all_edges` must be the full primal edge list for the current formula.
/// `all_vars` are the globally-numbered variables that are active in the
/// current subproblem (the top-level caller passes all variables).
pub(crate) fn refine_td_with_flowcutter_cut(
    td: TreeDecomposition,
    all_vars: &[u32],
    all_edges: &[(u32, u32)],
    deadline: Option<Instant>,
) -> TreeDecomposition {
    refine_inner(td, all_vars, all_edges, DEFAULT_MIN_SIDE_SIZE, 0, deadline)
}

fn refine_inner(
    td: TreeDecomposition,
    all_vars: &[u32],
    all_edges: &[(u32, u32)],
    min_side_size: usize,
    depth: u32,
    deadline: Option<Instant>,
) -> TreeDecomposition {
    if depth >= MAX_RECURSION_DEPTH {
        return td;
    }
    if all_vars.len() < min_side_size {
        return td;
    }
    // Large-graph gate: only applies when a deadline is set. A caller with no
    // deadline passes `None`, which bypasses the gate.
    if deadline.is_some() && all_vars.len() > MAX_VERTICES_FOR_REFINE {
        return td;
    }
    // Deadline guard: once the shared hard deadline passes, return the
    // current TD unchanged so the caller always has a valid decomposition.
    if expired(deadline) {
        return td;
    }

    let local_edges = restrict_to_subset(all_edges, all_vars);
    if local_edges.is_empty() {
        return td;
    }

    // The `0` timeout below is FlowCutter's "no wall-clock limit" sentinel —
    // the search is bounded purely by `fc_steps`. The shared `deadline` still
    // bounds the recursion between levels, but never reaches inside one
    // separator search.
    let fc_steps =
        (REFINE_STEPS_PER_VERTEX * all_vars.len() as i64).clamp(REFINE_STEPS_MIN, REFINE_STEPS_MAX);

    let sep_result = match flowcutter_compute_separator(
        all_vars.len(),
        &local_edges,
        fc_steps,
        FC_REFINE_ITERS,
        0,
    ) {
        Some(r) => r,
        None => return td,
    };

    if sep_result.separator.is_empty()
        || sep_result.side_a.is_empty()
        || sep_result.side_b.is_empty()
    {
        return td;
    }
    // Re-check deadline after the FC call: on large graphs a single FC can
    // consume most of the remaining budget, so projection + glue + recursion
    // would overrun.  Prefer returning the input TD over a partially-built
    // refinement that won't finish in time.
    if expired(deadline) {
        return td;
    }

    let sep_global: Vec<u32> = sep_result
        .separator
        .iter()
        .map(|&i| all_vars[i as usize])
        .collect();
    let side_a_global: Vec<u32> = sep_result
        .side_a
        .iter()
        .map(|&i| all_vars[i as usize])
        .collect();
    let side_b_global: Vec<u32> = sep_result
        .side_b
        .iter()
        .map(|&i| all_vars[i as usize])
        .collect();

    let keep_a: FxHashSet<u32> = side_a_global
        .iter()
        .chain(sep_global.iter())
        .copied()
        .collect();
    let keep_b: FxHashSet<u32> = side_b_global
        .iter()
        .chain(sep_global.iter())
        .copied()
        .collect();

    let num_vars = td.num_vars;

    let td_a = match project_td_keeping_global_ids(&td, &keep_a, num_vars) {
        Some(t) => t,
        None => return td,
    };
    let td_b = match project_td_keeping_global_ids(&td, &keep_b, num_vars) {
        Some(t) => t,
        None => return td,
    };
    let glued = match glue_at_separator(td_a.clone(), td_b.clone(), &sep_global, num_vars) {
        Some(g) => g,
        None => return td,
    };

    let glued_better = (glued.width(), glued.total_bag_size()) < (td.width(), td.total_bag_size());
    if !glued_better {
        return td;
    }

    let mut a_vars_sorted: Vec<u32> = side_a_global
        .iter()
        .chain(sep_global.iter())
        .copied()
        .collect();
    a_vars_sorted.sort_unstable();
    a_vars_sorted.dedup();

    let mut b_vars_sorted: Vec<u32> = side_b_global
        .iter()
        .chain(sep_global.iter())
        .copied()
        .collect();
    b_vars_sorted.sort_unstable();
    b_vars_sorted.dedup();

    let td_a_refined = refine_inner(
        td_a,
        &a_vars_sorted,
        all_edges,
        min_side_size,
        depth + 1,
        deadline,
    );
    let td_b_refined = refine_inner(
        td_b,
        &b_vars_sorted,
        all_edges,
        min_side_size,
        depth + 1,
        deadline,
    );

    match glue_at_separator(td_a_refined, td_b_refined, &sep_global, num_vars) {
        Some(g) => {
            // Defensive: if recursion regressed despite the per-level guard on
            // each side, keep the best of (glued-pre-recurse, td-original).
            if (g.width(), g.total_bag_size()) <= (glued.width(), glued.total_bag_size()) {
                g
            } else if (glued.width(), glued.total_bag_size()) < (td.width(), td.total_bag_size()) {
                glued
            } else {
                td
            }
        }
        None => glued,
    }
}
