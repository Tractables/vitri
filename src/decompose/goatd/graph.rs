//! Mutable graph for elimination-based tree-decomposition.
//!
//! Adjacency is `Vec<Vec<u32>>` rather than `Vec<FxHashSet<u32>>`: Vec
//! iteration is more cache-friendly than hashbrown bucket scanning at the
//! degrees min-fill's hot path sees on CNF-derived graphs. Vertices are never
//! removed from the top-level vector; elimination marks them inactive and
//! clears their row.
//!
//! Dense, small graphs additionally maintain a flat bitset adjacency
//! alongside the Vec; most methods below have a bitset-mode and a
//! sparse-mode path.

/// Maximum graph size for which full bitset adjacency is maintained.
/// At n = 16384: 16384 * 256 words * 8 bytes = 32 MB per graph.
const BITSET_THRESH: usize = 16384;

/// Mutable graph used by goatd during preprocessing, min-fill, and nested
/// dissection. Supports active/inactive vertices for constant-time elimination.
#[derive(Clone)]
pub(crate) struct Graph {
    pub adj: Vec<Vec<u32>>,
    pub active: Vec<bool>,
    pub num_active: usize,
    /// Count of undirected edges among active vertices. Enables O(1)
    /// clique-residual detection: the residual is complete iff
    /// `num_edges == num_active*(num_active-1)/2`.
    pub num_edges: usize,
    /// Stamp-marker scratch for deduping fill-edge additions in
    /// O(Σdeg + k²) instead of O(k²·deg_avg). u16 halves memory footprint vs
    /// u32; the stamp wraps and clears the marker array when it does.
    elim_marker: Vec<u16>,
    elim_stamp: u16,
    /// Flat bitset adjacency: vertex `v` occupies words
    /// `v * bitset_words .. (v+1) * bitset_words`; bit `u` in that slice is
    /// set iff edge (v, u) exists. Empty when bitset mode is disabled.
    pub bitset: Vec<u64>,
    /// Number of u64 words per vertex in `bitset`. 0 iff bitset is disabled.
    pub bitset_words: usize,
}

impl Graph {
    pub(crate) fn with_capacity(n: usize) -> Self {
        Graph {
            adj: vec![Vec::new(); n],
            active: vec![true; n],
            num_active: n,
            num_edges: 0,
            elim_marker: vec![0u16; n],
            elim_stamp: 0,
            bitset: Vec::new(),
            bitset_words: 0,
        }
    }

    pub(crate) fn from_edges(n: u32, edges: &[(u32, u32)]) -> Self {
        let n = n as usize;
        let mut g = Graph::with_capacity(n);
        for &(u, v) in edges {
            if (u as usize) < n && (v as usize) < n && u != v && !g.adj[u as usize].contains(&v) {
                g.adj[u as usize].push(v);
                g.adj[v as usize].push(u);
                g.num_edges += 1;
            }
        }
        if n <= BITSET_THRESH && g.num_edges.saturating_mul(128) > n.saturating_mul(n) {
            let w = n.div_ceil(64);
            g.bitset = vec![0u64; n * w];
            g.bitset_words = w;
            for v in 0..n {
                for &u in g.adj[v].iter() {
                    g.bitset[v * w + u as usize / 64] |= 1u64 << (u as usize % 64);
                }
            }
        }
        g
    }

    /// True when promoting from adj-only to bitset-assisted representation is
    /// worthwhile: density has crossed the break-even where bitset's
    /// O(k · words) beats the marker path's O(k · avg_deg). With
    /// `avg_deg = 2·num_edges / num_active` and `words ≈ n/64`, that break-even
    /// is `128·num_edges > n · num_active` — `num_active`, not `n`, so
    /// promotion still fires when fill edges densify the graph mid-elimination
    /// even though `from_edges` saw it as sparse.
    pub(super) fn should_promote_bitset(&self) -> bool {
        if self.bitset_words > 0 {
            return false;
        }
        let n = self.adj.len();
        if n == 0 || n > BITSET_THRESH {
            return false;
        }
        self.num_edges.saturating_mul(128) > n.saturating_mul(self.num_active.max(1))
    }

    /// Allocate and populate the bitset adjacency from `adj`, switching the
    /// graph into bitset mode. After this, `adj` is no longer maintained, so
    /// a caller that reads `graph.adj` directly must not call this mid-loop.
    pub(super) fn promote_bitset(&mut self) {
        debug_assert_eq!(self.bitset_words, 0);
        let n = self.adj.len();
        if n == 0 || n > BITSET_THRESH {
            return;
        }
        let w = n.div_ceil(64);
        let mut bs = vec![0u64; n * w];
        for v in 0..n {
            if !self.active[v] {
                continue;
            }
            for &u in self.adj[v].iter() {
                bs[v * w + u as usize / 64] |= 1u64 << (u as usize % 64);
            }
        }
        self.bitset = bs;
        self.bitset_words = w;
    }

    /// Clone preserving only the bitset; adj rows are allocated empty. Only
    /// valid when `bitset_words > 0`.
    pub(super) fn clone_bitset_only(&self) -> Self {
        debug_assert!(self.bitset_words > 0);
        Graph {
            adj: vec![Vec::new(); self.adj.len()],
            active: self.active.clone(),
            num_active: self.num_active,
            num_edges: self.num_edges,
            elim_marker: self.elim_marker.clone(),
            elim_stamp: self.elim_stamp,
            bitset: self.bitset.clone(),
            bitset_words: self.bitset_words,
        }
    }

    /// Add edge (u, v) using the bitset for O(1) existence check. Assumes
    /// `bitset_words > 0`.
    fn add_edge_bs(&mut self, u: u32, v: u32) -> bool {
        if u == v {
            return false;
        }
        let ui = u as usize;
        let vi = v as usize;
        let w = self.bitset_words;
        let word_u = ui / 64;
        let bit_u = 1u64 << (ui % 64);
        if self.bitset[vi * w + word_u] & bit_u != 0 {
            return false;
        }
        self.bitset[vi * w + word_u] |= bit_u;
        self.bitset[ui * w + vi / 64] |= 1u64 << (vi % 64);
        self.adj[ui].push(v);
        self.adj[vi].push(u);
        self.num_edges += 1;
        true
    }

    pub(crate) fn len(&self) -> usize {
        self.adj.len()
    }

    pub(crate) fn degree(&self, v: u32) -> usize {
        if self.bitset_words > 0 {
            let vi = v as usize;
            let w = self.bitset_words;
            let vb = vi * w;
            self.bitset[vb..vb + w]
                .iter()
                .map(|x| x.count_ones() as usize)
                .sum()
        } else {
            self.adj[v as usize].len()
        }
    }

    pub(crate) fn collect_live_nbrs_into(&self, v: u32, buf: &mut Vec<u32>) {
        if self.bitset_words > 0 {
            let vi = v as usize;
            let w = self.bitset_words;
            let vb = vi * w;
            for j in 0..w {
                let mut bits = self.bitset[vb + j];
                while bits != 0 {
                    let lsb = bits.trailing_zeros() as usize;
                    buf.push((j * 64 + lsb) as u32);
                    bits &= bits - 1;
                }
            }
        } else {
            buf.extend_from_slice(&self.adj[v as usize]);
        }
    }

    pub(crate) fn contains_edge(&self, u: u32, v: u32) -> bool {
        if self.bitset_words > 0 {
            let w = self.bitset_words;
            let vi = v as usize;
            self.bitset[u as usize * w + vi / 64] & (1u64 << (vi % 64)) != 0
        } else {
            self.adj[u as usize].contains(&v)
        }
    }

    pub(crate) fn add_edge(&mut self, u: u32, v: u32) -> bool {
        if u == v {
            return false;
        }
        if self.bitset_words > 0 {
            self.add_edge_bs(u, v)
        } else {
            if self.adj[u as usize].contains(&v) {
                return false;
            }
            self.adj[u as usize].push(v);
            self.adj[v as usize].push(u);
            self.num_edges += 1;
            true
        }
    }

    /// Return a copy of `v`'s live neighbour list. In bitset mode this reads
    /// set bits directly and is correct even though `adj` itself goes stale.
    pub(crate) fn live_neighbours(&self, v: u32) -> Vec<u32> {
        if self.bitset_words > 0 {
            let mut buf = Vec::new();
            self.collect_live_nbrs_into(v, &mut buf);
            buf
        } else {
            self.adj[v as usize].clone()
        }
    }

    pub(crate) fn eliminate(&mut self, v: u32) -> Vec<u32> {
        let neighbours = self.live_neighbours(v);
        self.eliminate_with_nbrs(v, &neighbours);
        neighbours
    }

    /// Eliminate vertex `v` given its pre-collected live neighbours. Avoids
    /// the extra `live_neighbours` allocation when the caller already has
    /// them.
    pub(crate) fn eliminate_with_nbrs(&mut self, v: u32, neighbours: &[u32]) {
        if self.bitset_words > 0 {
            self.eliminate_with_nbrs_bs(v, neighbours);
        } else {
            self.eliminate_with_nbrs_marker(v, neighbours);
        }
    }

    fn eliminate_with_nbrs_bs(&mut self, v: u32, neighbours: &[u32]) {
        let vi = v as usize;
        let w = self.bitset_words;
        let vb = vi * w;
        let mut pushes: usize = 0;

        for &u_raw in neighbours {
            let u = u_raw as usize;
            let ub = u * w;
            // The symmetric fill edge (bitset[wj] gaining bit u) is set when
            // wj's own outer-loop iteration runs, not here — bitset[wj] still
            // lacks bit u at that point, so u still shows up in wj's mask.
            for j in 0..w {
                let mut fill_mask = self.bitset[vb + j] & !self.bitset[ub + j];
                if j == vi / 64 {
                    fill_mask &= !(1u64 << (vi % 64));
                }
                if j == u / 64 {
                    fill_mask &= !(1u64 << (u % 64));
                }
                self.bitset[ub + j] |= fill_mask;
                pushes += fill_mask.count_ones() as usize;
            }
            self.bitset[ub + vi / 64] &= !(1u64 << (vi % 64));
        }

        for j in 0..w {
            self.bitset[vb + j] = 0;
        }
        if self.active[vi] {
            self.active[vi] = false;
            self.num_active -= 1;
        }
        self.num_edges -= neighbours.len();
        self.num_edges += pushes / 2;
    }

    fn eliminate_with_nbrs_marker(&mut self, v: u32, neighbours: &[u32]) {
        let marker = self.elim_marker.as_mut_slice();
        let mut pushes: usize = 0;
        for &u_raw in neighbours {
            let u = u_raw as usize;
            self.elim_stamp = self.elim_stamp.wrapping_add(1);
            if self.elim_stamp == 0 {
                marker.fill(0);
                self.elim_stamp = 1;
            }
            let s = self.elim_stamp;
            let row = &mut self.adj[u];
            let mut v_pos = usize::MAX;
            for (idx, &w) in row.iter().enumerate() {
                v_pos = if w == v { idx } else { v_pos };
                marker[w as usize] = s;
            }
            if v_pos != usize::MAX {
                row.swap_remove(v_pos);
            }
            marker[u] = s;
            for &w in neighbours {
                let wi = w as usize;
                if marker[wi] != s {
                    marker[wi] = s;
                    row.push(w);
                    pushes += 1;
                }
            }
        }
        self.adj[v as usize].clear();
        if self.active[v as usize] {
            self.active[v as usize] = false;
            self.num_active -= 1;
        }
        self.num_edges -= neighbours.len();
        self.num_edges += pushes / 2;
    }

    /// Remove vertex `v` without filling its neighbourhood — safe only when
    /// the caller already knows `v`'s removal cannot need a fill edge.
    /// Returns the vertex's live neighbours.
    pub(crate) fn remove_without_fill(&mut self, v: u32) -> Vec<u32> {
        let neighbours = self.live_neighbours(v);
        self.remove_without_fill_nbrs(v, &neighbours);
        neighbours
    }

    /// Remove `v`, given its live neighbours, without filling — safe only
    /// when the caller has verified N(v) is already a clique (no fill edges
    /// needed). Cheaper than `eliminate_with_nbrs`: no stamp-marker work.
    pub(crate) fn remove_without_fill_nbrs(&mut self, v: u32, nbrs: &[u32]) {
        let vi = v as usize;
        if self.bitset_words > 0 {
            let w = self.bitset_words;
            for &u in nbrs {
                self.bitset[u as usize * w + vi / 64] &= !(1u64 << (vi % 64));
            }
            let vb = vi * w;
            for j in 0..w {
                self.bitset[vb + j] = 0;
            }
        } else {
            for &u in nbrs {
                let row = &mut self.adj[u as usize];
                if let Some(pos) = row.iter().position(|&x| x == v) {
                    row.swap_remove(pos);
                }
            }
            self.adj[vi].clear();
        }
        if self.active[vi] {
            self.active[vi] = false;
            self.num_active -= 1;
        }
        self.num_edges -= nbrs.len();
    }

    /// O(1) check: is the active residual a complete graph?
    pub(crate) fn is_residual_clique(&self) -> bool {
        let n = self.num_active;
        self.num_edges == n * (n - 1) / 2
    }

    /// Is the live neighbourhood of `v` a clique?
    pub(crate) fn is_simplicial(&self, v: u32) -> bool {
        if self.bitset_words > 0 {
            let vi = v as usize;
            let w = self.bitset_words;
            let vb = vi * w;
            let vbs = &self.bitset[vb..vb + w];
            for j in 0..w {
                let mut word = vbs[j];
                while word != 0 {
                    let lsb = word.trailing_zeros() as usize;
                    let u = j * 64 + lsb;
                    let ub = u * w;
                    // v is not simplicial iff some other neighbour w2 of v is
                    // not a neighbour of u, i.e. N(v) & ~N(u) has a bit set
                    // besides u's own.
                    for (l, &v_word) in vbs.iter().enumerate() {
                        let non_nbrs = v_word & !self.bitset[ub + l];
                        let masked = if l == u / 64 {
                            non_nbrs & !(1u64 << (u % 64))
                        } else {
                            non_nbrs
                        };
                        if masked != 0 {
                            return false;
                        }
                    }
                    word &= word - 1;
                }
            }
            true
        } else {
            let neighbours = &self.adj[v as usize];
            for i in 0..neighbours.len() {
                for j in (i + 1)..neighbours.len() {
                    if !self.contains_edge(neighbours[i], neighbours[j]) {
                        return false;
                    }
                }
            }
            true
        }
    }

    /// Fill count of `v` via bitset intersection: for each u ∈ N(v),
    /// popcount(bitset[u] & bitset[v]) counts N(v) members adjacent to u;
    /// summed and halved gives edges within N(v). O(k · words) vs
    /// O(k · avg_deg) for the marker path.
    pub(crate) fn fill_count_of_bs(&self, v: u32) -> u64 {
        let vi = v as usize;
        let w = self.bitset_words;
        let vb = vi * w;
        let vbs = &self.bitset[vb..vb + w];
        let k: u64 = vbs.iter().map(|x| x.count_ones() as u64).sum();
        if k < 2 {
            return 0;
        }
        let total_pairs = k * (k - 1) / 2;

        // Sparse path: k(k-1)/2 < k·w (i.e. k ≤ 2w) means iterating pairs
        // directly beats a dense row scan per neighbour.
        if k < (2 * w) as u64 {
            let mut nbrs: [u32; 256] = [0; 256];
            let klen = k as usize;
            if klen <= nbrs.len() {
                let mut idx = 0;
                for (j, &v_word) in vbs.iter().enumerate() {
                    let mut word = v_word;
                    while word != 0 {
                        let lsb = word.trailing_zeros() as usize;
                        nbrs[idx] = (j * 64 + lsb) as u32;
                        idx += 1;
                        word &= word - 1;
                    }
                }
                let mut edges = 0u64;
                for i in 0..klen {
                    let u = nbrs[i] as usize;
                    let ub = u * w;
                    for &other in &nbrs[i + 1..klen] {
                        let x = other as usize;
                        let bit = (self.bitset[ub + (x >> 6)] >> (x & 63)) & 1;
                        edges += bit;
                    }
                }
                return total_pairs - edges;
            }
        }

        // Dense fallback: O(k · w).
        let mut doubled = 0u64;
        for j in 0..w {
            let mut word = vbs[j];
            while word != 0 {
                let lsb = word.trailing_zeros() as usize;
                let u = j * 64 + lsb;
                let ub = u * w;
                let ubs = &self.bitset[ub..ub + w];
                for l in 0..w {
                    doubled += (ubs[l] & vbs[l]).count_ones() as u64;
                }
                word &= word - 1;
            }
        }
        total_pairs - doubled / 2
    }
}
