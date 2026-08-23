//! Min-degree elimination: repeatedly remove the active vertex of lowest
//! current degree, ties broken by the caller's salt and then by vertex id.
//!
//! One instantiation of the greedy skeleton in `greedy`. `width_opt` runs it as
//! a slot of the elimination schedule; the bags it emits through the sink are
//! what a tree decomposition — and then a vtree — is built from.

use super::greedy::{ElimPolicy, eliminate_greedy};
use super::*;

/// Heap entry for min-degree — (degree, salt, vertex) ascending.
#[derive(Eq, PartialEq)]
pub(super) struct DegEntry {
    /// Ordering key: `(degree, salt, vertex)`.
    pub key: (Reverse<u64>, Reverse<u32>, Reverse<u32>),
    pub vertex: u32,
    /// Duplicated out of `key` so the stale-snapshot guard
    /// (`graph.degree(v) != degree`) can read it without destructuring the
    /// `Reverse`-wrapped tuple.
    pub degree: u64,
}

impl DegEntry {
    pub(super) fn new(degree: u64, salt: u32, v: u32) -> Self {
        DegEntry {
            key: (Reverse(degree), Reverse(salt), Reverse(v)),
            vertex: v,
            degree,
        }
    }
}

ord_by_key!(DegEntry);

impl ElimEntry for DegEntry {
    fn vertex(&self) -> u32 {
        self.vertex
    }
    fn snapshot(&self) -> u64 {
        self.degree
    }
}

/// Greedy min-degree: rank by current degree, break ties by salt. This is the
/// skeleton's plainest instance — it takes every default, including owing its
/// neighbours nothing when a vertex is eliminated. The stale-snapshot guard
/// corrects an out-of-date entry when it surfaces, which keeps the heap at
/// O(n) entries instead of accumulating one per degree change.
struct MinDegree<'a> {
    heap: BinaryHeap<DegEntry>,
    salt: &'a [u32],
}

impl ElimPolicy for MinDegree<'_> {
    type Entry = DegEntry;

    const CHEAP_MODE: bool = true;
    // Degree is a single lookup either way, so the bitset would buy nothing.
    const MAINTAIN_BITSET: bool = false;
    // Ranking by degree says nothing about whether N(v) is already a clique.
    const ZERO_SCORE_IS_SIMPLICIAL: bool = false;

    fn heap(&mut self) -> &mut BinaryHeap<DegEntry> {
        &mut self.heap
    }

    fn push(&mut self, _: &Graph, v: u32, score: u64) {
        self.heap
            .push(DegEntry::new(score, self.salt[v as usize], v));
    }

    fn live_score(&mut self, graph: &Graph, v: u32) -> u64 {
        graph.degree(v) as u64
    }
}

/// Pure min-degree elimination — ranks by (degree, salt, vertex). Cheaper
/// than min-fill because it skips the fill recomputation step. Sometimes wins
/// on graphs where the leading vertex choice is dominated by current degree
/// (sparse graphs, long chains) rather than fill.
pub(crate) fn eliminate_mindegree(
    graph: &mut Graph,
    salt: &[u32],
    sink: ElimSink<'_>,
    stop: ElimStop,
) -> ElimExit {
    let n = graph.len();
    assert_eq!(salt.len(), n);
    let mut policy = MinDegree {
        heap: BinaryHeap::with_capacity(n),
        salt,
    };
    eliminate_greedy(&mut policy, graph, sink, stop)
}
