//! Multilevel graph bisection (Karypis & Kumar, SIAM 1998); recursive-bisection
//! partitioner for `nparts=2`.
//!
//! Three phases, one per submodule: `coarsen` contracts the graph down to a few
//! dozen vertices, `initial` partitions that, `refine` improves the partition on
//! the way back up. `multilevel_pass` drives all three.
//!
//! Consumers: `goatd`'s nested-dissection elimination ordering, `goatd`'s
//! refinement step, and the hybrid tree-decomposition/bisection vtree builder.
//! Each of them wants a small separator rather than a small edge cut, a
//! distinction `multilevel_bisect` acts on.
//!
//! Sibling of [`multilevel_hg_bisect`](super::multilevel_hg_bisect), the
//! hypergraph variant. The two share the containers and the pass bookkeeping in
//! [`fm_common`](super::fm_common); the gain models stay apart, and are not to
//! be merged — each is a perf-sensitive, benchmark-validated inner loop written
//! against its own notion of what a cut costs. Every other difference between
//! the two is recorded there as well, under "Where the two bisectors differ".

use super::fm_common::{index_split, tiny_bisection};
use super::rng::{Xorshift64, bisector_stream};

mod coarsen;
mod graph;
mod initial;
mod refine;

#[cfg(test)]
mod tests;

use coarsen::*;
use graph::*;
use initial::*;
use refine::*;

const MIN_COARSEN_SIZE: usize = 20;

/// Majority vote down one level: a coarse vertex takes the side most of its
/// fine vertices are on, ties to side 0.
///
/// `src` is indexed by fine vertex and `mapping[fine] = coarse`, so `dst` comes
/// back `nc` long and indexed by coarse vertex.
#[inline]
fn project_partition(
    src: &[u8],
    mapping: &[u32],
    nc: usize,
    count_scratch: &mut Vec<[u32; 2]>,
    dst: &mut Vec<u8>,
) {
    count_scratch.clear();
    count_scratch.resize(nc, [0, 0]);
    for (v, &m) in mapping.iter().enumerate() {
        count_scratch[m as usize][src[v] as usize] += 1;
    }
    dst.clear();
    dst.resize(nc, 0);
    for cv in 0..nc {
        dst[cv] = if count_scratch[cv][1] > count_scratch[cv][0] {
            1
        } else {
            0
        };
    }
}

/// One coarsen -> partition -> uncoarsen sweep; returns 0/1 per vertex of
/// `graph`.
///
/// `part` is an existing partition of `graph` to improve rather than replace:
/// it is projected down as the levels are built, coarsening is told to keep its
/// two sides apart, and no initial partition is taken. That is the V-cycle
/// (Karypis & Kumar 1998, Section 5.4).
fn multilevel_pass(
    graph: &CsrGraph,
    part: Option<&[u8]>,
    rng: &mut Xorshift64,
    max_imbalance: f64,
    scratch: &mut FmScratch,
) -> Vec<u8> {
    let n = graph.num_vertices();

    // Pre-allocated once, reused across all levels: avoids O(L²) reallocation.
    let mut count_scratch: Vec<[u32; 2]> = Vec::with_capacity(n);
    let mut proj_scratch: Vec<u8> = Vec::with_capacity(n);

    // Coarsening. Each level contracts matched pairs, so `levels` ends up
    // ordered finest-first and `current` walks down to the coarsest graph.
    let mut levels: Vec<CoarseningLevel> = Vec::new();
    let mut current = graph;
    loop {
        // Full re-projection each level, not incremental: incremental
        // projection degrades partition quality here.
        let mut fine_part: Option<Vec<u8>> = None;
        if let Some(p) = part {
            let mut fp = p.to_vec();
            for lv in &levels {
                let nc = lv.graph.num_vertices();
                project_partition(&fp, &lv.mapping, nc, &mut count_scratch, &mut proj_scratch);
                std::mem::swap(&mut fp, &mut proj_scratch);
            }
            fine_part = Some(fp);
        }
        let level = coarsen_one_level(current, MIN_COARSEN_SIZE, rng, fine_part.as_deref());

        if let Some(level) = level {
            levels.push(level);
            current = &levels.last().unwrap().graph;
        } else {
            break;
        }
    }

    // Coarsest level: either the caller's partition projected the whole way
    // down, or a fresh one grown here. This is the only point in the sweep
    // where a partition is created rather than improved.
    let mut coarse_part = if let Some(p) = part {
        let mut fine_part = p.to_vec();
        for level in &levels {
            let nc = level.graph.num_vertices();
            project_partition(
                &fine_part,
                &level.mapping,
                nc,
                &mut count_scratch,
                &mut proj_scratch,
            );
            std::mem::swap(&mut fine_part, &mut proj_scratch);
        }
        fine_part
    } else {
        initial_partition(current, rng, max_imbalance, scratch)
    };

    // Coarse edges carry the summed weight of everything contracted into them,
    // so a move here is worth many fine edges and this is the cheapest place in
    // the sweep to buy cut.
    fm_refine(current, &mut coarse_part, max_imbalance, scratch);

    // Uncoarsening. Each step hands every fine vertex its coarse vertex's side,
    // then refines with the freedom the finer graph exposes; only the finest
    // level pays for the localized passes on top.
    let mut result_part = coarse_part;
    for (li, level) in levels.iter().enumerate().rev() {
        let fine_n = level.mapping.len();
        proj_scratch.clear();
        proj_scratch.resize(fine_n, 0);
        for v in 0..fine_n {
            proj_scratch[v] = result_part[level.mapping[v] as usize];
        }
        std::mem::swap(&mut result_part, &mut proj_scratch);

        let fine_graph = if li > 0 { &levels[li - 1].graph } else { graph };
        if li == 0 {
            fm_refine_with_local(fine_graph, &mut result_part, max_imbalance, scratch);
        } else {
            fm_refine(fine_graph, &mut result_part, max_imbalance, scratch);
        }
    }

    let count0 = result_part.iter().filter(|&&p| p == 0).count();
    if count0 == 0 || count0 == n {
        result_part = index_split(n);
    }

    result_part
}

fn multilevel_bisect_once(
    graph: &CsrGraph,
    rng: &mut Xorshift64,
    max_imbalance: f64,
    scratch: &mut FmScratch,
) -> Vec<u8> {
    let mut part = multilevel_pass(graph, None, rng, max_imbalance, scratch);

    // Arbitrary tuned thresholds: more V-cycles for larger graphs, where
    // quality matters more.
    let num_vcycles = if graph.num_vertices() >= 400 {
        4
    } else if graph.num_vertices() >= 100 {
        2
    } else {
        1
    };
    // The first cycle that fails to improve ends the loop, so `num_vcycles` is
    // a ceiling rather than a count.
    for _ in 0..num_vcycles {
        let old_cut = edge_cut(graph, &part);
        let new_part = multilevel_pass(graph, Some(&part), rng, max_imbalance, scratch);
        let new_cut = edge_cut(graph, &new_part);
        if new_cut < old_cut {
            part = new_part;
        } else {
            break;
        }
    }

    part
}

/// Multilevel 2-way bisection: one pass, refined by V-cycles.
///
/// One pass and not a best-of-N over restarts, for the reason under "Where the
/// two bisectors differ" in [`fm_common`](super::fm_common).
///
/// `base_seed` is the caller's context seed (a goatd schedule slot, or `0`
/// for a caller with none); it selects which RNG stream the pass runs on.
pub(super) fn multilevel_bisect(
    n: usize,
    edges: &[(u32, u32)],
    max_imbalance: f64,
    base_seed: u64,
) -> Vec<u8> {
    if let Some(part) = tiny_bisection(n) {
        return part;
    }

    let graph = build_csr(n, edges);

    if graph.adjncy.is_empty() {
        return index_split(n);
    }

    let mut scratch = FmScratch::new();

    let mut rng = bisector_stream(super::bisect_seed::restart_seed(base_seed, 0));
    multilevel_bisect_once(&graph, &mut rng, max_imbalance, &mut scratch)
}
