//! The CSR graph every level of the hierarchy is made of, and the builder that
//! turns a local edge list into the finest one.
//!
//! Only the finest level has all-ones weights: coarsening sums the weight of
//! everything it contracts, so `vwgt` and `adjwgt` carry the fine graph's mass
//! upward and let the coarse levels stand in for it.

/// Vertices are 0-indexed.
pub(super) struct CsrGraph {
    pub(super) xadj: Vec<u32>,
    pub(super) adjncy: Vec<u32>,
    /// Number of fine vertices collapsed into this coarse vertex. Balance
    /// constraints are expressed in these units.
    pub(super) vwgt: Vec<u32>,
    /// Parallel to `adjncy`: total weight of the fine edges collapsed into that
    /// coarse edge. Gains and cuts are expressed in these units.
    pub(super) adjwgt: Vec<u32>,
}

impl CsrGraph {
    pub(super) fn num_vertices(&self) -> usize {
        self.xadj.len() - 1
    }

    /// Vertices plus arcs: what one pass over this graph touches, and the unit
    /// every phase of the bisection charges its passes in.
    ///
    /// The charge is taken once per pass rather than inside the loops that make
    /// one up, because several of those loops index `adjncy` directly instead of
    /// going through [`CsrGraph::neighbors`] — a charge on the accessor would
    /// miss most of the work the pass actually does.
    pub(super) fn pass_units(&self) -> u64 {
        (self.xadj.len() as u64).saturating_add(self.adjncy.len() as u64)
    }

    pub(super) fn neighbors(&self, v: usize) -> &[u32] {
        let start = self.xadj[v] as usize;
        let end = self.xadj[v + 1] as usize;
        &self.adjncy[start..end]
    }
}

/// `edges` are undirected pairs indexing into `0..n`; an endpoint outside that
/// range leaves no trace in the result.
///
/// Repeats collapse and every edge starts at weight 1, so multiplicity in
/// `edges` does not reach `adjwgt` — a weight above 1 only ever comes from
/// coarsening.
pub(super) fn build_csr(n: usize, edges: &[(u32, u32)]) -> CsrGraph {
    let mut adj_list: Vec<Vec<u32>> = vec![Vec::new(); n];
    for &(u, v) in edges {
        let (u, v) = (u as usize, v as usize);
        if u < n && v < n {
            adj_list[u].push(v as u32);
            adj_list[v].push(u as u32);
        }
    }
    for list in &mut adj_list {
        list.sort_unstable();
        list.dedup();
    }

    let mut xadj = Vec::with_capacity(n + 1);
    let mut adjncy = Vec::new();
    xadj.push(0u32);
    for list in &adj_list {
        adjncy.extend_from_slice(list);
        xadj.push(adjncy.len() as u32);
    }
    let adjwgt = vec![1u32; adjncy.len()];
    let vwgt = vec![1u32; n];
    CsrGraph {
        xadj,
        adjncy,
        vwgt,
        adjwgt,
    }
}
