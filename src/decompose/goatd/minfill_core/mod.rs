//! Randomized min-fill elimination with RNG-salted tie-breaking.
//!
//! Ties break first by degree (lower wins), then by a salt (the RNG-provided
//! priority) rather than vertex id — this gives multi-seed diversity without
//! changing the algorithm.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::time::Instant;

use super::graph::Graph;
use crate::decompose::rng::Xorshift64;

/// Generates `Ord`/`PartialOrd` for a heap-entry struct that orders solely by
/// its `key` field (each slot `Reverse`-wrapped so minimums pop first on
/// Rust's max-heap).
macro_rules! ord_by_key {
    ($t:ty) => {
        impl Ord for $t {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                self.key.cmp(&other.key)
            }
        }
        impl PartialOrd for $t {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }
    };
}

mod greedy;
mod heap_degree;
mod heap_fill;
mod sampling;

#[cfg(test)]
mod tests;

pub(super) use heap_degree::eliminate_mindegree;
pub(super) use heap_fill::eliminate_minfill;
pub(super) use sampling::{eliminate_mindegree_sampling, eliminate_minfill_sampling};

/// When an elimination run must stop.
///
/// The soft `deadline` does not end the run: it degrades min-fill/min-degree
/// scoring to a stale-heap cheap mode, so bookkeeping stops dominating cost on
/// pathologically dense graphs. `hard_deadline` ends it, leaving a path
/// decomposition over whatever is still active, so a complete decomposition
/// comes back even where cheap-mode overshoots by seconds per elimination.
/// `width_bound` is the best width the caller's schedule already holds: a bag
/// wider than that cannot win, so finishing is wasted work.
///
/// One value rather than three parameters, so which of them a core reads is
/// visible where the run is set up. The two sampling cores have no cheap mode
/// to degrade to and so read no soft deadline — their callers write it `None`.
#[derive(Clone, Copy, Default)]
pub(super) struct ElimStop {
    /// Soft cutoff: degrade to cheap-mode scoring past it.
    pub(super) deadline: Option<Instant>,
    /// Hard cutoff: stop eliminating past it.
    pub(super) hard_deadline: Option<Instant>,
    /// The width past which this run's decomposition can no longer win.
    pub(super) width_bound: Option<u32>,
}

/// Check deadline every 64 heap pops — cheap wall-clock check (~tens of ns),
/// small enough granularity that we won't run far past the deadline even on
/// graphs whose per-step work spans milliseconds.
const DEADLINE_CHECK_STRIDE: u32 = 64;

/// Above this many active vertices, a single cheap-mode eliminate on a dense
/// residual can overshoot the deadline by seconds. Emergency-bail immediately
/// on deadline rather than attempting cheap-mode elimination at this scale.
pub(super) const CHEAP_MODE_MAX_ACTIVE: usize = 512;

/// How an elimination run ended. Callers use `WidthAborted` to skip the slot
/// entirely (no valid TD produced, just bail without emitting emergency bags);
/// `DeadlineBailed` means the run stopped short, and the caller that asked for
/// `force_emit` gets the emergency path-decomp appended so it still holds a
/// complete TD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ElimExit {
    Natural,
    DeadlineBailed,
    WidthAborted,
}

/// Early-abort test: a bag strictly wider than `bound + 1` can no longer win
/// even after the total-bag-size tiebreak on an exact width tie.
#[inline]
pub(super) fn exceeds_width_bound(bag_len: usize, bound: Option<u32>) -> bool {
    matches!(bound, Some(b) if bag_len > b as usize + 1)
}

/// What an elimination produced: one bag per eliminated vertex, and the step
/// each vertex went at.
///
/// The two are one value because they are one account of the same eliminations,
/// read together by everything downstream — a bag list of one length beside a
/// rank list of another describes no decomposition at all. [`ElimSteps::sink`]
/// is the only way to append, which is what keeps them the same length and the
/// step numbering continuous across the several eliminations that may
/// contribute to one run.
#[derive(Clone, Default)]
pub(crate) struct ElimSteps {
    /// Each entry is `{eliminated_v} ++ its live neighbours at the time`.
    pub bags: Vec<Vec<u32>>,
    /// `(vertex, step)` per eliminated vertex; `step` indexes [`Self::bags`].
    pub rank_pairs: Vec<(u32, usize)>,
}

impl ElimSteps {
    /// A sink that appends to these steps, numbering its first bag after the
    /// last one already here — so an elimination resuming after a preprocessing
    /// prefix, or a helper finishing another's tail, keeps the numbering going.
    pub(super) fn sink(&mut self) -> ElimSink<'_> {
        let start_step = self.bags.len();
        ElimSink::new(&mut self.bags, &mut self.rank_pairs, start_step)
    }

    /// Append these steps, taken over a component's own `0..k` vertex ids, to
    /// `bags` and `rank` in the original ids `comp` maps them back to.
    ///
    /// `rank` is indexed by original vertex id and holds each vertex's step in
    /// the concatenated bag list, which is why the bags go first: their count so
    /// far is the offset every step here shifts by.
    pub(super) fn append_reindexed(self, comp: &[u32], bags: &mut Vec<Vec<u32>>, rank: &mut [u32]) {
        let base = bags.len();
        for mut bag in self.bags {
            for v in &mut bag {
                *v = comp[*v as usize];
            }
            bags.push(bag);
        }
        for (local_v, step) in self.rank_pairs {
            rank[comp[local_v as usize] as usize] = (base + step) as u32;
        }
    }
}

/// Where an elimination writes what it produced, one bag at a time.
///
/// [`ElimSink::record`] is the only thing that appends, so the bag list, the
/// ranks and the step counter advance as one.
pub(super) struct ElimSink<'a> {
    bags: &'a mut Vec<Vec<u32>>,
    ranks: &'a mut Vec<(u32, usize)>,
    step: usize,
}

impl<'a> ElimSink<'a> {
    /// Append to `bags`/`ranks`, numbering the first bag `start_step`. A run
    /// resuming after a preprocessing prefix passes that prefix's length.
    pub(super) fn new(
        bags: &'a mut Vec<Vec<u32>>,
        ranks: &'a mut Vec<(u32, usize)>,
        start_step: usize,
    ) -> Self {
        ElimSink {
            bags,
            ranks,
            step: start_step,
        }
    }

    /// Record `bag`, emitted by eliminating `v`, at the current step.
    #[inline]
    pub(super) fn record(&mut self, v: u32, bag: Vec<u32>) {
        self.bags.push(bag);
        self.ranks.push((v, self.step));
        self.step += 1;
    }
}

/// Emergency path decomposition of the remaining active vertices: a chain of
/// bags where bag[i] = {remaining[i..]}, so every pair of remaining vertices
/// shares a bag (width = #remaining − 1). Used only when the hard deadline
/// fires, so a complete TD is always returned rather than none.
pub(super) fn emergency_path_decomp(graph: &Graph, sink: &mut ElimSink<'_>) {
    let remaining: Vec<u32> = (0..graph.len() as u32)
        .filter(|&v| graph.active[v as usize])
        .collect();
    for (i, &v) in remaining.iter().enumerate() {
        sink.record(v, remaining[i..].to_vec());
    }
}

/// Scratch state for min-fill's fill-count computation, reused across calls
/// to avoid reallocating per vertex.
struct FillScratch {
    /// Stamp-based marker: `marker[v] == stamp` iff v ∈ current N(v) being
    /// checked. u16 (2 bytes/entry) instead of u32 (4 bytes/entry) halves the
    /// marker footprint — at N=16K the marker goes from 64KB (2×L1) to 32KB
    /// (fits L1), reducing random-access cache pressure in the fill-count
    /// inner loop. Wraparound at u16::MAX triggers a full `fill(0)` reset so
    /// stale stamp values from the previous cycle are never misread.
    marker: Vec<u16>,
    stamp: u16,
}

impl FillScratch {
    fn new(n: usize) -> Self {
        FillScratch {
            marker: vec![0; n],
            stamp: 0,
        }
    }

    #[inline]
    fn bump_stamp(&mut self) {
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            self.marker.fill(0);
            self.stamp = 1;
        }
    }

    /// Count of fill edges needed to eliminate `v`. Counts edges inside N(v)
    /// via a stamp-marked array in O(|N(v)| + Σ deg(u) for u ∈ N(v)) time,
    /// avoiding an O(|N(v)|²) pair scan.
    ///
    /// Relies on `graph.adj` holding only active vertices — a graph invariant,
    /// not enforced here.
    fn fill_count_of(&mut self, graph: &Graph, v: u32) -> u64 {
        // Bitset path is O(k · words) vs O(Σdeg) for the marker path below;
        // wins when avg_deg >> words ≈ n/64.
        if graph.bitset_words > 0 {
            return graph.fill_count_of_bs(v);
        }

        let nbrs_v = graph.adj[v as usize].as_slice();
        let k = nbrs_v.len();
        if k < 2 {
            return 0;
        }

        self.bump_stamp();
        let s = self.stamp;

        for &u in nbrs_v {
            self.marker[u as usize] = s;
        }

        // Each edge in the induced subgraph is counted from both endpoints,
        // so `doubled` sums to 2× the true edge count.
        //
        // SAFETY: every index below is a vertex id in [0, graph.len()) — the
        // graph only ever stores ids it was built with, and elimination
        // deactivates vertices rather than renumbering or removing them, so
        // that bound holds for the whole run. `marker` is allocated to
        // `graph.len()` by `FillScratch::new` and the scratch is built from
        // the same graph it is used with, so the two lengths agree; the
        // bounds-checked store into `marker[u]` a few lines above has already
        // proved that for every `u` this loop visits. Bounds checks cost ~30%
        // of this loop's instructions (a scalar gather on a u16 marker LLVM
        // won't vectorize).
        let mut doubled = 0u64;
        let marker = self.marker.as_ptr();
        for &u in nbrs_v {
            let adj_u = unsafe { graph.adj.get_unchecked(u as usize) };
            for &w in adj_u.iter() {
                // SAFETY: `w` is a vertex id, under the same bound as above.
                let m = unsafe { *marker.add(w as usize) };
                doubled += (m == s) as u64;
            }
        }
        let edge_count = doubled / 2;

        let total_pairs = (k as u64) * (k as u64 - 1) / 2;
        total_pairs - edge_count
    }
}

/// Snapshot `v`'s live neighbours into `nbrs_buf` and build the bag its
/// elimination emits: `v` first, then those neighbours.
fn take_bag(graph: &Graph, v: u32, nbrs_buf: &mut Vec<u32>) -> Vec<u32> {
    nbrs_buf.clear();
    graph.collect_live_nbrs_into(v, nbrs_buf);
    let mut bag = Vec::with_capacity(nbrs_buf.len() + 1);
    bag.push(v);
    bag.extend_from_slice(nbrs_buf);
    bag
}

/// Drain a heap in order, eliminating each popped vertex via
/// `remove_without_fill_nbrs`. Safe only when the caller has verified the
/// active residual is a clique (every remaining pop is simplicial).
fn drain_clique_tail<E: Ord + ElimEntry>(
    graph: &mut Graph,
    sink: &mut ElimSink<'_>,
    heap: &mut BinaryHeap<E>,
    nbrs_buf: &mut Vec<u32>,
) {
    while let Some(entry) = heap.pop() {
        let v = entry.vertex();
        let vi = v as usize;
        if !graph.active[vi] {
            continue;
        }
        let bag = take_bag(graph, v, nbrs_buf);
        graph.remove_without_fill_nbrs(v, nbrs_buf);
        sink.record(v, bag);
    }
}

/// What every elimination heap entry can be asked, whatever its ordering key:
/// which vertex it stands for, and the score it recorded when it was pushed.
trait ElimEntry {
    fn vertex(&self) -> u32;
    /// The fill or degree at push time. The skeleton compares it against a
    /// fresh measurement to spot an entry whose ordering key no longer holds.
    fn snapshot(&self) -> u64;
}

/// Priority → vertex buckets with O(log n) insert/remove and O(1) indexed
/// access into the min-key bucket. The min bucket *is* the tie set (no
/// secondary key), so a caller can sample from it directly — mirrors htd's
/// `PriorityQueue::topCollection`.
#[derive(Clone)]
pub(super) struct BucketMap {
    buckets: BTreeMap<u64, Vec<u32>>,
    position: Vec<Option<(u64, usize)>>,
}

impl BucketMap {
    fn with_capacity(n: usize) -> Self {
        BucketMap {
            buckets: BTreeMap::new(),
            position: vec![None; n],
        }
    }

    fn insert(&mut self, v: u32, key: u64) {
        let bucket = self.buckets.entry(key).or_default();
        let idx = bucket.len();
        bucket.push(v);
        self.position[v as usize] = Some((key, idx));
    }

    fn remove_vertex(&mut self, v: u32) {
        if let Some((key, idx)) = self.position[v as usize].take() {
            let bucket = self.buckets.get_mut(&key).expect("bucket missing");
            let last_idx = bucket.len() - 1;
            if idx != last_idx {
                let moved = bucket[last_idx];
                bucket[idx] = moved;
                self.position[moved as usize] = Some((key, idx));
            }
            bucket.pop();
            if bucket.is_empty() {
                self.buckets.remove(&key);
            }
        }
    }

    fn update(&mut self, v: u32, new_key: u64) {
        if let Some((cur_key, _)) = self.position[v as usize] {
            if cur_key == new_key {
                return;
            }
            self.remove_vertex(v);
        }
        self.insert(v, new_key);
    }

    fn min_bucket(&self) -> Option<(u64, &Vec<u32>)> {
        self.buckets.iter().next().map(|(k, v)| (*k, v))
    }

    fn key_of(&self, v: u32) -> Option<u64> {
        self.position[v as usize].map(|(key, _)| key)
    }
}

/// Fill counts for every active vertex via adj-based `FillScratch`, so the
/// O(n·d²) computation can be cached once and reused across multiple seeds.
pub(super) fn compute_initial_fill(graph: &Graph) -> Vec<u64> {
    let n = graph.len();
    let mut scratch = FillScratch::new(n);
    (0..n)
        .map(|v| {
            if graph.active[v] {
                scratch.fill_count_of(graph, v as u32)
            } else {
                0
            }
        })
        .collect()
}

/// Pick one vertex from `tie_set` with probability proportional to
/// `weight[v] + 1` (the `+1` avoids zero-weight vertices being unreachable).
/// A one-vertex tie set draws nothing at all, so the RNG stream depends only
/// on the ties the elimination actually had to break.
fn sample_tie_set(tie_set: &[u32], weight: &[u32], rng: &mut Xorshift64) -> u32 {
    debug_assert!(!tie_set.is_empty());
    if tie_set.len() == 1 {
        return tie_set[0];
    }
    let mut total: u64 = 0;
    for &v in tie_set {
        total += weight[v as usize] as u64 + 1;
    }
    // Compose two u32 draws into one u64 so the draw covers `total` up to 2^64.
    let hi = rng.next_u32() as u64;
    let lo = rng.next_u32() as u64;
    let r = ((hi << 32) | lo) % total;
    let mut acc: u64 = 0;
    for &v in tie_set {
        acc += weight[v as usize] as u64 + 1;
        if r < acc {
            return v;
        }
    }
    tie_set[tie_set.len() - 1]
}
