//! The hypergraph every level of the hierarchy is made of, stored in both
//! directions at once.
//!
//! Coarsening walks vertex -> hyperedges -> pins to find match candidates and
//! refinement walks hyperedge -> pins to update gains, so both incidence
//! directions are materialized rather than derived on demand. Only the finest
//! level has all-ones weights: coarsening sums what it merges.

pub(super) struct Hypergraph {
    pub(super) num_vertices: usize,
    /// Fine vertices collapsed into this one. Balance is measured in these
    /// units and cut cost in `hewgt` units; the two are not commensurate, which
    /// matters wherever a single quantity has to price both (see
    /// `refine_flow`).
    pub(super) vwgt: Vec<u32>,
    /// Default: all 1s when no weights given.
    pub(super) hewgt: Vec<u32>,
    pub(super) he_start: Vec<u32>,
    pins: Vec<u32>,
    pub(super) v_he_start: Vec<u32>,
    v_he: Vec<u32>,
}

impl Hypergraph {
    /// Pins are stored exactly as given: each hyperedge must already be
    /// deduplicated, since `hg_greedy_growing` and the FM passes compare a
    /// running count of pins on one side against `he_start[i+1] - he_start[i]`,
    /// and a repeated pin makes a fully-contained hyperedge never reach its own
    /// pin count. `weights` is parallel to `hyperedges`.
    pub(super) fn from_hyperedges(
        num_vertices: usize,
        hyperedges: &[Vec<u32>],
        weights: Option<&[u32]>,
    ) -> Self {
        let mut he_start = Vec::with_capacity(hyperedges.len() + 1);
        let mut pins = Vec::new();
        he_start.push(0);
        for he in hyperedges {
            pins.extend_from_slice(he);
            he_start.push(pins.len() as u32);
        }

        let hewgt = match weights {
            Some(w) => w.to_vec(),
            None => vec![1; hyperedges.len()],
        };

        let mut v_to_he: Vec<Vec<u32>> = vec![Vec::new(); num_vertices];
        for (hei, he) in hyperedges.iter().enumerate() {
            for &v in he {
                v_to_he[v as usize].push(hei as u32);
            }
        }
        let mut v_he_start = Vec::with_capacity(num_vertices + 1);
        let mut v_he = Vec::new();
        v_he_start.push(0);
        for list in &v_to_he {
            v_he.extend_from_slice(list);
            v_he_start.push(v_he.len() as u32);
        }

        Hypergraph {
            num_vertices,
            vwgt: vec![1; num_vertices],
            hewgt,
            he_start,
            pins,
            v_he_start,
            v_he,
        }
    }

    pub(super) fn num_hyperedges(&self) -> usize {
        self.he_start.len() - 1
    }

    /// One of the two ways into the pin structure, and therefore one of the two
    /// places this family's work is charged.
    ///
    /// Every coarsening, partitioning and refinement loop here reaches its data
    /// through this accessor or through [`Hypergraph::vertex_hyperedges`], so
    /// charging the length of the slice each hands back prices all of them from
    /// one place rather than from a charge in every loop. A caller that takes a
    /// slice only to read its length is charged for pins it never visits, which
    /// is the safe direction for a clock whose job is to stop a build before a
    /// wall does.
    pub(super) fn hyperedge_pins(&self, hei: usize) -> &[u32] {
        let start = self.he_start[hei] as usize;
        let end = self.he_start[hei + 1] as usize;
        crate::decompose::meter::charge((end - start) as u64);
        &self.pins[start..end]
    }

    /// How many pins each hyperedge has on each side of `part`.
    ///
    /// The whole hypergraph gain model is a statement about these two numbers
    /// reaching 0, 1 or 2, so every refiner starts by building them and then
    /// maintains them across its own moves.
    pub(super) fn pin_counts(&self, part: &[u8]) -> Vec<[u32; 2]> {
        let mut counts = vec![[0u32; 2]; self.num_hyperedges()];
        for (hei, he_counts) in counts.iter_mut().enumerate() {
            for &v in self.hyperedge_pins(hei) {
                he_counts[part[v as usize] as usize] += 1;
            }
        }
        counts
    }

    /// The other way in; see [`Hypergraph::hyperedge_pins`] for what the charge
    /// prices.
    pub(super) fn vertex_hyperedges(&self, v: usize) -> &[u32] {
        let start = self.v_he_start[v] as usize;
        let end = self.v_he_start[v + 1] as usize;
        crate::decompose::meter::charge((end - start) as u64);
        &self.v_he[start..end]
    }
}
