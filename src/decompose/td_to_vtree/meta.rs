//! Tree-decomposition metadata that survives the TD → vtree conversion.
//!
//! A vtree records WHERE each variable ended up, but not WHY. The conversion
//! computes, for every variable, the ONE tree-decomposition bag it is assigned
//! to, plus a BFS order over the rooted TD's bags — then drops both the moment
//! the vtree is built. [`BagMetadata`] keeps them instead, so a consumer can
//! schedule clause work along the decomposition rather than only the vtree.
//! [`BagMetadata::from_assignment`] is the one place bag ranks are defined.
//!
//! **Rank invariant.** `bag_rank` is a bottom-up topological index over the
//! ROOTED TD: a bijection onto `0..num_bags` in which every child bag ranks
//! strictly below its parent. Because it is a bijection, grouping by rank IS
//! grouping by bag. A clause's rank is the MAXIMUM rank over its variables — the
//! first point in the bottom-up sweep at which all of them are available.
//!
//! **Availability is best-effort, never required.** Metadata exists only for a
//! vtree that a TD conversion produced and that won its own candidate selection
//! ([`super::TdConversionMeta`] pairs metadata with its vtree so the two can't be
//! mismatched); any other construction path returns `None`. Variable ids are
//! only valid within the formula the conversion ran on — see
//! [`BagMetadata::num_vars`].
//!
//! A missing metadata value degrades scheduling to the ordinary ordering; it
//! never changes the compiled function.

/// Sentinel `var_bag` entry for a variable that appears in no bag (an isolated
/// variable — the conversion appends a bare leaf for it at the top level).
pub(super) const NO_BAG: u32 = u32::MAX;

/// Per-variable tree-decomposition placement, carried out of the TD → vtree
/// conversion so a consumer can consult the decomposition, not just the vtree.
#[derive(Clone, Debug)]
pub struct BagMetadata {
    num_vars: u32,
    /// `var_bag[v]` = assigned bag id, or [`NO_BAG`].
    var_bag: Vec<u32>,
    /// `bag_rank[b]` = bottom-up topological rank of bag `b`.
    bag_rank: Vec<u32>,
    /// Width of the source decomposition.
    treewidth: u32,
}

impl BagMetadata {
    /// Build the metadata from the arrays the TD → vtree conversion already has
    /// in hand at the end of its bag-assignment step. `bfs_order` is the BFS
    /// visit order over bags (shallowest first); reversing it gives the
    /// bottom-up build order, which IS the rank order, since children are
    /// visited after their parent in BFS and hence before it in reverse.
    ///
    /// `treewidth` is the source decomposition's own
    /// [`treewidth`](crate::decompose::TreeDecomposition::treewidth) — taken
    /// from the bags rather than counted off `var_bag_usize`, which holds ONE
    /// bag per variable and so would undercount every bag a variable is in but
    /// not assigned to.
    pub(super) fn from_assignment(
        num_vars: u32,
        var_bag_usize: &[usize],
        bfs_order: &[usize],
        num_bags: usize,
        treewidth: u32,
    ) -> Self {
        debug_assert_eq!(
            bfs_order.len(),
            num_bags,
            "BagMetadata::from_assignment: BFS order must cover every bag exactly once",
        );
        let mut bag_rank = vec![0u32; num_bags];
        let n = bfs_order.len();
        for (i, &bag) in bfs_order.iter().enumerate() {
            bag_rank[bag] = (n - 1 - i) as u32;
        }
        let var_bag: Vec<u32> = var_bag_usize
            .iter()
            .map(|&b| if b == usize::MAX { NO_BAG } else { b as u32 })
            .collect();
        debug_assert_eq!(var_bag.len(), num_vars as usize);
        Self {
            num_vars,
            var_bag,
            bag_rank,
            treewidth,
        }
    }

    /// Number of variables this metadata describes. Consumers MUST check it
    /// against the formula they are about to compile: metadata built for a
    /// component-local variable space is meaningless in the global one.
    pub fn num_vars(&self) -> u32 {
        self.num_vars
    }

    /// Number of bags in the source tree decomposition.
    pub fn num_bags(&self) -> u32 {
        self.bag_rank.len() as u32
    }

    /// Width of the source decomposition — its largest bag less one, on the
    /// graph projection the conversion ran on. Not measured over
    /// [`var_bag`](Self::var_bag): a variable sits in every bag of a connected
    /// subtree and is assigned to exactly one of them.
    pub fn treewidth(&self) -> u32 {
        self.treewidth
    }

    /// Bag id assigned to variable `v`, or `None` when `v` is in no bag (or out
    /// of range).
    pub fn var_bag(&self, v: u32) -> Option<u32> {
        match self.var_bag.get(v as usize) {
            Some(&b) if b != NO_BAG => Some(b),
            _ => None,
        }
    }

    /// Bottom-up topological rank of variable `v`'s bag, or `None` when `v` is
    /// in no bag.
    pub fn var_bag_rank(&self, v: u32) -> Option<u32> {
        self.var_bag(v).map(|b| self.bag_rank[b as usize])
    }

    /// Rank of a clause = the MAXIMUM bag rank over its variables — the earliest
    /// point in the bottom-up sweep at which every one of its variables is
    /// available. Returns `None` when no variable of the clause is assigned to
    /// a bag (isolated variables only).
    pub fn clause_bag_rank(&self, vars: impl IntoIterator<Item = u32>) -> Option<u32> {
        vars.into_iter().filter_map(|v| self.var_bag_rank(v)).max()
    }
}
