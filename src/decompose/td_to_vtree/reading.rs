//! A **reading** of a tree decomposition: the triple that fixes one way of
//! turning it into a vtree, and the vocabulary each of its three dimensions
//! takes.
//!
//! A decomposition does not name a vtree by itself — it has to be rooted, every
//! variable has to be given one bag, and each bag's children and leaves have to
//! be folded into a binary subtree. Those three choices are the reading, and
//! [`Reading`] is a reading with any of them left open: a dimension a caller
//! names is fixed, and a dimension left `None` is one the conversion searches.
//!
//! The order of [`PLACES`] and [`FOLDS`] is the order the search walks them in,
//! so it is also what a truncated search gets through first.

/// Which bag of the decomposition becomes its root.
///
/// [`Root::First`] and [`Root::Centroid`] are strategies, applied per connected
/// component — a decomposition that is a forest gets one root per component.
/// [`Root::Leaf`] names a single bag, which is what the search enumerates over
/// and what a caller holding its own decomposition can pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Root {
    /// Each component's lowest-index bag — whatever order the decomposition was
    /// written in.
    First,
    /// Each component's centroid: the bag minimising the largest part left when
    /// it is removed.
    Centroid,
    /// This bag, by index. The component it does not reach keeps its
    /// lowest-index bag.
    Leaf(usize),
}

/// Which of the bags holding a variable gets its leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// The bag furthest from the root. Deep placement lets each clause's
    /// variables meet as far from the root as the decomposition allows, which
    /// is what carries the decomposition's width over to the tree. Given the
    /// CNF, a tie between equally deep bags goes to the one holding more of the
    /// variable's clause partners.
    Deep,
    /// The bag closest to the root, so a variable shared by several branches
    /// sits above all of them.
    Shallow,
}

/// How one bag's already-built child subtrees and its own variable leaves are
/// folded into a single binary subtree.
///
/// The clause-aware folds — everything from [`Fold::ClauseSplit`] on — read the
/// CNF; handed none, each falls back to [`Fold::Balanced`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    /// Children then leaves, the list halved recursively into a balanced
    /// subtree.
    Balanced,
    /// The same, children sorted by subtree leaf count ascending first.
    BySize,
    /// The same, leaves before children.
    VarsFirst,
    /// A chain instead of a balanced split: one item per level, the first
    /// nearest the root.
    LeftDeep,
    /// The items bisected greedily so that as few clauses as possible span both
    /// halves, recursively.
    ClauseSplit,
    /// The same objective under the multilevel partitioner, clauses as
    /// hyperedges.
    Hypergraph,
    /// The leaves that also appear in the parent bag split off as one subtree,
    /// beside everything else — the cut variables kept contiguous.
    Boundary,
    /// Children bisected to share as few of this bag's variables as possible; a
    /// leaf goes to the side that uses it, rises above the cut when both sides
    /// do, and follows its clause partners when neither does. Written for
    /// [`Place::Shallow`]: under [`Place::Deep`] a shared variable already sits
    /// inside one branch, so nothing rises above a cut.
    TdEdge,
    /// [`Fold::Balanced`] over leaves chained by clause co-occurrence rather
    /// than by variable number, so variables sharing clauses sit adjacent.
    Affinity,
}

/// How a [`Root`] strategy is written in a `--vtree` spec. [`Root::Leaf`] is
/// absent: the search reaches every leaf bag, and no spec can name one of them
/// apart.
pub(crate) const ROOTS: &[(&str, Root)] = &[("first", Root::First), ("centroid", Root::Centroid)];

/// How a [`Place`] is written, in the order the search tries them.
pub(crate) const PLACES: &[(&str, Place)] = &[("deep", Place::Deep), ("shallow", Place::Shallow)];

/// How a [`Fold`] is written, in the order the search tries them.
pub(crate) const FOLDS: &[(&str, Fold)] = &[
    ("balanced", Fold::Balanced),
    ("by-size", Fold::BySize),
    ("vars-first", Fold::VarsFirst),
    ("left-deep", Fold::LeftDeep),
    ("clause-split", Fold::ClauseSplit),
    ("hypergraph", Fold::Hypergraph),
    ("boundary", Fold::Boundary),
    ("td-edge", Fold::TdEdge),
    ("affinity", Fold::Affinity),
];

/// The word `table` spells `value` with.
fn token<T: PartialEq>(table: &[(&'static str, T)], value: &T) -> &'static str {
    table
        .iter()
        .find(|(_, v)| v == value)
        .map(|(n, _)| *n)
        .expect("every value has a row in its own table")
}

impl std::fmt::Display for Root {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Root::Leaf(bag) => write!(f, "leaf#{bag}"),
            named => f.write_str(token(ROOTS, named)),
        }
    }
}

impl std::fmt::Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(token(PLACES, self))
    }
}

impl std::fmt::Display for Fold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(token(FOLDS, self))
    }
}

/// One way of reading a vtree off a tree decomposition, with any of its three
/// dimensions left open.
///
/// A dimension set to `Some` is fixed; a dimension left `None` is one the
/// conversion searches, scoring every reading it reaches and keeping the
/// cheapest. All three named is therefore exactly one reading, and the default
/// — all three `None` — is the whole search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    /// Which bag the decomposition is rooted at.
    pub root: Option<Root>,
    /// Which bag holding a variable gets its leaf.
    pub place: Option<Place>,
    /// How each bag is folded into a binary subtree.
    pub fold: Option<Fold>,
}

impl Reading {
    /// This reading with every dimension it leaves open taken from `base` —
    /// what makes a run-wide reading a default the specs written under it
    /// refine rather than a second setting competing with them.
    #[must_use]
    pub fn inherit(self, base: Reading) -> Reading {
        Reading {
            root: self.root.or(base.root),
            place: self.place.or(base.place),
            fold: self.fold.or(base.fold),
        }
    }
}

/// One reading with nothing left open — what a single conversion is run under.
///
/// The search resolves a [`Reading`] into these; [`super::algo::convert_one`]
/// takes one and builds exactly one tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FixedReading {
    pub(crate) root: Root,
    pub(crate) place: Place,
    pub(crate) fold: Fold,
}

impl std::fmt::Display for FixedReading {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "root={} place={} fold={}",
            self.root, self.place, self.fold
        )
    }
}
