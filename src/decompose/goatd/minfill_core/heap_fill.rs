//! Min-fill elimination: repeatedly remove the active vertex whose removal adds
//! the fewest fill edges, ties broken by degree and then by the caller's salt.
//!
//! One instantiation of the greedy skeleton in `greedy`, and the schedule's
//! main order. Fill is costly enough to be maintained rather than recomputed
//! per pop: a seeding scan measures every active vertex, each elimination
//! re-scores its live neighbours, and a dirty flag lets an untouched entry pop
//! without a recount.
//!
//! Past the soft deadline the run continues in cheap mode — neighbours are
//! re-pushed with fill 0, so the rest of the elimination pops in degree order.
//! What it emits is still a complete decomposition, but no longer a min-fill
//! one, and nothing in the result says so.

use std::time::Instant;

use super::greedy::{AfterElim, ElimPolicy, Seeded, eliminate_greedy};
use super::*;
use crate::budget::expired;

/// Heap entry ordered ascending by (fill, degree, salt). The `fill` field is
/// duplicated out of the key so the stale-snapshot check can compare it
/// against a live recomputed fill without destructuring the `Reverse` tuple.
#[derive(Eq, PartialEq)]
pub(super) struct HeapEntry {
    pub key: (Reverse<u64>, Reverse<usize>, Reverse<u32>, Reverse<u32>),
    pub vertex: u32,
    pub fill: u64,
}

impl HeapEntry {
    pub(super) fn new(fill: u64, degree: usize, salt: u32, v: u32) -> Self {
        HeapEntry {
            key: (Reverse(fill), Reverse(degree), Reverse(salt), Reverse(v)),
            vertex: v,
            fill,
        }
    }
}

ord_by_key!(HeapEntry);

impl ElimEntry for HeapEntry {
    fn vertex(&self) -> u32 {
        self.vertex
    }
    fn snapshot(&self) -> u64 {
        self.fill
    }
}

/// Fill counts for every active vertex, leaving 0 for anything the scan did
/// not reach. The core measures everything up front and only then builds the
/// heap, so that a scan cut short by the deadline still yields an entry per
/// active vertex.
///
/// `deadline`/`hard_deadline` bound the scan itself, which on a very large
/// graph can consume most of a slot's budget on its own: `Bailed` means the
/// hard deadline passed and nothing should be eliminated, `CheapMode` means
/// the soft deadline cut the scan short and the run starts with incomplete
/// scores.
fn scan_fill(
    scratch: &mut FillScratch,
    graph: &Graph,
    fill_count: &mut [u64],
    deadline: Option<Instant>,
    hard_deadline: Option<Instant>,
) -> Seeded {
    let mut init_check = 0u32;
    for (v, slot) in fill_count.iter_mut().enumerate() {
        if !graph.active[v] {
            continue;
        }
        init_check += 1;
        if init_check >= DEADLINE_CHECK_STRIDE {
            init_check = 0;
            if expired(hard_deadline) {
                return Seeded::Bailed;
            }
            if expired(deadline) {
                return Seeded::CheapMode;
            }
        }
        *slot = scratch.fill_count_of(graph, v as u32);
    }
    Seeded::Ready
}

/// Greedy min-fill: rank by the number of fill edges eliminating a vertex
/// would add, breaking ties by degree and then by salt.
struct MinFill<'a> {
    heap: BinaryHeap<HeapEntry>,
    scratch: FillScratch,
    /// `dirty[v]` — a neighbour of `v` was eliminated since `v`'s entry was
    /// pushed, so that entry's fill snapshot may no longer hold. Clear means
    /// the snapshot is still exact and the pop can skip a recompute; a vertex
    /// with `k` neighbour-eliminations before its own pop is refreshed once
    /// per heap bounce instead of `k` times.
    dirty: Vec<bool>,
    salt: &'a [u32],
}

impl ElimPolicy for MinFill<'_> {
    type Entry = HeapEntry;

    const CHEAP_MODE: bool = true;
    const MAINTAIN_BITSET: bool = true;
    const ZERO_SCORE_IS_SIMPLICIAL: bool = true;

    fn heap(&mut self) -> &mut BinaryHeap<HeapEntry> {
        &mut self.heap
    }

    fn push(&mut self, graph: &Graph, v: u32, score: u64) {
        self.heap.push(HeapEntry::new(
            score,
            graph.degree(v),
            self.salt[v as usize],
            v,
        ));
    }

    fn live_score(&mut self, graph: &Graph, v: u32) -> u64 {
        self.scratch.fill_count_of(graph, v)
    }

    fn seed(
        &mut self,
        graph: &mut Graph,
        deadline: Option<Instant>,
        hard_deadline: Option<Instant>,
    ) -> Seeded {
        let mut fill_count: Vec<u64> = vec![0; graph.len()];
        let outcome = scan_fill(
            &mut self.scratch,
            graph,
            &mut fill_count,
            deadline,
            hard_deadline,
        );
        if matches!(outcome, Seeded::Bailed) {
            return outcome;
        }
        for (v, &fill) in fill_count.iter().enumerate() {
            if graph.active[v] {
                self.push(graph, v as u32, fill);
            }
        }
        outcome
    }

    fn rescore_on_pop(&mut self, graph: &Graph, v: u32) -> Option<u64> {
        std::mem::replace(&mut self.dirty[v as usize], false)
            .then(|| self.scratch.fill_count_of(graph, v))
    }

    fn after_eliminate(
        &mut self,
        graph: &Graph,
        nbrs: &[u32],
        cheap_mode: bool,
        deadline: Option<Instant>,
    ) -> AfterElim {
        if cheap_mode {
            // Fill accuracy is already abandoned: re-push each live neighbour
            // with a zero fill so the rest of the run pops in min-degree order.
            for &u in nbrs {
                if graph.active[u as usize] {
                    self.heap
                        .push(HeapEntry::new(0, graph.degree(u), self.salt[u as usize], u));
                }
            }
            return AfterElim::Continue;
        }
        // Eagerly re-score each affected neighbour and push a fresh entry.
        // Without this the heap picks by *stale* fill counts and can commit
        // to a vertex whose true fill is far above some unpopped candidate.
        // The dirty flag still guards the pop, because the neighbour's older
        // entry may surface before the new one.
        //
        // Checked inside this loop, not only between pops: each re-score is
        // superlinear in the neighbourhood, so on a dense graph one loop can
        // run for seconds on its own.
        for &u in nbrs {
            let ui = u as usize;
            if !graph.active[ui] {
                continue;
            }
            if expired(deadline) {
                if graph.num_active > CHEAP_MODE_MAX_ACTIVE {
                    return AfterElim::Bail;
                }
                return AfterElim::EnterCheapMode;
            }
            let live = self.scratch.fill_count_of(graph, u);
            self.push(graph, u, live);
            self.dirty[ui] = true;
        }
        AfterElim::Continue
    }
}

/// Eliminate every remaining active vertex from `graph` using the greedy
/// min-fill rule, recording the emitted bags (first vertex = eliminated, rest
/// = live neighbours) into `sink`.
///
/// `salt[v]` breaks (fill, degree) ties; `0` salt gives deterministic
/// vertex-id order, random values give diversification across seeds.
pub(crate) fn eliminate_minfill(
    graph: &mut Graph,
    salt: &[u32],
    sink: ElimSink<'_>,
    stop: ElimStop,
) -> ElimExit {
    let n = graph.len();
    assert_eq!(salt.len(), n);
    let mut policy = MinFill {
        heap: BinaryHeap::with_capacity(n),
        scratch: FillScratch::new(n),
        dirty: vec![false; n],
        salt,
    };
    eliminate_greedy(&mut policy, graph, sink, stop)
}
