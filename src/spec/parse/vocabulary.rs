//! What a `--vtree` spec may say: the value tables each parameter draws from,
//! the base names, and the parameter table that says which family takes which
//! key.
//!
//! One owner per vocabulary. [`super::parse_vtree_spec`] matches against these
//! tables, `--help` prints them, and a rejection quotes them, so a name added
//! here is accepted and advertised at once.

use crate::decompose::{
    BINARIZATIONS, ClauseWeight, ForceMode, InitMode, OrientRule, PLACES, ROOTS, RootRule,
    WeightRule,
};

use super::{force_dim_range, force_feedback_range, force_restarts_range, one_of};

// ---------------------------------------------------------------------------
// Value vocabularies
//
// One table per axis a parameter can set: the word a spec writes, and the value
// it selects. Each table is the single owner of that axis's spelling — the
// parser reads a value through it, `--help` lists it, and a rejection quotes
// it, so a value added to one of these is accepted and advertised at once.
//
// The FIRST row of each table is that axis's default, which is what
// `TdToVtreeConfig::default()` and `ForceConfig::new` already produce; the
// `defaults_are_the_first_row_of_every_value_table` test holds the two together.
// ---------------------------------------------------------------------------

// The three conversion axes are spelled in `decompose` beside the values they
// name ([`ROOTS`], [`PLACES`], [`BINARIZATIONS`]): the order of the last two is also the
// order the conversion searches them in, and one owner keeps the two from
// drifting.

/// The value `name` selects in `table`, or `None` when the table has no such
/// word.
pub(super) fn lookup<T: Copy>(table: &[(&'static str, T)], name: &str) -> Option<T> {
    table.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
}

/// Every word `table` accepts, in table order.
pub(super) fn value_names<T>(table: &[(&'static str, T)]) -> impl Iterator<Item = &'static str> {
    table.iter().map(|(n, _)| *n)
}

/// Which tree-ifier turns the `force` embedding into a vtree.
pub(super) const FORCE_TREEIFIERS: &[(&str, ForceMode)] =
    &[("mst", ForceMode::Mst), ("cut", ForceMode::Cut)];

/// How the `force` MST is rooted.
pub(super) const FORCE_ROOTS: &[(&str, RootRule)] = &[
    ("merge", RootRule::Merge),
    ("balance", RootRule::Balance),
    ("hybrid", RootRule::Hybrid),
];

/// How a `force` MST edge is oriented into a left/right child pair.
pub(super) const FORCE_ORIENTS: &[(&str, OrientRule)] = &[
    ("x", OrientRule::X),
    ("small", OrientRule::Small),
    ("big", OrientRule::Big),
];

/// What a `force` MST edge weighs.
pub(super) const FORCE_WEIGHTS: &[(&str, WeightRule)] =
    &[("euclid", WeightRule::Euclid), ("co", WeightRule::Co)];

/// How a clause pulls the variables it holds together in the `force` embedding.
pub(super) const FORCE_CLAUSE_WEIGHTS: &[(&str, ClauseWeight)] = &[
    ("uniform", ClauseWeight::Uniform),
    ("short", ClauseWeight::Short),
];

/// How the `force` layout starts.
pub(super) const FORCE_INITS: &[(&str, InitMode)] =
    &[("rand", InitMode::Rand), ("force1d", InitMode::Force1d)];

/// How an elimination order breaks ties: deterministically, or by sampling
/// weighted by the SAT-aware Jeroslow-Wang score. Only some orders have the
/// second core, which is why writing it can be refused.
pub(super) const TIE_BREAKS: &[(&str, bool)] = &[("fixed", false), ("jw-sample", true)];

/// Whether the goatd schedule ends in its refinement pass.
pub(super) const REFINEMENTS: &[(&str, bool)] = &[("on", true), ("off", false)];

/// What `candidate=` accepts, spelt from the schedule's own cap.
pub(super) fn candidate_range() -> String {
    format!(
        "an integer from 0 to {}",
        crate::decompose::MAX_GOATD_CANDIDATES - 1
    )
}

// ---------------------------------------------------------------------------
// The base-name vocabulary
// ---------------------------------------------------------------------------

/// How a base name is offered to a reader. The parser treats every name the
/// same; this is what `--help` groups them by.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BaseGroup {
    /// A construction that builds the tree from a decomposition — or a
    /// partition — of a graph view of the CNF.
    Decomposition,
    /// A tree built from the variable numbering alone, consulting no clause.
    Baseline,
    /// Named on its own rather than in a list: the portfolio, which is the
    /// default and the one spec with a candidate set, and the force-directed
    /// embedding, which carries an axis grammar of its own.
    Standalone,
}

/// One `--vtree` base name: the name a spec writes, and the family writing it
/// selects.
pub(super) struct VtreeBaseName {
    /// The base name, as written in a spec.
    pub(super) name: &'static str,
    /// The family it selects.
    pub(super) family: VtreeBase,
}

impl VtreeBaseName {
    /// One row of [`VTREE_BASE_NAMES`].
    const fn new(name: &'static str, family: VtreeBase) -> Self {
        Self { name, family }
    }
}

/// The base name of the balanced baseline — the one construction this crate
/// falls back on unasked, so its name is reachable as a constant as well as a
/// table row.
pub(crate) const BALANCED_SPEC: &str = "balanced";

/// Every base name outside the elimination table, in the order the messages
/// offer them.
///
/// The single source for this vocabulary: [`classify_base`] matches against it,
/// [`super::unknown_vtree_type`] offers it, and `--help` groups it, so a name added
/// here is recognized and advertised at once. The single elimination orders are
/// deliberately absent — the construction table already holds those names
/// ([`crate::decompose::elimination_spec_names`]), and classification ends with
/// a lookup into it.
pub(super) const VTREE_BASE_NAMES: &[VtreeBaseName] = &[
    VtreeBaseName::new(BALANCED_SPEC, VtreeBase::Balanced),
    VtreeBaseName::new("linear", VtreeBase::Linear),
    VtreeBaseName::new("reverse-linear", VtreeBase::ReverseLinear),
    VtreeBaseName::new("random", VtreeBase::Random),
    VtreeBaseName::new("portfolio", VtreeBase::Portfolio),
    VtreeBaseName::new(
        "flowcutter-primal",
        VtreeBase::Flowcutter { incidence: false },
    ),
    VtreeBaseName::new(
        "flowcutter-incidence",
        VtreeBase::Flowcutter { incidence: true },
    ),
    VtreeBaseName::new("goatd-primal", VtreeBase::Goatd { incidence: false }),
    VtreeBaseName::new("goatd-incidence", VtreeBase::Goatd { incidence: true }),
    VtreeBaseName::new("guided-bisect", VtreeBase::GuidedBisect),
    VtreeBaseName::new("hypergraph-bisect", VtreeBase::HypergraphBisect),
    VtreeBaseName::new("primal-bisect", VtreeBase::PrimalBisect),
    VtreeBaseName::new("force", VtreeBase::Force),
];

/// Every base name a `--vtree` spec may write, in grammar order: the
/// numbering-only baselines, the portfolio, the decomposition families, the
/// force-directed embedding, and every single elimination order in both of its
/// graph views.
///
/// The COMPLETE list — with [`super::spec_param_docs`] it is everything a
/// reader needs to write any spec the parser accepts, which is what `--help`
/// is rendered from and what `docs/vtrees.md` is held to naming.
pub fn vtree_spec_bases() -> Vec<String> {
    let mut names: Vec<String> = VTREE_BASE_NAMES
        .iter()
        .map(|b| b.name.to_string())
        .collect();
    for name in crate::decompose::elimination_spec_names() {
        for (suffix, _) in crate::decompose::VIEW_SUFFIXES {
            names.push(format!("{name}{suffix}"));
        }
    }
    names
}

/// The base names that build from a decomposition or partition, in table order.
pub(crate) fn decomposition_spec_names() -> impl Iterator<Item = &'static str> {
    base_names(BaseGroup::Decomposition)
}

/// The base names of the numbering-only baselines, in table order.
pub(crate) fn baseline_spec_names() -> impl Iterator<Item = &'static str> {
    base_names(BaseGroup::Baseline)
}

/// The base names offered on their own rather than inside a list, in table
/// order.
pub(crate) fn standalone_spec_names() -> impl Iterator<Item = &'static str> {
    base_names(BaseGroup::Standalone)
}

/// The base names in `group`, in table order.
fn base_names(group: BaseGroup) -> impl Iterator<Item = &'static str> {
    VTREE_BASE_NAMES
        .iter()
        .filter(move |b| help_group(b.family) == Some(group))
        .map(|b| b.name)
}

/// Which of `--help`'s lists a base in `family` is offered in — `None` for the
/// two families it offers elsewhere: the single elimination orders, which have
/// a paragraph of their own built from the construction table, and the
/// unrecognized base, which is no offer at all.
///
/// Read off the family rather than carried per name, so a name cannot be filed
/// under a list its construction does not belong to.
fn help_group(family: VtreeBase) -> Option<BaseGroup> {
    Some(match family {
        VtreeBase::Balanced | VtreeBase::Linear | VtreeBase::ReverseLinear | VtreeBase::Random => {
            BaseGroup::Baseline
        }
        VtreeBase::Portfolio | VtreeBase::Force => BaseGroup::Standalone,
        VtreeBase::Goatd { .. }
        | VtreeBase::Flowcutter { .. }
        | VtreeBase::GuidedBisect
        | VtreeBase::HypergraphBisect
        | VtreeBase::PrimalBisect => BaseGroup::Decomposition,
        VtreeBase::Elimination { .. } | VtreeBase::Unknown => return None,
    })
}

/// Base-name family of a `--vtree` spec, after its `:key=value` parameters have
/// been stripped. One variant per base family that the CLI knows how to
/// validate and/or build. [`classify_base`] is the single place base strings
/// are matched, so every consumer agrees on which family a base belongs to; the
/// match on this enum is exhaustive with no wildcard arm, so adding a variant
/// here is a compile error until every consumer handles it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum VtreeBase {
    /// Matches `balanced`: a fixed balanced binary split over the declared
    /// (unpermuted) variable order, with no CNF-structure awareness — a
    /// comparison baseline, not a strategy for real workloads.
    Balanced,
    /// Matches `linear`: a chain vtree. Leaves are laid out in forward
    /// declared-variable order (`0..n`), variable 1 at the leftmost leaf —
    /// matching the OBDD order 1..n.
    Linear,
    /// Matches `reverse-linear`: `linear`'s mirror — the same chain shape but
    /// leaves in reversed declared-variable order (`n-1..0`), variable n at
    /// the leftmost leaf — see [`crate::vtree::Vtree::reverse_linear`].
    ReverseLinear,
    /// Matches any base starting with `random` (e.g. `random`,
    /// `random-anything`): a randomly shaped vtree over a randomly permuted
    /// variable order, both drawn from a fixed seed, so it is reproducible.
    /// Takes no parameter.
    Random,
    /// Matches `portfolio`: several FlowCutter portfolio candidates plus
    /// goatd, keeping the best-scoring candidate.
    Portfolio,
    /// Matches `goatd-primal` and `goatd-incidence`: a scheduled, selected
    /// tree-decomposition-to-vtree construction, run on the graph view the base
    /// names.
    Goatd {
        /// Which graph view the base named, resolved here so no backend
        /// re-reads it.
        incidence: bool,
    },
    /// Matches the single-order elimination family — `minfill`, `mindegree`,
    /// `nested-dissection` ([`crate::decompose::elimination_spec_names`] is the
    /// name list), each in both graph views. One fixed elimination order, no
    /// schedule and no refinement.
    Elimination {
        /// The order this base names, as
        /// [`crate::decompose::elimination_spec`] resolved it — a `'static`
        /// name from the construction table, so an error can quote it.
        name: &'static str,
        /// Which graph view the base named.
        incidence: bool,
    },
    /// Matches `flowcutter-primal` and `flowcutter-incidence`: in-process
    /// FlowCutter run on the CNF's primal graph (variables only) or on its
    /// incidence graph (variables and clauses both as vertices).
    Flowcutter {
        /// Which graph view the base named.
        incidence: bool,
    },
    /// Matches `guided-bisect`: recursive multilevel bisection of the primal
    /// graph, with a FlowCutter incidence decomposition offered at every level
    /// and kept where it scores better than the partition. Takes the FlowCutter
    /// search budget and nothing else — it binarizes no bag, so it names no
    /// reading.
    GuidedBisect,
    /// Matches `hypergraph-bisect`: multilevel hypergraph bisection, taking an
    /// optional `imbalance`.
    HypergraphBisect,
    /// Matches `primal-bisect`: the same multilevel core cutting the primal
    /// graph instead of the clause hypergraph, taking the same optional
    /// `imbalance`.
    PrimalBisect,
    /// Matches `force`: the force-directed embedding of the variables, tree-ified
    /// by MST or median cut. Carries its own axis parameters
    /// ([`super::parse_force_config`]).
    Force,
    /// Anything unrecognized. [`super::validate_vtree_spec`] refuses it by name, and
    /// so does the builder's own arm for a spec that reached construction
    /// without being validated.
    Unknown,
}

impl VtreeBase {
    /// Does this family read the CNF's primal/incidence graph, so that building
    /// each independent component on its own graph can help
    /// ([`crate::component::build_vtree_split`])?
    ///
    /// The numbering-only baselines are exactly `--help`'s baseline group, so
    /// the two answers come off the one table rather than from a second list
    /// that a name added to [`VTREE_BASE_NAMES`] could fall out of. An
    /// unrecognized base counts as structural, which costs nothing: the build
    /// it reaches reports the base rather than producing a tree.
    pub(crate) fn is_structural(self) -> bool {
        help_group(self) != Some(BaseGroup::Baseline)
    }
}

/// Classifies a `--vtree` base string into a [`VtreeBase`] family. Input is
/// the base already stripped of its `:key=value` parameters (callers compute
/// that split via [`vtree_spec_base`] / `split_vtree_spec`).
pub(crate) fn classify_base(base: &str) -> VtreeBase {
    let named = VTREE_BASE_NAMES.iter().find(|b| match b.family {
        // `random` is the one name matched as a prefix: `random-<anything>` is
        // the same fixed-seed baseline.
        VtreeBase::Random => base.starts_with(b.name),
        _ => base == b.name,
    });
    if let Some(b) = named {
        return b.family;
    }
    // The elimination table decides this family, and the lookup that decides it
    // is also the one that resolves the construction — so the name and the
    // graph view travel in the variant instead of being re-derived from the
    // string downstream.
    match crate::decompose::elimination_spec(base) {
        Some((name, incidence)) => VtreeBase::Elimination { name, incidence },
        None => VtreeBase::Unknown,
    }
}

/// Does `spec` name a construction that builds and scores several candidate
/// vtrees, and therefore has a candidate set to retain ([`crate::candidates`])?
///
/// True only for the portfolio — every other spec builds exactly one vtree, so
/// a retained candidate set could never hold more than that entry. The config
/// validator uses this to refuse an inert `candidates > 1`.
pub(crate) fn spec_has_candidates(spec: &str) -> bool {
    matches!(classify_base(vtree_spec_base(spec)), VtreeBase::Portfolio)
}

/// Tokenize a `--vtree` spec into `(base, params)` — `<base>[:params]`.
pub(super) fn split_vtree_spec(spec: &str) -> (&str, Option<&str>) {
    match spec.split_once(':') {
        Some((b, p)) => (b, Some(p)),
        None => (spec, None),
    }
}

/// The base-name head of a spec, with its `:key=value` parameters stripped.
pub(crate) fn vtree_spec_base(spec: &str) -> &str {
    split_vtree_spec(spec).0
}

// ---------------------------------------------------------------------------
// The parameter vocabulary
// ---------------------------------------------------------------------------

/// One `:key=value` parameter: the key a spec writes, which families accept it,
/// the values it takes, and what leaving it out means.
///
/// The single source for the parameter vocabulary: [`super::parse_vtree_spec`] refuses
/// a key whose row does not accept the spec's family, `--help` prints these
/// rows, and a rejection lists the keys the family does accept — all by reading
/// this table. The row does not carry the *parsing* of a value (a numeric range
/// and an enum table are not one shape); it carries what the value may be, as a
/// reader is told it.
pub(super) struct SpecParamKey {
    /// The key, without the `=`.
    pub(super) key: &'static str,
    /// Whether a spec whose base is in `family` may write this key.
    pub(super) accepts: fn(VtreeBase) -> bool,
    /// The values it takes, as `--help` and a rejection spell them out.
    pub(super) values: fn() -> String,
    /// What it means when the spec leaves it out.
    pub(super) default: &'static str,
    /// What writing it changes, in one phrase.
    pub(super) what: &'static str,
}

/// Every `:key=value` parameter, in the order `--help` and the messages offer
/// them.
pub(super) const SPEC_PARAM_KEYS: &[SpecParamKey] = &[
    SpecParamKey {
        key: "seed",
        accepts: |f| matches!(f, VtreeBase::Goatd { .. } | VtreeBase::Elimination { .. }),
        values: || "an integer".to_string(),
        default: "0",
        what: "which random tie-break the elimination takes",
    },
    SpecParamKey {
        key: "ties",
        // Only the orders that HAVE a sampling core, so an order without one
        // neither advertises the key nor accepts it.
        accepts: |f| {
            matches!(f, VtreeBase::Elimination { name, .. }
                if crate::decompose::elimination_order_samples(name))
        },
        values: || one_of(value_names(TIE_BREAKS)),
        default: "fixed",
        what: "how the elimination breaks a tie between two candidate variables",
    },
    SpecParamKey {
        key: "refine",
        accepts: |f| matches!(f, VtreeBase::Goatd { .. }),
        values: || one_of(value_names(REFINEMENTS)),
        default: "on",
        what: "whether the schedule ends in the refinement pass, or runs one \
               unrefined elimination slot",
    },
    SpecParamKey {
        key: "candidate",
        accepts: |f| matches!(f, VtreeBase::Goatd { .. }),
        values: candidate_range,
        default: "0",
        what: "which of the refined schedule's decompositions becomes the tree: 0 \
               the winner, refined; n above 0 its nth runner-up, unrefined",
    },
    SpecParamKey {
        key: "imbalance",
        accepts: |f| matches!(f, VtreeBase::HypergraphBisect | VtreeBase::PrimalBisect),
        values: || "a fraction in 0.0..=0.5".to_string(),
        default: "0.03",
        what: "how far either side may deviate from an even split",
    },
    SpecParamKey {
        key: "budget",
        accepts: fc_family,
        values: || "<N>ms (timed) or <N>steps (step-budgeted)".to_string(),
        default: "200ms",
        what: "how hard FlowCutter looks for a decomposition",
    },
    SpecParamKey {
        key: "iters",
        accepts: fc_family,
        values: || "an integer".to_string(),
        default: "100000 timed, 900 step-budgeted",
        what: "how many FlowCutter iterations the search runs",
    },
    SpecParamKey {
        key: "patience",
        accepts: fc_family,
        values: || "milliseconds without an improvement before the search stops".to_string(),
        default: "100 with no budget written, 150 with one",
        what: "how long the timed search waits for an improvement",
    },
    SpecParamKey {
        key: "root",
        accepts: conversion_family,
        values: || one_of(value_names(ROOTS)),
        default: "searched",
        what: "which bag the decomposition is rooted at",
    },
    SpecParamKey {
        key: "place",
        accepts: conversion_family,
        values: || one_of(value_names(PLACES)),
        default: "searched",
        what: "which bag of the decomposition each variable is placed in",
    },
    SpecParamKey {
        key: "binarize",
        accepts: conversion_family,
        values: || one_of(value_names(BINARIZATIONS)),
        default: "searched",
        what: "how each bag's children and variable leaves are binarized",
    },
    SpecParamKey {
        key: "treeify",
        accepts: is_force,
        values: || one_of(value_names(FORCE_TREEIFIERS)),
        default: "mst",
        what: "which tree-ifier turns the embedding into a vtree",
    },
    SpecParamKey {
        key: "root",
        accepts: is_force,
        values: || one_of(value_names(FORCE_ROOTS)),
        default: "merge",
        what: "where the MST is rooted",
    },
    SpecParamKey {
        key: "orient",
        accepts: is_force,
        values: || one_of(value_names(FORCE_ORIENTS)),
        default: "x",
        what: "how an MST edge becomes a left/right child pair",
    },
    SpecParamKey {
        key: "weights",
        accepts: is_force,
        values: || one_of(value_names(FORCE_WEIGHTS)),
        default: "euclid",
        what: "what an MST edge weighs",
    },
    SpecParamKey {
        key: "feedback",
        accepts: is_force,
        values: force_feedback_range,
        default: "0",
        what: "how many feedback rounds reshape the layout",
    },
    SpecParamKey {
        key: "clause-weight",
        accepts: is_force,
        values: || one_of(value_names(FORCE_CLAUSE_WEIGHTS)),
        default: "uniform",
        what: "how strongly a clause pulls its variables together",
    },
    SpecParamKey {
        key: "dim",
        accepts: is_force,
        values: force_dim_range,
        default: "2",
        what: "how many dimensions the variables are embedded in",
    },
    SpecParamKey {
        key: "restarts",
        accepts: is_force,
        values: force_restarts_range,
        default: "1",
        what: "how many layouts are tried, keeping the best",
    },
    SpecParamKey {
        key: "init",
        accepts: is_force,
        values: || one_of(value_names(FORCE_INITS)),
        default: "rand",
        what: "how the layout starts",
    },
];

/// The families that build a tree decomposition and then read it: both
/// FlowCutter views, both goatd views and every single elimination order. They
/// share the three conversion parameters because they share the conversion.
fn conversion_family(family: VtreeBase) -> bool {
    matches!(
        family,
        VtreeBase::Flowcutter { .. } | VtreeBase::Goatd { .. } | VtreeBase::Elimination { .. }
    )
}

/// The families that run FlowCutter and therefore own its search budget: both
/// views of the family itself, and `guided-bisect`, whose decomposition is one.
fn fc_family(family: VtreeBase) -> bool {
    matches!(
        family,
        VtreeBase::Flowcutter { .. } | VtreeBase::GuidedBisect
    )
}

/// The force-directed embedding, which owns the eight axis parameters and the
/// tree-ifier.
fn is_force(family: VtreeBase) -> bool {
    matches!(family, VtreeBase::Force)
}

/// The `force` axes that reshape the MST, and are therefore refused under
/// `treeify=cut`, which has no MST to reshape.
pub(super) const FORCE_MST_ONLY_KEYS: &[&str] = &["root", "orient", "weights", "feedback"];

pub(super) fn keys_for(family: VtreeBase) -> Vec<String> {
    SPEC_PARAM_KEYS
        .iter()
        .filter(|k| (k.accepts)(family))
        .map(|k| format!("{}=", k.key))
        .collect()
}
