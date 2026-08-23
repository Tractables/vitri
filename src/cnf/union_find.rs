//! Disjoint-set (union-find) with path splitting.
//!
//! Path splitting (each node points to its grandparent during `find`) gives
//! amortized near-O(1) per operation without full path compression.
//!
//! [`UnionFind::union`] uses union-by-rank, so the tree stays balanced and no
//! caller controls which root wins.

pub(crate) struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl UnionFind {
    pub(crate) fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    pub(crate) fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            let p = self.parent[x];
            self.parent[x] = self.parent[p]; // path splitting
            x = p;
        }
        x
    }

    /// Union by rank. No caller control over which root wins.
    pub(crate) fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return;
        }
        if self.rank[ra] < self.rank[rb] {
            self.parent[ra] = rb;
        } else if self.rank[ra] > self.rank[rb] {
            self.parent[rb] = ra;
        } else {
            self.parent[rb] = ra;
            self.rank[ra] += 1;
        }
    }
}
