//! What the two Fiduccia-Mattheyses refiners share.
//!
//! [`multilevel_bisect`](super::multilevel_bisect) moves vertices of a graph and
//! [`multilevel_hg_bisect`](super::multilevel_hg_bisect) moves pins of a
//! hypergraph, and their gain models have nothing in common. What they do share
//! is the bookkeeping around a pass: the balance bound it works under, the
//! random partition it can start from, which prefix of a finished move sequence
//! survives, the queue that hands out the best-gain candidate, and the rule that
//! ends a pass that has stopped improving. Each of them reads vertex weights,
//! move records and gains a caller computed, so no graph or hypergraph type
//! reaches here — the container is shared, the gain model is not. They are
//! written once, and neither bisector has to know the other exists.
//!
//! # Where the two bisectors differ, and why
//!
//! Recorded here, once, because a difference written into one backend is a
//! claim about the other that a change to the other would falsify.
//!
//! - **What the caller asked for.** `multilevel_bisect`'s consumers want a
//!   small separator; `multilevel_hg_bisect`'s want a small hyperedge cut.
//!   The rest follows from this one.
//! - **One sweep against best-of-N.** The graph side takes a single
//!   well-refined pass. Minimum edge cut does not correlate with minimum
//!   separator width, so ranking restarts by cut would select against what its
//!   callers came for, and a well-refined pass gives lower treewidth on
//!   average. The hypergraph side keeps the best cut over restarts, the cut
//!   being the objective there; its restart count and its V-cycle count take
//!   the same square root of the effort budget, so a bigger budget splits
//!   between more restarts and more refinement of each instead of multiplying
//!   into either.
//! - **Restarts of the initial partition.** Fixed at 4 on the graph side; 6
//!   above 30 vertices and 4 below on the hypergraph side. Nothing in the tree
//!   records why.
//! - **The greedy-growing gain.** The graph side updates a score incrementally
//!   that ranks candidates but is not the cut reduction; the hypergraph side
//!   recomputes the exact gain each step, paying a scan of every unplaced
//!   vertex's incidences for it. Nothing in the tree records why.

use super::rng::Xorshift64;

/// The bisection of `n` vertices by index: the first half to side 0, the rest
/// to side 1.
///
/// The answer both bisectors fall back on when the partitioner has nothing to
/// work with — no edges at all, or a pass that put every vertex on one side.
/// Neither is a partition a caller can recurse into, and a caller that asked
/// for a bisection has to get two non-empty sides for any `n >= 2`.
pub(super) fn index_split(n: usize) -> Vec<u8> {
    let mut part = vec![0u8; n];
    part[n / 2..].fill(1);
    part
}

/// The bisection of `n` vertices that needs no partitioner at all, for the
/// three sizes where there is only one answer; `None` once there is a choice
/// to make.
pub(super) fn tiny_bisection(n: usize) -> Option<Vec<u8>> {
    match n {
        0 => Some(Vec::new()),
        1 => Some(vec![0]),
        2 => Some(vec![0, 1]),
        _ => None,
    }
}

/// The weight window either side of a bisection has to stay in under
/// `max_imbalance`, as `(min, max)`.
///
/// One bound, applied to both sides: whatever `max_imbalance` allows the heavy
/// side is denied to the light one. Weights are in `vwgt` units, so at coarse
/// levels a single vertex can be too heavy to move anywhere.
pub(super) fn balance_bounds(vwgt: &[u32], max_imbalance: f64) -> (u32, u32) {
    let total_weight: u32 = vwgt.iter().sum();
    let max_part_weight = ((total_weight as f64) * (0.5 + max_imbalance)) as u32;
    let min_part_weight = total_weight.saturating_sub(max_part_weight);
    (min_part_weight, max_part_weight)
}

/// The balance a Fiduccia-Mattheyses pass starts from: the weight already on
/// each side of the partition, and the window a move has to leave both sides
/// inside.
pub(super) struct FmBalance {
    /// Total vertex weight on each side.
    pub(super) weight: [u32; 2],
    /// The lightest a side may become.
    pub(super) min_part_weight: u32,
    /// The heaviest a side may become.
    pub(super) max_part_weight: u32,
}

/// Weigh `part` and read off the balance window, or `None` when there is
/// nothing for a pass to do — with two vertices or fewer, moving one cannot
/// improve a cut without emptying a side.
///
/// `n` is the vertex count of the graph `part` partitions; `vwgt` is that
/// graph's vertex weights.
pub(super) fn fm_balance(
    n: usize,
    vwgt: &[u32],
    part: &[u8],
    max_imbalance: f64,
) -> Option<FmBalance> {
    if n <= 2 {
        return None;
    }
    let (min_part_weight, max_part_weight) = balance_bounds(vwgt, max_imbalance);
    let mut weight = [0u32; 2];
    for v in 0..n {
        weight[part[v] as usize] += vwgt[v];
    }
    Some(FmBalance {
        weight,
        min_part_weight,
        max_part_weight,
    })
}

/// Fills side 0 in random order while the next vertex still fits under half the
/// total weight.
///
/// A vertex that does not fit is skipped and never reconsidered, so at coarse
/// levels — where vertex weights are large and uneven — side 0 can finish well
/// short of half. FM is what pulls the result back toward balance.
pub(super) fn random_bisection(vwgt: &[u32], rng: &mut Xorshift64) -> Vec<u8> {
    let n = vwgt.len();
    let total_weight: u32 = vwgt.iter().sum();
    let target = total_weight / 2;

    let mut perm: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = (rng.next_u64() as usize) % (i + 1);
        perm.swap(i, j);
    }

    let mut part = vec![1u8; n];
    let mut weight0: u32 = 0;
    for &v in &perm {
        if weight0 + vwgt[v] <= target {
            part[v] = 0;
            weight0 += vwgt[v];
        }
    }
    part
}

/// Keep the best prefix of a finished move sequence and undo everything after
/// it, reporting whether anything survived.
///
/// `moves` is the sequence in the order it was applied to `part`, and
/// `cumulative_gain[i]` is the running gain after move `i`. A strictly positive
/// best prefix gain is required, so a pass that only matched the starting cut
/// reports no improvement and unwinds completely rather than handing back an
/// equal-cut partition the caller would loop on. On `false` — an empty sequence
/// included — `part` comes back exactly as the pass found it.
pub(super) fn commit_best_prefix(
    moves: &[(usize, i32)],
    cumulative_gain: &[i64],
    part: &mut [u8],
) -> bool {
    if moves.is_empty() {
        return false;
    }

    let mut best_idx: i32 = -1;
    let mut best_prefix_gain: i64 = 0;
    for (i, &cg) in cumulative_gain.iter().enumerate() {
        if cg > best_prefix_gain {
            best_prefix_gain = cg;
            best_idx = i as i32;
        }
    }

    if best_prefix_gain <= 0 {
        for &(v, _) in moves.iter().rev() {
            part[v] = 1 - part[v];
        }
        return false;
    }

    for &(v, _) in moves[(best_idx as usize + 1)..].iter().rev() {
        part[v] = 1 - part[v];
    }

    true
}

/// Vertices bucketed by gain, so the best candidate is found without a scan.
///
/// Gain `g` lives in bucket `g + max_gain`, which centres the reachable range
/// on a `Vec` of `2 * max_gain + 1` buckets and leaves `max_idx` — the best
/// non-empty bucket, or `-1` when the queue is empty — as the only thing a
/// lookup has to consult. Each bucket is used as a stack: among equal gains the
/// most recently inserted or updated vertex comes out first, and after a move
/// that vertex is one of the neighbours the move just touched.
///
/// What a gain MEANS is the caller's business — this stores the number it is
/// handed and never computes one — which is what lets a graph refiner and a
/// hypergraph refiner share the container without sharing a gain model.
pub(super) struct GainBuckets {
    buckets: Vec<Vec<usize>>,
    /// Which bucket holds `v`, or `usize::MAX` when `v` is not queued.
    bucket_of: Vec<usize>,
    /// Where `v` sits inside its bucket, which is what makes removal O(1).
    /// Meaningful only while `bucket_of[v]` is set.
    pos_in_bucket: Vec<usize>,
    /// Added to a gain to get a bucket index.
    offset: i32,
    /// Highest non-empty bucket index, or `-1`.
    max_idx: i32,
}

impl GainBuckets {
    /// A queue over `n` vertices whose gains lie in `[-max_gain, max_gain]`.
    pub(super) fn new(n: usize, max_gain: i32) -> Self {
        let mut queue = GainBuckets::empty();
        queue.reset(n, max_gain);
        queue
    }

    /// A queue sized for nothing, for a caller that keeps one across passes and
    /// calls [`GainBuckets::reset`] at the head of each.
    pub(super) fn empty() -> Self {
        GainBuckets {
            buckets: Vec::new(),
            bucket_of: Vec::new(),
            pos_in_bucket: Vec::new(),
            offset: 0,
            max_idx: -1,
        }
    }

    /// Empty the queue and re-size it for `n` vertices and `max_gain`, keeping
    /// the allocations the last pass left behind.
    pub(super) fn reset(&mut self, n: usize, max_gain: i32) {
        for bucket in self.buckets.iter_mut() {
            bucket.clear();
        }
        self.buckets
            .resize_with((2 * max_gain + 1) as usize, Vec::new);
        self.bucket_of.clear();
        self.bucket_of.resize(n, usize::MAX);
        self.pos_in_bucket.clear();
        self.pos_in_bucket.resize(n, usize::MAX);
        self.offset = max_gain;
        self.max_idx = -1;
    }

    /// Is `v` queued?
    pub(super) fn contains(&self, v: usize) -> bool {
        self.bucket_of[v] != usize::MAX
    }

    /// The best-gain vertex, left in the queue; `None` when nothing is queued.
    pub(super) fn top(&self) -> Option<usize> {
        let idx = usize::try_from(self.max_idx).ok()?;
        self.buckets[idx].last().copied()
    }

    /// Queue `v` at `gain`. Appending is what makes the tie-break the most
    /// recent vertex rather than an arbitrary one.
    pub(super) fn insert(&mut self, v: usize, gain: i32) {
        let idx = (gain + self.offset) as usize;
        self.pos_in_bucket[v] = self.buckets[idx].len();
        self.buckets[idx].push(v);
        self.bucket_of[v] = idx;
        if idx as i32 > self.max_idx {
            self.max_idx = idx as i32;
        }
    }

    /// Take `v` out of the queue; a no-op for a vertex that is not in it.
    pub(super) fn remove(&mut self, v: usize) {
        let idx = self.bucket_of[v];
        if idx == usize::MAX {
            return;
        }
        let pos = self.pos_in_bucket[v];
        let bucket = &mut self.buckets[idx];
        bucket.swap_remove(pos);
        if pos < bucket.len() {
            let moved = bucket[pos];
            self.pos_in_bucket[moved] = pos;
        }
        self.bucket_of[v] = usize::MAX;
        self.pos_in_bucket[v] = usize::MAX;

        if bucket.is_empty() && idx as i32 == self.max_idx {
            while self.max_idx >= 0 && self.buckets[self.max_idx as usize].is_empty() {
                self.max_idx -= 1;
            }
        }
    }

    /// Re-file `v` under `new_gain`, which also puts it at the head of the
    /// tie-break among its new equals.
    pub(super) fn update(&mut self, v: usize, new_gain: i32) {
        self.remove(v);
        self.insert(v, new_gain);
    }
}

/// How long a pass has gone without bettering the best running gain it has
/// seen, and how long it is allowed to.
///
/// Both refiners stop short of the textbook pass, which moves every vertex
/// before rolling back to the best prefix: a run of losing moves is what climbs
/// out of a local minimum, but past the limit the rest of the pass is a rollback
/// nobody is paying for.
pub(super) struct Stall {
    limit: usize,
    since_improvement: usize,
    best_gain: i64,
}

impl Stall {
    /// `limit` consecutive moves without an improvement end the pass.
    pub(super) fn new(limit: usize) -> Self {
        Stall {
            limit,
            since_improvement: 0,
            best_gain: 0,
        }
    }

    /// Record the running gain after a move, reporting whether the pass has
    /// stalled.
    pub(super) fn record(&mut self, running_gain: i64) -> bool {
        if running_gain > self.best_gain {
            self.best_gain = running_gain;
            self.since_improvement = 0;
            false
        } else {
            self.since_improvement += 1;
            self.since_improvement >= self.limit
        }
    }
}
