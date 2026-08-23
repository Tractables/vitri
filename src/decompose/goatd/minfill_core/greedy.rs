//! The elimination skeleton the heap-driven greedy cores share.
//!
//! Min-fill and min-degree are one algorithm with two different notions of
//! "cheapest vertex". They differ in how a vertex is scored, what a heap
//! entry carries, and what they owe a just-removed vertex's neighbours;
//! everything else — the pop loop, lazy deletion, deadline handling, the
//! stale-snapshot guard, bag emission, the width-bound abort, the
//! clique-tail drain — is written once here and specialized through
//! [`ElimPolicy`].
//!
//! The policy is a type parameter rather than a trait object: each core still
//! monomorphizes to the loop it had as a standalone function, with every hook
//! a direct call.
//!
//! The two sampling cores in `sampling.rs` deliberately stay outside this
//! skeleton.

use std::collections::BinaryHeap;
use std::time::Instant;

use super::{
    CHEAP_MODE_MAX_ACTIVE, DEADLINE_CHECK_STRIDE, ElimEntry, ElimExit, ElimSink, ElimStop, Graph,
    drain_clique_tail, exceeds_width_bound, take_bag,
};
use crate::budget::expired;

/// How a core's initial scoring pass ended.
pub(super) enum Seeded {
    /// The heap holds every active vertex with a true score; run normally.
    Ready,
    /// The soft deadline passed during the scan, so some scores are missing
    /// and the run starts already degraded (see [`ElimPolicy::CHEAP_MODE`]).
    CheapMode,
    /// The hard deadline passed during the scan; nothing was eliminated.
    Bailed,
}

/// What the skeleton does next once a core has reacted to an elimination.
pub(super) enum AfterElim {
    /// Carry on with the next pop.
    Continue,
    /// The soft deadline passed mid-update: abandon score maintenance and
    /// finish the run in cheap mode. Only a core with
    /// [`ElimPolicy::CHEAP_MODE`] set may ask for this.
    EnterCheapMode,
    /// The residual is too large to finish even in cheap mode. The caller
    /// emits an emergency path decomposition over what is left.
    Bail,
}

/// The parts of a greedy heap elimination that genuinely differ per core.
///
/// A policy owns its heap and its scoring scratch; the skeleton owns the
/// graph, the emitted bags and the loop. "Score" means whatever that core
/// ranks by — fill edges for min-fill, degree for min-degree — carried as
/// `u64` because a fill count can exceed `u32` on a dense graph.
///
/// Each entry a core pushes carries a *snapshot* of the true score at push
/// time. A snapshot can go stale when a neighbour is eliminated, so the
/// skeleton offers each popped entry back to
/// [`rescore_on_pop`](Self::rescore_on_pop) before acting on it; a moved
/// score means the entry is stale, and it is re-pushed instead of acted on.
///
/// Only [`heap`](Self::heap), [`push`](Self::push) and
/// [`live_score`](Self::live_score) have to be written. The defaults below
/// are the simplest core there is — seed the heap in one pass, re-score every
/// pop, owe the neighbours nothing — and each override marks a real
/// difference between the cores.
pub(super) trait ElimPolicy {
    /// This core's heap entry.
    type Entry: Ord + ElimEntry;

    /// Whether this core degrades instead of giving up when the soft deadline
    /// passes. A core with cheap mode stops maintaining scores and keeps
    /// eliminating in whatever order the stale heap pops, so it always emits
    /// a complete decomposition; it enforces the hard deadline only while
    /// degraded. A core without it has no degraded state and enforces the
    /// hard deadline directly, on the same stride.
    const CHEAP_MODE: bool;

    /// Whether this core keeps the graph's dense-vertex bitset current. Worth
    /// it where scoring is superlinear in the neighbourhood (min-fill), where
    /// the bitset turns a fill count into an AND-and-popcount; pointless where
    /// scoring is a single degree lookup.
    const MAINTAIN_BITSET: bool;

    /// Whether a score of 0 proves N(v) is already a clique, so eliminating
    /// `v` adds no fill edge and the fill machinery can be skipped. True for
    /// the cores that rank by fill; the min-degree cores have no score-driven
    /// fast path and leave it false.
    const ZERO_SCORE_IS_SIMPLICIAL: bool;

    /// The heap this core pops from and pushes to.
    fn heap(&mut self) -> &mut BinaryHeap<Self::Entry>;

    /// Push `v` with a freshly measured `score`.
    fn push(&mut self, graph: &Graph, v: u32, score: u64);

    /// `v`'s true score in the current graph.
    fn live_score(&mut self, graph: &Graph, v: u32) -> u64;

    /// Score every active vertex and fill the heap. The default scores each
    /// vertex as it reaches it; a core whose scoring pass is expensive enough
    /// to need its own deadline handling overrides this.
    fn seed(&mut self, graph: &mut Graph, _: Option<Instant>, _: Option<Instant>) -> Seeded {
        for v in 0..graph.len() {
            if graph.active[v] {
                let score = self.live_score(graph, v as u32);
                self.push(graph, v as u32, score);
            }
        }
        Seeded::Ready
    }

    /// Next candidate: its vertex and the score snapshot its entry recorded.
    fn pop(&mut self) -> Option<(u32, u64)> {
        self.heap().pop().map(|e| (e.vertex(), e.snapshot()))
    }

    /// `v`'s live score, if its popped snapshot has to be re-checked at all.
    /// The default re-scores unconditionally, which is right when a score is a
    /// single lookup; a core whose scoring is expensive tracks which vertices
    /// were disturbed and returns `None` for the rest.
    fn rescore_on_pop(&mut self, graph: &Graph, v: u32) -> Option<u64> {
        Some(self.live_score(graph, v))
    }

    /// React to `v`'s elimination; `nbrs` were its live neighbours. The
    /// default owes them nothing and lets the stale-snapshot guard sort them
    /// out when they surface.
    fn after_eliminate(
        &mut self,
        _graph: &Graph,
        _nbrs: &[u32],
        _cheap_mode: bool,
        _deadline: Option<Instant>,
    ) -> AfterElim {
        AfterElim::Continue
    }
}

/// Eliminate every remaining active vertex from `graph`, always taking the
/// cheapest one `policy` offers.
///
/// Records one bag per elimination into `sink` (eliminated vertex first, then
/// its live neighbours). A bag wider than `stop.width_bound` ends the run with
/// [`ElimExit::WidthAborted`] rather than pay for a decomposition nobody will
/// pick.
pub(super) fn eliminate_greedy<P: ElimPolicy>(
    policy: &mut P,
    graph: &mut Graph,
    mut sink: ElimSink<'_>,
    stop: ElimStop,
) -> ElimExit {
    let ElimStop {
        deadline,
        hard_deadline,
        width_bound,
    } = stop;
    // Preprocessing may have removed low-degree vertices, pushing density past
    // the bitset break-even even when the graph was built in adj-only mode.
    // Check once before the initial scan, a major hotspot on dense residuals.
    if P::MAINTAIN_BITSET && graph.should_promote_bitset() {
        graph.promote_bitset();
    }

    let mut cheap_mode = match policy.seed(graph, deadline, hard_deadline) {
        Seeded::Ready => false,
        Seeded::CheapMode => true,
        Seeded::Bailed => return ElimExit::DeadlineBailed,
    };

    let mut nbrs_buf: Vec<u32> = Vec::with_capacity(graph.len());
    let mut check_counter = 0u32;
    // A clique residual stays a clique as vertices are removed from it, so
    // this only needs to latch true once — that is what lets the tail drain
    // skip all scoring work on the giant-clique endgame.
    let mut clique_residual = false;

    while let Some((v, snapshot)) = policy.pop() {
        if !graph.active[v as usize] {
            continue; // lazy deletion: v was already eliminated
        }

        if cheap_mode {
            // Score maintenance is off, so the only thing left to respect is
            // the hard deadline — checked every pop, because a single cheap
            // elimination on a dense residual can still take a while.
            if expired(hard_deadline) {
                return ElimExit::DeadlineBailed;
            }
        } else {
            check_counter += 1;
            if check_counter >= DEADLINE_CHECK_STRIDE {
                check_counter = 0;
                if P::CHEAP_MODE {
                    if expired(deadline) {
                        // On a large residual even a cheap elimination can
                        // overshoot the hard deadline by seconds, so there is
                        // nothing to gain from degrading — bail straight to
                        // the emergency path decomposition.
                        if graph.num_active > CHEAP_MODE_MAX_ACTIVE {
                            return ElimExit::DeadlineBailed;
                        }
                        cheap_mode = true;
                    }
                } else if expired(hard_deadline) {
                    return ElimExit::DeadlineBailed;
                }
                // Fill edges can push an initially sparse graph past the
                // density break-even mid-elimination, turning scoring into
                // the hotspot.
                if P::MAINTAIN_BITSET && graph.should_promote_bitset() {
                    graph.promote_bitset();
                }
            }
        }

        // Stale-snapshot guard: re-score a popped vertex and, if it moved,
        // re-push and let the heap decide again. Skipped in cheap mode, where
        // score accuracy is already abandoned.
        if !cheap_mode
            && let Some(live) = policy.rescore_on_pop(graph, v)
            && live != snapshot
        {
            policy.push(graph, v, live);
            continue;
        }

        let bag = take_bag(graph, v, &mut nbrs_buf);
        let bag_len = bag.len();

        if !clique_residual && graph.is_residual_clique() {
            clique_residual = true;
        }

        let simplicial = P::ZERO_SCORE_IS_SIMPLICIAL && !cheap_mode && snapshot == 0;
        if clique_residual || simplicial {
            graph.remove_without_fill_nbrs(v, &nbrs_buf);
        } else {
            graph.eliminate_with_nbrs(v, &nbrs_buf);
        }
        sink.record(v, bag);

        if exceeds_width_bound(bag_len, width_bound) {
            return ElimExit::WidthAborted;
        }

        if clique_residual {
            // The drain below will put every remaining vertex in one bag;
            // check the bound against that now rather than after draining.
            if exceeds_width_bound(graph.num_active, width_bound) {
                return ElimExit::WidthAborted;
            }
            drain_clique_tail(graph, &mut sink, policy.heap(), &mut nbrs_buf);
            return ElimExit::Natural;
        }

        match policy.after_eliminate(graph, &nbrs_buf, cheap_mode, deadline) {
            AfterElim::Continue => {}
            AfterElim::EnterCheapMode => {
                debug_assert!(P::CHEAP_MODE, "core without cheap mode asked to enter it");
                if P::CHEAP_MODE {
                    cheap_mode = true;
                }
            }
            AfterElim::Bail => return ElimExit::DeadlineBailed,
        }
    }
    ElimExit::Natural
}
