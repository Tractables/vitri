//! A **reading** of a tree decomposition: the triple that fixes one way of
//! turning it into a vtree, and the vocabulary each of its three dimensions
//! takes.
//!
//! A decomposition does not name a vtree by itself — it has to be rooted, every
//! variable has to be given one bag, and each bag's children and leaves have to
//! be binarized into one subtree. Those three choices are the reading, and
//! [`Reading`] is a reading with any of them left open: a dimension a caller
//! names is fixed, and a dimension left `None` is one the conversion searches.
//!
//! The order of [`PLACES`] and [`BINARIZATIONS`] is the order the search walks them in,
//! so it is also what a truncated search gets through first.

/// How the decomposition is rooted.
///
/// Each of the three is a strategy rather than a bag, applied per connected
/// component — a decomposition that is a forest gets one root per component.
/// [`Root::Leaf`] names a set of them, so naming it still leaves the search a
/// choice; the reported reading says which bag it settled on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Root {
    /// Each component's lowest-index bag — whatever order the decomposition was
    /// written in.
    First,
    /// Each component's centroid: the bag minimising the largest part left when
    /// it is removed.
    Centroid,
    /// One of the decomposition's leaf bags — those with a single neighbour —
    /// searched over.
    Leaf,
}

/// Which of the bags holding a variable gets its leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// The bag closest to the root, so a variable shared by several branches
    /// sits above all of them.
    Shallow,
    /// The bag furthest from the root. Deep placement lets each clause's
    /// variables meet as far from the root as the decomposition allows, which
    /// is what carries the decomposition's width over to the tree. Given the
    /// CNF, a tie between equally deep bags goes to the one holding more of the
    /// variable's clause partners.
    Deep,
}

/// How one bag's already-built child subtrees and its own variable leaves are
/// binarized into a single subtree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binarization {
    /// Children bisected to share as few of this bag's variables as possible; a
    /// leaf goes to the side that uses it, rises above the cut when both sides
    /// do, and follows its clause partners when neither does. Written for
    /// [`Place::Shallow`]: under [`Place::Deep`] a shared variable already sits
    /// inside one branch, so nothing rises above a cut.
    ///
    /// Reads the CNF; handed none, it binarizes as [`Binarization::Balanced`] does.
    Edge,
    /// The items bisected under the multilevel partitioner so that as few
    /// clauses as possible span both halves, clauses as hyperedges.
    ///
    /// Reads the CNF; handed none, it binarizes as [`Binarization::Balanced`] does.
    Hypergraph,
    /// Children then leaves, the list halved recursively into a balanced
    /// subtree. Reads no clause, which is also what makes it the binarization a
    /// conversion handed no CNF runs.
    Balanced,
}

/// How a [`Root`] is written, in the order the search enumerates the
/// strategies in.
pub(crate) const ROOTS: &[(&str, Root)] = &[
    ("first", Root::First),
    ("centroid", Root::Centroid),
    ("leaf", Root::Leaf),
];

/// How a [`Place`] is written. The first row is the one the screen runs at.
pub(crate) const PLACES: &[(&str, Place)] = &[("shallow", Place::Shallow), ("deep", Place::Deep)];

/// How a [`Binarization`] is written. The first row is the one the screen runs at.
pub(crate) const BINARIZATIONS: &[(&str, Binarization)] = &[
    ("edge", Binarization::Edge),
    ("hypergraph", Binarization::Hypergraph),
    ("balanced", Binarization::Balanced),
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
        f.write_str(token(ROOTS, self))
    }
}

impl std::fmt::Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(token(PLACES, self))
    }
}

impl std::fmt::Display for Binarization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(token(BINARIZATIONS, self))
    }
}

/// One way of reading a vtree off a tree decomposition, with any of its three
/// dimensions left open.
///
/// A dimension set to `Some` is fixed; a dimension left `None` is one the
/// conversion searches, scoring every reading it reaches and keeping the
/// cheapest. The default — all three `None` — is the whole search. All three
/// named leaves only the choice [`Root::Leaf`] carries, which is a search over
/// the decomposition's leaf bags and settles the same way every time it is
/// given the same time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    /// How the decomposition is rooted.
    pub root: Option<Root>,
    /// Which bag holding a variable gets its leaf.
    pub place: Option<Place>,
    /// How each bag is binarized into one subtree.
    pub binarize: Option<Binarization>,
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
            binarize: self.binarize.or(base.binarize),
        }
    }
}

/// The bag a rooting settled on: a strategy applied to every component, or one
/// named bag with the rest of the components keeping their lowest-index one.
///
/// [`Root`] is what a spec writes and [`RootPick`] is what one conversion runs
/// at, which is why a search over [`Root::Leaf`] reports `leaf#<bag>`: the
/// caller asked for a leaf bag, and this is the one that won.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RootPick {
    /// [`Root::First`].
    First,
    /// [`Root::Centroid`].
    Centroid,
    /// This bag, by index.
    Leaf(usize),
}

impl std::fmt::Display for RootPick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RootPick::First => f.write_str("first"),
            RootPick::Centroid => f.write_str("centroid"),
            RootPick::Leaf(bag) => write!(f, "leaf#{bag}"),
        }
    }
}

/// One reading with nothing left open — what a single conversion is run under.
///
/// The search resolves a [`Reading`] into these; [`super::algo::convert_one`]
/// takes one and builds exactly one tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FixedReading {
    pub(crate) root: RootPick,
    pub(crate) place: Place,
    pub(crate) binarize: Binarization,
}

impl std::fmt::Display for FixedReading {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "root={} place={} binarize={}",
            self.root, self.place, self.binarize
        )
    }
}
