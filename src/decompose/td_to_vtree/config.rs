//! The option space of the TD → vtree conversion: how a variable occurring in
//! several bags picks one, where the decomposition is rooted, how variables are
//! ordered inside a bag, and how a bag's children and leaves are arranged
//! before binarization.
//!
//! Public, so a caller converting its own decomposition can set every knob; the
//! conversion in `algo` and the sweep in `portfolio` read the same struct. Each
//! variant documents the vtree shape it produces — [`ItemOrdering::TdEdgeAligned`]
//! and [`BagAssignment::Shallowest`] are the one pair meant to be set together.

/// How variables are assigned to TD bags in `td_to_vtree`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BagAssignment {
    /// Assign each variable to its deepest bag (furthest from root).
    /// Empirically correct: exploits the running intersection property to keep
    /// clause LCAs at leaf-direction levels, minimising context width.
    Deepest,
    /// Assign each variable to its shallowest bag (closest to root).
    /// Empirically worse: variables float to high-level bags, all clause LCAs
    /// pile up near the root — same behaviour as a balanced vtree.
    Shallowest,
}

/// How to choose the root of the tree decomposition before converting to a vtree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TdRootStrategy {
    /// Use the first bag (`bags[0]`) as root. This is the default — whatever the
    /// TD solver produced, we use its natural ordering.
    FirstBag,
    /// Root at the centroid of the TD tree (the node that minimizes the maximum
    /// depth of any other node). This produces a more balanced vtree.
    Centroid,
}

/// How to order variables within a TD bag before binarization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarOrderInBag {
    /// Natural variable ID order (0, 1, 2, ...). Default.
    Natural,
    /// Order by clause co-occurrence: variables that share clauses are adjacent.
    /// Uses a greedy nearest-neighbor heuristic on the primal subgraph within the bag.
    /// A hub clause — one over an internal length cap — is left out of that
    /// subgraph, here as in every other reader of it: the clique it contributes
    /// would outvote every short clause in each bag it touches.
    ClauseAffinity,
}

/// How to arrange child subtrees and local variable leaves at each TD node
/// before binarization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemOrdering {
    /// Children first (in TD adjacency order), then local variable leaves. Default.
    ChildrenFirst,
    /// Local variable leaves first, then children.
    VariablesFirst,
    /// Sort children by subtree size (ascending), then local variable leaves.
    ChildrenBySize,
    /// Reversed: children in reverse order, then variables in reverse order.
    Reversed,
    /// Clause-aware: use clause co-occurrence to guide the bisection of items.
    /// Items that share clauses are placed in the same subtree.
    ClauseSplit,
    /// Left-deep (right-linear) chain: each internal node has a single item on the
    /// left and the rest on the right. Items earlier in the list become leaves near
    /// the top of the vtree.
    LeftDeep,
    /// Sort children by descending subtree size (largest first), then variables.
    /// Balanced binarization. Puts the largest child subtrees on the left.
    LargestFirst,
    /// Multilevel hypergraph bisection: uses the multilevel partitioner to split
    /// items based on clause connectivity — high-quality cuts that minimize the
    /// number of clauses spanning both halves.
    HypergraphBisect,
    /// Boundary-aware: at each TD node `t`, partition local variables into
    /// **boundary** (those that also appear in the parent bag) and **interior**
    /// (the rest). Build a balanced subtree of the interior side (children +
    /// interior var leaves) and a balanced subtree of the boundary variables,
    /// then combine `(interior, boundary)`. Keeps cut variables contiguous as a
    /// sub-vtree adjacent to the rest of `t`'s subtree.
    BoundaryAdjacent,
    /// TD-edge-aligned faithful combiner. At each TD node the recursive vtree
    /// partition follows the tree-decomposition's own edge structure:
    ///
    /// - **Edge-aligned cuts** — children (each a distinct TD branch) are
    ///   bisected so the two halves share as few local separator variables as
    ///   possible; every cut's crossing set is therefore bounded by a
    ///   bag/separator, not by the accumulated pathwidth of a midpoint split.
    /// - **Bag-local leaf routing** — a variable assigned to this bag attaches
    ///   on the edge toward the child subtree holding most of its clause
    ///   partners (primal neighbours).
    /// - **Separator lifting** — a variable shared by k>1 children is placed at
    ///   the LOWEST shared ancestor of exactly the children that use it, NOT
    ///   chained above everything, so it crosses only the cuts between the
    ///   branches that actually reference it.
    ///
    /// Intended to pair with [`BagAssignment::Shallowest`]: separator lifting
    /// only fires when a shared variable is assigned to the LCA node rather than
    /// pushed into one deepest branch. Under `Deepest`, separators are already
    /// pushed down and this combiner degenerates to edge-aligned children plus
    /// local-leaf routing.
    TdEdgeAligned,
}

/// Full configuration for the TD → vtree conversion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TdToVtreeConfig {
    /// How to assign a variable shared by several TD bags to exactly one bag.
    pub bag_assignment: BagAssignment,
    /// How to choose the tree decomposition's root before conversion.
    pub root_strategy: TdRootStrategy,
    /// How to order variables within a single TD bag before binarization.
    pub var_order: VarOrderInBag,
    /// How to arrange child subtrees and local variable leaves at each TD node
    /// before binarization.
    pub item_ordering: ItemOrdering,
}

impl Default for TdToVtreeConfig {
    fn default() -> Self {
        Self {
            bag_assignment: BagAssignment::Deepest,
            root_strategy: TdRootStrategy::FirstBag,
            var_order: VarOrderInBag::Natural,
            item_ordering: ItemOrdering::ChildrenFirst,
        }
    }
}

impl TdToVtreeConfig {
    /// Build a config with the given [`BagAssignment`] and every other field at
    /// its [`Default`] value.
    pub fn from_bag_assignment(ba: BagAssignment) -> Self {
        Self {
            bag_assignment: ba,
            ..Default::default()
        }
    }
}
