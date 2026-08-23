//! Safe reduction rules for tree-decomposition preprocessing.
//!
//! Five fixed-point rules (ideas from htd + arboretum):
//!   - **islet**: a degree-0 vertex can be eliminated with an empty bag.
//!   - **twig**: a degree-1 vertex can be eliminated; its bag is {v, u}.
//!   - **series**: a degree-2 vertex with non-adjacent neighbours — add the
//!     fill edge, then eliminate; bag = {v, a, b}.
//!   - **simplicial**: a vertex whose live neighbours form a clique —
//!     eliminate with zero fill; bag = {v} ∪ N(v).
//!   - **almost-simplicial** (Bodlaender): a vertex `v` whose live neighbours
//!     form a clique except for one missing edge, *and* whose degree satisfies
//!     `deg(v) ≤ LB` for some valid lower bound `LB` on treewidth — here
//!     `LB = max deg over simplicial eliminations so far`. Adds one fill edge
//!     then eliminates; bag size = `deg(v)+1 ≤ LB+1`.
//!
//! Each rule is safe: re-inserting the emitted bags as a prefix onto any valid
//! TD of the reduced graph yields a valid TD of the original graph with the
//! same width (simplicial/series/almost-simplicial under the LB condition) or
//! no width increase (islet/twig).

use super::graph::Graph;
use super::minfill_core::ElimSteps;

/// Output of preprocessing. `Clone` so a single preprocess result can be
/// reused across multiple configs in the schedule — preprocessing is
/// deterministic (no salt/seed) so sharing is safe.
#[derive(Clone)]
pub(crate) struct Reduced {
    /// Reduced graph (still holds inactive slots for eliminated vertices).
    pub graph: Graph,
    /// The eliminations the rules already did, which a run over `graph`
    /// continues from.
    pub prefix: ElimSteps,
}

pub(crate) fn preprocess(mut graph: Graph) -> Reduced {
    let mut prefix = ElimSteps::default();
    // Running lower bound on tw(G), maintained across simplicial/series
    // eliminations; gates the almost-simplicial rule below.
    let mut tw_lb: usize = 0;

    loop {
        let mut fired = false;
        let n = graph.len();

        for v in 0..n {
            if !graph.active[v] {
                continue;
            }
            if graph.degree(v as u32) == 0 {
                let bag = vec![v as u32];
                graph.active[v] = false;
                graph.num_active -= 1;
                prefix.sink().record(v as u32, bag);
                fired = true;
            }
        }

        for v in 0..n {
            if !graph.active[v] {
                continue;
            }
            if graph.degree(v as u32) == 1 {
                let u = graph.live_neighbours(v as u32)[0];
                let bag = vec![v as u32, u];
                graph.remove_without_fill(v as u32);
                prefix.sink().record(v as u32, bag);
                fired = true;
            }
        }

        for v in 0..n {
            if !graph.active[v] {
                continue;
            }
            if graph.degree(v as u32) == 2 {
                let nbrs = graph.live_neighbours(v as u32);
                let (a, b) = (nbrs[0], nbrs[1]);
                let bag = vec![v as u32, a, b];
                if !graph.contains_edge(a, b) {
                    graph.eliminate(v as u32);
                    prefix.sink().record(v as u32, bag);
                    tw_lb = tw_lb.max(2);
                    fired = true;
                }
            }
        }

        for v in 0..n {
            if !graph.active[v] {
                continue;
            }
            if graph.degree(v as u32) >= 2 && graph.is_simplicial(v as u32) {
                let d = graph.degree(v as u32);
                let mut bag = Vec::with_capacity(d + 1);
                bag.push(v as u32);
                bag.extend(graph.live_neighbours(v as u32));
                graph.eliminate(v as u32);
                prefix.sink().record(v as u32, bag);
                tw_lb = tw_lb.max(d);
                fired = true;
            }
        }

        // Almost-simplicial pass — safe iff `deg(v) ≤ tw_lb`: the emitted bag
        // of size `deg(v)+1` then can't exceed the known LB on tw(G).
        if tw_lb >= 2 {
            for v in 0..n {
                if !graph.active[v] {
                    continue;
                }
                let d = graph.degree(v as u32);
                if d < 2 || d > tw_lb {
                    continue;
                }
                let Some((a, b)) = almost_simplicial_nonedge(&graph, v as u32) else {
                    continue;
                };
                graph.add_edge(a, b);
                let mut bag = Vec::with_capacity(d + 1);
                bag.push(v as u32);
                bag.extend(graph.live_neighbours(v as u32));
                graph.eliminate(v as u32);
                prefix.sink().record(v as u32, bag);
                tw_lb = tw_lb.max(d);
                fired = true;
            }
        }

        if !fired {
            break;
        }
    }

    Reduced { graph, prefix }
}

/// If v's live neighbourhood is a clique except for exactly one missing edge,
/// return that edge `(a, b)`. Otherwise return `None` (either simplicial,
/// caught by the simplicial pass earlier, or ≥ 2 missing edges).
fn almost_simplicial_nonedge(graph: &Graph, v: u32) -> Option<(u32, u32)> {
    let nbrs = graph.live_neighbours(v);
    let mut miss: Option<(u32, u32)> = None;
    for i in 0..nbrs.len() {
        for j in (i + 1)..nbrs.len() {
            if !graph.contains_edge(nbrs[i], nbrs[j]) {
                if miss.is_some() {
                    return None;
                }
                miss = Some((nbrs[i], nbrs[j]));
            }
        }
    }
    miss
}
