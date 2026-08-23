//! The two tie-set-sampling cores.
//!
//! These stay outside `greedy.rs`'s skeleton on purpose. They do not pop from
//! a heap: they read the whole minimum-priority bucket out of a [`BucketMap`]
//! and draw a vertex from it at random, which means no lazy deletion, no stale
//! entries to skip, and a priority structure that has to be kept exact rather
//! than corrected on pop. `eliminate_minfill_sampling` also has no
//! clique-residual fast drain — at fill 0 every remaining vertex ties, and
//! draining them in index order instead of sampling order produced a
//! measurably worse vtree — and updates its buckets *before* the vertex is
//! removed on the bitset path, where the skeleton updates after. Folding them
//! in would mean two more hooks that only they use, and the sampling loop
//! would be harder to read for it, not easier.

use super::*;
use crate::budget::expired;
use crate::decompose::rng::{SEED_OFFSET, Xorshift64};

/// Re-measure every still-active neighbour's fill and move it to the matching
/// bucket. This is the eager half of `eliminate_minfill_sampling`'s bucket
/// maintenance — the sampler cannot tolerate a stale key, since a stale bucket
/// biases which vertex gets sampled, not just which entry pops first.
fn rescore_neighbours(
    scratch: &mut FillScratch,
    graph: &Graph,
    nbrs: &[u32],
    buckets: &mut BucketMap,
) {
    for &u in nbrs {
        if graph.active[u as usize] {
            let new_fill = scratch.fill_count_of(graph, u);
            buckets.update(u, new_fill);
        }
    }
}

/// htd-style min-fill elimination: priority = fill only (no secondary degree
/// or salt key), ties broken by random sampling from the full min-fill tie
/// set. `weight` biases the sample: P(v) ∝ `weight[v] + 1`. Direction
/// (low-eliminate-first vs high-eliminate-first) is baked into the weight by
/// the caller — `sat_score::compute_weight`'s argmin convention is inverted
/// before it reaches here.
pub(crate) fn eliminate_minfill_sampling(
    graph: &mut Graph,
    weight: &[u32],
    seed: u64,
    mut sink: ElimSink<'_>,
    stop: ElimStop,
    initial_fill: Option<&[u64]>,
) -> ElimExit {
    // No cheap mode here to degrade into, so the soft deadline is not this
    // core's to read.
    let ElimStop {
        hard_deadline,
        width_bound,
        ..
    } = stop;
    let n = graph.len();
    assert_eq!(weight.len(), n);

    if graph.should_promote_bitset() {
        graph.promote_bitset();
    }

    let mut scratch = FillScratch::new(n);
    let mut live_nbrs: Vec<u32> = Vec::with_capacity(n);
    let mut buckets = BucketMap::with_capacity(n);
    for v in 0..n {
        if graph.active[v] {
            let f = match initial_fill {
                Some(f) => f[v],
                None => scratch.fill_count_of(graph, v as u32),
            };
            buckets.insert(v as u32, f);
        }
    }

    // `+ SEED_OFFSET` keeps a seed of 0 off xorshift64's zero fixed point, and
    // is part of the tie-break stream this sampler has always drawn.
    let mut rng = Xorshift64::from_state(seed.wrapping_add(SEED_OFFSET));
    let mut check_counter = 0u32;

    while let Some((min_fill, tie_set)) = buckets.min_bucket() {
        check_counter += 1;
        if check_counter >= DEADLINE_CHECK_STRIDE {
            check_counter = 0;
            if expired(hard_deadline) {
                return ElimExit::DeadlineBailed;
            }
            if graph.should_promote_bitset() {
                graph.promote_bitset();
            }
        }

        let v = sample_tie_set(tie_set, weight, &mut rng);

        buckets.remove_vertex(v);

        let bag = take_bag(graph, v, &mut live_nbrs);

        if min_fill == 0 {
            // Bitset mode: exact Δfill update in O(w) before removing v.
            // When v is simplicial, N(v) is a clique so no fill edges are added.
            // Δfill(u) = -|N(u) \ N(v) \ {v}| = -(popcount(bs[u] & ~bs[v]) - 1).
            if graph.bitset_words > 0 {
                let w = graph.bitset_words;
                let vb = v as usize * w;
                for &u in &live_nbrs {
                    let ui = u as usize;
                    let ub = ui * w;
                    let mut o_count = 0u64;
                    for j in 0..w {
                        o_count +=
                            (graph.bitset[ub + j] & !graph.bitset[vb + j]).count_ones() as u64;
                    }
                    o_count = o_count.saturating_sub(1); // exclude v's own bit
                    if let Some(old_key) = buckets.key_of(u) {
                        buckets.update(u, old_key.saturating_sub(o_count));
                    }
                }
                graph.remove_without_fill_nbrs(v, &live_nbrs);
            } else {
                graph.remove_without_fill_nbrs(v, &live_nbrs);
                rescore_neighbours(&mut scratch, graph, &live_nbrs, &mut buckets);
            }
        } else {
            graph.eliminate_with_nbrs(v, &live_nbrs);
            rescore_neighbours(&mut scratch, graph, &live_nbrs, &mut buckets);
        }
        let bag_len = bag.len();
        sink.record(v, bag);

        if exceeds_width_bound(bag_len, width_bound) {
            return ElimExit::WidthAborted;
        }
    }
    ElimExit::Natural
}

/// htd-style min-degree elimination: priority = degree only, ties broken by
/// random sampling from the full min-degree tie set. `weight` biases the
/// sample (see `eliminate_minfill_sampling`).
pub(crate) fn eliminate_mindegree_sampling(
    graph: &mut Graph,
    weight: &[u32],
    seed: u64,
    mut sink: ElimSink<'_>,
    stop: ElimStop,
) -> ElimExit {
    // As in `eliminate_minfill_sampling`: no cheap mode, so no soft deadline.
    let ElimStop {
        hard_deadline,
        width_bound,
        ..
    } = stop;
    let n = graph.len();
    assert_eq!(weight.len(), n);

    let mut buckets = BucketMap::with_capacity(n);
    for v in 0..n {
        if graph.active[v] {
            buckets.insert(v as u32, graph.degree(v as u32) as u64);
        }
    }

    // `+ SEED_OFFSET` keeps a seed of 0 off xorshift64's zero fixed point, and
    // is part of the tie-break stream this sampler has always drawn.
    let mut rng = Xorshift64::from_state(seed.wrapping_add(SEED_OFFSET));
    let mut nbrs_buf: Vec<u32> = Vec::with_capacity(n);
    let mut check_counter = 0u32;
    let mut clique_residual = false;
    // Lazy degree tracking — defer bucket update to sample time.
    let mut degree_stale: Vec<bool> = vec![false; n];

    while let Some((min_deg, tie_set)) = buckets.min_bucket() {
        check_counter += 1;
        if check_counter >= DEADLINE_CHECK_STRIDE {
            check_counter = 0;
            if expired(hard_deadline) {
                return ElimExit::DeadlineBailed;
            }
        }

        let v = sample_tie_set(tie_set, weight, &mut rng);
        let vi = v as usize;

        if degree_stale[vi] {
            let live_degree = graph.degree(v) as u64;
            degree_stale[vi] = false;
            if live_degree != min_deg {
                buckets.update(v, live_degree);
                continue;
            }
        }

        buckets.remove_vertex(v);

        let bag = take_bag(graph, v, &mut nbrs_buf);
        let bag_len = bag.len();

        if !clique_residual && graph.is_residual_clique() {
            clique_residual = true;
        }
        if clique_residual {
            graph.remove_without_fill_nbrs(v, &nbrs_buf);
        } else {
            graph.eliminate_with_nbrs(v, &nbrs_buf);
        }
        sink.record(v, bag);

        if exceeds_width_bound(bag_len, width_bound) {
            return ElimExit::WidthAborted;
        }

        for &u in &nbrs_buf {
            degree_stale[u as usize] = true;
        }
    }
    ElimExit::Natural
}
