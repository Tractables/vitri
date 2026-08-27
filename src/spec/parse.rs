//! The `--vtree` spec grammar: the vocabulary of construction names and the
//! `:key=value` parameters each family accepts, typed into the [`ParsedSpec`]
//! every backend is handed.
//!
//! One reader. [`parse_vtree_spec`] is the only thing that looks at a spec
//! string; [`validate_vtree_spec`] is that parse with the value dropped, and
//! [`super::build_one_vtree_artifacts`] dispatches on what came out. A spec
//! that validates is therefore exactly a spec some backend can build.
//!
//! # The grammar
//!
//! ```text
//! spec   := base [ ":" params ]
//! params := key "=" value { "," key "=" value }
//! ```
//!
//! There is exactly ONE parameter syntax. A parameter is always written with
//! its key, never positionally and never as a bare token appended to the base,
//! so what a spec varies is readable off the string without knowing which
//! family's grammar it belongs to. A key the base's family does not accept is
//! refused by name rather than ignored, a key may be written at most once, and
//! every key's values and default are declared once in [`SPEC_PARAM_KEYS`] —
//! which is also what `--help` prints and what a rejection lists.

use crate::decompose::{
    BINARIZATIONS, ClauseWeight, FC_BARE_TIMEOUT_MS, FC_DEFAULT_ITERS, FC_DEFAULT_STEPS_ITERS,
    FC_PATIENCE_MS_BARE, FC_PATIENCE_MS_PARAMETRIZED, ForceConfig, ForceMode, InitMode, OrientRule,
    PLACES, ROOTS, Reading, RootRule, WeightRule,
};
use crate::error::VitriError;

/// Reject one token of `spec`: `what` it was read as, the token itself, and the
/// form that would have been accepted.
///
/// The one place this file's rejections are worded, in the house style
/// [`crate::error`]'s module doc fixes — so a grammar rule added below is
/// reported the way every other one already is.
/// The `dim=` range, spelled once from the constant the layout enforces.
fn force_dim_range() -> String {
    format!("an integer 2..={}", crate::decompose::FORCE_MAX_DIM)
}

fn invalid_token(spec: &str, what: &str, got: &str, expected: &str) -> VitriError {
    VitriError::spec(spec, format!("invalid {what} {got:?}, expected {expected}"))
}

/// A closed vocabulary as a message offers it: every name in table order, comma
/// separated, with `or` before the last.
fn one_of<T: std::fmt::Display>(names: impl IntoIterator<Item = T>) -> String {
    let names: Vec<String> = names.into_iter().map(|n| n.to_string()).collect();
    match names.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
    }
}

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

/// The value `name` selects in `table`, or `None` when the table has no such
/// word.
fn lookup<T: Copy>(table: &[(&'static str, T)], name: &str) -> Option<T> {
    table.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
}

/// Every word `table` accepts, in table order.
fn value_names<T>(table: &[(&'static str, T)]) -> impl Iterator<Item = &'static str> {
    table.iter().map(|(n, _)| *n)
}

// The three conversion axes are spelled in `decompose` beside the values they
// name ([`ROOTS`], [`PLACES`], [`BINARIZATIONS`]): the order of the last two is also the
// order the conversion searches them in, and one owner keeps the two from
// drifting.

/// Which tree-ifier turns the `force` embedding into a vtree.
const FORCE_TREEIFIERS: &[(&str, ForceMode)] = &[("mst", ForceMode::Mst), ("cut", ForceMode::Cut)];

/// How the `force` MST is rooted.
const FORCE_ROOTS: &[(&str, RootRule)] = &[
    ("merge", RootRule::Merge),
    ("balance", RootRule::Balance),
    ("hybrid", RootRule::Hybrid),
];

/// How a `force` MST edge is oriented into a left/right child pair.
const FORCE_ORIENTS: &[(&str, OrientRule)] = &[
    ("x", OrientRule::X),
    ("small", OrientRule::Small),
    ("big", OrientRule::Big),
];

/// What a `force` MST edge weighs.
const FORCE_WEIGHTS: &[(&str, WeightRule)] =
    &[("euclid", WeightRule::Euclid), ("co", WeightRule::Co)];

/// How a clause pulls the variables it holds together in the `force` embedding.
const FORCE_CLAUSE_WEIGHTS: &[(&str, ClauseWeight)] = &[
    ("uniform", ClauseWeight::Uniform),
    ("short", ClauseWeight::Short),
];

/// How the `force` layout starts.
const FORCE_INITS: &[(&str, InitMode)] =
    &[("rand", InitMode::Rand), ("force1d", InitMode::Force1d)];

/// How an elimination order breaks ties: deterministically, or by sampling
/// weighted by the SAT-aware Jeroslow-Wang score. Only some orders have the
/// second core, which is why writing it can be refused.
const TIE_BREAKS: &[(&str, bool)] = &[("fixed", false), ("jw-sample", true)];

/// Whether the goatd schedule ends in its refinement pass.
const REFINEMENTS: &[(&str, bool)] = &[("on", true), ("off", false)];

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
struct VtreeBaseName {
    /// The base name, as written in a spec.
    name: &'static str,
    /// The family it selects.
    family: VtreeBase,
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
/// [`unknown_vtree_type`] offers it, and `--help` groups it, so a name added
/// here is recognized and advertised at once. The single elimination orders are
/// deliberately absent — the construction table already holds those names
/// ([`crate::decompose::elimination_spec_names`]), and classification ends with
/// a lookup into it.
const VTREE_BASE_NAMES: &[VtreeBaseName] = &[
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

/// Every base name a `--vtree` spec may write, in grammar order: the
/// numbering-only baselines, the portfolio, the decomposition families, the
/// force-directed embedding, and every single elimination order in both of its
/// graph views.
///
/// The COMPLETE list — with [`spec_param_docs`] it is everything a reader needs
/// to write any spec the parser accepts, which is what `--help` and
/// `docs/vtrees.md` are held to.
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
    /// ([`parse_force_config`]).
    Force,
    /// Anything unrecognized: the validator passes it (`Ok`) and the builder's
    /// "Unknown vtree type" handler reports it.
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
fn split_vtree_spec(spec: &str) -> (&str, Option<&str>) {
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
/// The single source for the parameter vocabulary: [`parse_vtree_spec`] refuses
/// a key whose row does not accept the spec's family, `--help` prints these
/// rows, and a rejection lists the keys the family does accept — all by reading
/// this table. The row does not carry the *parsing* of a value (a numeric range
/// and an enum table are not one shape); it carries what the value may be, as a
/// reader is told it.
struct SpecParamKey {
    /// The key, without the `=`.
    key: &'static str,
    /// Whether a spec whose base is in `family` may write this key.
    accepts: fn(VtreeBase) -> bool,
    /// The values it takes, as `--help` and a rejection spell them out.
    values: fn() -> String,
    /// What it means when the spec leaves it out.
    default: &'static str,
    /// What writing it changes, in one phrase.
    what: &'static str,
}

/// Every `:key=value` parameter, in the order `--help` and the messages offer
/// them.
const SPEC_PARAM_KEYS: &[SpecParamKey] = &[
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
        values: || "an integer 0..=8".to_string(),
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
        values: || force_dim_range(),
        default: "2",
        what: "how many dimensions the variables are embedded in",
    },
    SpecParamKey {
        key: "restarts",
        accepts: is_force,
        values: || "an integer 1..=16".to_string(),
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
const FORCE_MST_ONLY_KEYS: &[&str] = &["root", "orient", "weights", "feedback"];

/// Read the three conversion parameters out of a spec whose family builds one
/// tree decomposition and then reads it.
///
/// The one place those keys are consumed, so the families that take them cannot
/// come apart on what they mean. A key the spec left out stays `None`, which is
/// the dimension the conversion searches.
fn read_reading(params: &mut KeyedParams<'_>) -> Result<Reading, VitriError> {
    Ok(Reading {
        root: params.enum_value("root", ROOTS)?,
        place: params.enum_value("place", PLACES)?,
        binarize: params.enum_value("binarize", BINARIZATIONS)?,
    })
}

fn keys_for(family: VtreeBase) -> Vec<String> {
    SPEC_PARAM_KEYS
        .iter()
        .filter(|k| (k.accepts)(family))
        .map(|k| format!("{}=", k.key))
        .collect()
}

/// One `--vtree` parameter as `--help` prints it.
///
/// Rendered from the parameter table this module matches against, so the help
/// text cannot advertise a key the parser does not accept, or a default it does
/// not apply.
pub struct SpecParamDoc {
    /// The key, without the `=`.
    pub key: &'static str,
    /// The values it takes.
    pub values: String,
    /// What leaving it out means.
    pub default: &'static str,
    /// What writing it changes, in one phrase.
    pub what: &'static str,
}

/// Every `:key=value` parameter the base `spec_base` accepts, in grammar order.
///
/// `spec_base` is a base NAME (`flowcutter-primal`, `force`), not a whole spec.
/// A base no family claims accepts nothing, so the list comes back empty.
pub fn spec_param_docs(spec_base: &str) -> Vec<SpecParamDoc> {
    let family = classify_base(spec_base);
    SPEC_PARAM_KEYS
        .iter()
        .filter(|k| (k.accepts)(family))
        .map(|k| SpecParamDoc {
            key: k.key,
            values: (k.values)(),
            default: k.default,
            what: k.what,
        })
        .collect()
}

/// Assemble a spec string from a base and its already-joined parameter text.
///
/// THE one place the `base[:params]` shape is written out: [`ParsedSpec`]'s
/// `Display` renders through it, and so does the portfolio's candidate naming,
/// so one construction is spelled one way wherever it is reported.
pub(crate) fn spec_string(base: &str, params: Option<&str>) -> String {
    match params {
        Some(p) if !p.is_empty() => format!("{base}:{p}"),
        _ => base.to_string(),
    }
}

/// The `key=value` pairs of one spec, each remembering whether a family rule
/// read it.
///
/// Reading a key marks it used; [`KeyedParams::finish`] then refuses the first
/// key nothing read, which is what makes "a parameter the spec cannot honor is
/// refused rather than ignored" hold for every family without each family
/// restating it.
struct KeyedParams<'a> {
    /// The whole spec string, for the messages that name it.
    spec: &'a str,
    /// One entry per written `key=value`, in written order.
    entries: Vec<Entry<'a>>,
}

/// One written `key=value`.
struct Entry<'a> {
    key: &'a str,
    value: &'a str,
    used: bool,
}

impl<'a> KeyedParams<'a> {
    /// Split a spec's parameter text into its pairs, refusing a malformed pair,
    /// an empty key and a key written twice.
    ///
    /// A repeated key is refused rather than resolved: with one of the two
    /// values necessarily dropped, silently keeping either would make the spec
    /// mean something the reader did not write.
    fn new(spec: &'a str, raw: Option<&'a str>) -> Result<Self, VitriError> {
        let mut entries: Vec<Entry<'a>> = Vec::new();
        for part in raw.into_iter().flat_map(|p| p.split(',')) {
            let Some((key, value)) = part.split_once('=') else {
                return Err(VitriError::spec(
                    spec,
                    format!("parameter {part:?} must be written key=value"),
                ));
            };
            if key.is_empty() {
                return Err(VitriError::spec(
                    spec,
                    format!("parameter {part:?} has an empty key"),
                ));
            }
            if entries.iter().any(|e| e.key == key) {
                return Err(VitriError::spec(
                    spec,
                    format!(
                        "parameter {key:?} is written twice; one of the two values would be \
                         dropped. Write it once"
                    ),
                ));
            }
            entries.push(Entry {
                key,
                value,
                used: false,
            });
        }
        Ok(KeyedParams { spec, entries })
    }

    /// The pairs the spec wrote, ordered as [`SPEC_PARAM_KEYS`] declares them
    /// rather than as they were typed, so two spellings of one construction
    /// render alike.
    ///
    /// A key outside that table survives only under an unrecognized base, where
    /// `finish` never runs; it keeps its written position, which is all anything
    /// can say about it.
    fn written(&self) -> Vec<(&'a str, &'a str)> {
        let rank = |key: &str| {
            SPEC_PARAM_KEYS
                .iter()
                .position(|k| k.key == key)
                .unwrap_or(usize::MAX)
        };
        let mut pairs: Vec<(&'a str, &'a str)> =
            self.entries.iter().map(|e| (e.key, e.value)).collect();
        pairs.sort_by_key(|(key, _)| rank(key));
        pairs
    }

    /// The value written for `key`, marking it read. `None` when the spec did
    /// not write it.
    fn take(&mut self, key: &str) -> Option<&'a str> {
        self.entries.iter_mut().find(|e| e.key == key).map(|e| {
            e.used = true;
            e.value
        })
    }

    /// Was `key` written? Does NOT mark it read — for a rule that reacts to a
    /// key some other rule owns.
    fn wrote(&self, key: &str) -> bool {
        self.entries.iter().any(|e| e.key == key)
    }

    /// The value of `key` looked up in `table`, marking the key read.
    fn enum_value<T: Copy>(
        &mut self,
        key: &str,
        table: &[(&'static str, T)],
    ) -> Result<Option<T>, VitriError> {
        match self.take(key) {
            None => Ok(None),
            Some(v) => match lookup(table, v) {
                Some(found) => Ok(Some(found)),
                None => Err(invalid_token(
                    self.spec,
                    key,
                    v,
                    &one_of(value_names(table)),
                )),
            },
        }
    }

    /// The value of `key` parsed as a number, marking the key read. `expected`
    /// is how a rejection describes what would have been accepted.
    fn number<T: std::str::FromStr>(
        &mut self,
        key: &str,
        expected: &str,
    ) -> Result<Option<T>, VitriError> {
        match self.take(key) {
            None => Ok(None),
            Some(v) => match v.parse::<T>() {
                Ok(n) => Ok(Some(n)),
                Err(_) => Err(invalid_token(self.spec, key, v, expected)),
            },
        }
    }

    /// Refuse the first key no family rule read: either the family accepts no
    /// such key at all, or it accepts it only in a mode this spec is not in
    /// (the mode-specific rules report that themselves, before this runs).
    fn finish(&self, family: VtreeBase, base: &str) -> Result<(), VitriError> {
        let Some(unread) = self.entries.iter().find(|e| !e.used) else {
            return Ok(());
        };
        let accepted = keys_for(family);
        let offer = if accepted.is_empty() {
            format!("{base:?} takes no parameters")
        } else {
            format!("{base:?} takes {}", one_of(accepted))
        };
        Err(VitriError::spec(
            self.spec,
            format!("parameter \"{}=\" is not one {offer}", unread.key),
        ))
    }
}

// ---------------------------------------------------------------------------
// The parsed spec
// ---------------------------------------------------------------------------

/// A `--vtree` spec string after the one parse: its family, its typed
/// parameters, and the [`TdToVtreeConfig`] the conversion parameters set.
///
/// [`parse_vtree_spec`] is the only thing that reads the grammar.
/// [`validate_vtree_spec`] is that parse with the value dropped, and
/// [`build_one_vtree_artifacts`](super::build_one_vtree_artifacts) hands this
/// straight to the construction backends — so no backend re-reads the string,
/// and a spec that validates is exactly a spec some backend can build.
pub(crate) struct ParsedSpec<'a> {
    /// The spec exactly as it was written, for the one message that names the
    /// whole string rather than an offending token.
    pub raw: &'a str,
    /// The base-name head, parameters stripped.
    pub base: &'a str,
    /// Which family [`classify_base`] put `base` in.
    pub family: VtreeBase,
    /// The parameters, checked and typed for the family.
    pub param: SpecParam,
    /// Which dimensions of the conversion this spec named. A dimension left
    /// `None` is one the conversion searches; [`inherit`](Self::inherit) fills
    /// it from the run's own reading before any backend sees it.
    pub reading: Reading,
    /// The `key=value` pairs the spec wrote, in table order — what `Display`
    /// writes back out.
    written: Vec<(&'a str, &'a str)>,
}

/// A parsed spec spells itself the way it was accepted: the base, then every
/// parameter written on it.
///
/// A bundle publishes this and an error names it, so a construction can be read
/// back off either and handed to `--vtree` unchanged. A spec that wrote no
/// parameter is its bare base: the defaults it ran under are the defaults that
/// base still means.
impl std::fmt::Display for ParsedSpec<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let params: Vec<String> = self
            .written
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect();
        f.write_str(&spec_string(self.base, Some(&params.join(","))))
    }
}

impl ParsedSpec<'_> {
    /// Fill every dimension this spec left open from the run's own reading.
    ///
    /// Called once per build, before any component is built — so every
    /// component of one formula is read the same way, and a run-wide reading is
    /// a default the spec refines rather than a second setting competing with
    /// it.
    pub(crate) fn inherit(&mut self, base: Reading) {
        self.reading = self.reading.inherit(base);
    }
}

/// The typed parameters of a spec, one variant per shape a family's parameters
/// take. A family whose parameters need no carrier — and an unrecognized base,
/// whose parameters nothing will read — carries [`SpecParam::None`].
///
/// The parse types each family's parameters, so a build site is never handed a
/// variant its family does not carry: the accessors below answer such a variant
/// with that family's documented default, which is also what absent parameters
/// parse to, and the one accessor with no default to reach for
/// ([`SpecParam::fc_budget`]) reports against the base instead. Said once here,
/// so no accessor and no build site says it again.
pub(crate) enum SpecParam {
    /// No parameters, or a family that takes none.
    None,
    /// One elimination order: which of its two tie-breaking cores runs, and
    /// the RNG seed the tie-breaking draws on.
    Elimination {
        /// Break ties by JW-weighted sampling rather than deterministically.
        jw_sample: bool,
        /// The RNG seed. Absent means seed 0.
        seed: u64,
    },
    /// The goatd schedule: whether it ends in the refinement pass, and the RNG
    /// seed its tie-breaking and sampling draw on.
    Goatd {
        /// Run the refined schedule rather than one unrefined slot.
        refine: bool,
        /// The RNG seed. Absent means seed 0.
        seed: u64,
    },
    /// `imbalance=<f64>` — deviation from an even split, in `0.0..=0.5`.
    Imbalance(f64),
    /// FlowCutter timed mode: `budget=<N>ms`, with `iters=` and `patience=`.
    /// Also what a spec that named no budget resolves to — those defaults
    /// differ from the written form's, see
    /// [`FC_PATIENCE_MS_BARE`](crate::decompose::FC_PATIENCE_MS_BARE).
    FcTimed {
        /// Wall-clock budget for the timed search.
        timeout_ms: i64,
        /// FlowCutter iteration cap.
        iters: i32,
        /// Milliseconds without an improvement before the search gives up.
        patience_ms: i64,
    },
    /// FlowCutter step-budgeted mode: `budget=<N>steps`, with `iters=`.
    FcSteps {
        /// Computation-step budget handed to FlowCutter.
        steps: i64,
        /// FlowCutter iteration count.
        iters: i32,
    },
    /// `force` — the whole configuration, read from the tree-ifier and the
    /// eight axis parameters by [`parse_force_config`].
    Force(crate::decompose::ForceConfig),
}

impl SpecParam {
    /// The seed for a family whose parameters carry one; 0 otherwise.
    pub(crate) fn seed(&self) -> u64 {
        match *self {
            SpecParam::Elimination { seed, .. } | SpecParam::Goatd { seed, .. } => seed,
            _ => 0,
        }
    }

    /// Whether an elimination order breaks its ties by JW-weighted sampling.
    pub(crate) fn jw_sample(&self) -> bool {
        matches!(
            *self,
            SpecParam::Elimination {
                jw_sample: true,
                ..
            }
        )
    }

    /// Whether the goatd schedule ends in its refinement pass.
    pub(crate) fn refine(&self) -> bool {
        !matches!(*self, SpecParam::Goatd { refine: false, .. })
    }

    /// The partition imbalance for the bisection family, hypergraph or primal.
    pub(crate) fn imbalance(&self) -> f64 {
        match *self {
            SpecParam::Imbalance(v) => v,
            _ => crate::decompose::IMBALANCE_BALANCED,
        }
    }

    /// The whole configuration a `force` spec carries.
    pub(crate) fn force(&self) -> crate::decompose::ForceConfig {
        match *self {
            SpecParam::Force(cfg) => cfg,
            _ => crate::decompose::ForceConfig::new(crate::decompose::ForceMode::Mst),
        }
    }

    /// The search budget for a FlowCutter family, in either of the two shapes
    /// its grammar accepts. There is no budget a caller could mean by default,
    /// so this is the one accessor that reports rather than falls back.
    pub(crate) fn fc_budget(&self, base: &str) -> Result<crate::decompose::FcBudget, VitriError> {
        match *self {
            SpecParam::FcTimed {
                timeout_ms,
                iters,
                patience_ms,
            } => Ok(crate::decompose::FcBudget::timed(
                timeout_ms,
                patience_ms,
                iters,
            )),
            SpecParam::FcSteps { steps, iters } => {
                Ok(crate::decompose::FcBudget::Steps { steps, iters })
            }
            _ => Err(VitriError::spec(
                base,
                "no FlowCutter budget, expected \"budget=<N>ms\" or \"budget=<N>steps\"",
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// The parser
// ---------------------------------------------------------------------------

/// The parser for a *resolved* `--vtree` spec string: one pass that tokenizes,
/// classifies the base, and types the parameters, rejecting anything the spec's
/// family cannot honor.
///
/// Backend-independent: everything is decided from the string.
///
/// Returns [`VitriError::Spec`] naming the offending parameter and the accepting
/// spec form. An unrecognized base parses `Ok` with [`VtreeBase::Unknown`]; the
/// caller's unknown-spec handler reports it.
pub(crate) fn parse_vtree_spec(spec: &str) -> Result<ParsedSpec<'_>, VitriError> {
    let (base, raw_params) = split_vtree_spec(spec);
    let family = classify_base(base);
    let mut params = KeyedParams::new(spec, raw_params)?;
    let written = params.written();

    let mut reading = Reading::default();

    let param = match family {
        // Whole-formula strategies and the simple baselines: each builds one
        // fixed configuration, so no parameter can change what they produce.
        VtreeBase::Balanced
        | VtreeBase::Linear
        | VtreeBase::ReverseLinear
        | VtreeBase::Random
        | VtreeBase::Portfolio => SpecParam::None,

        // goatd: the schedule is fixed by the base and `refine`, and the seed
        // is the rest of it. Its schedule produces decompositions like any
        // other family's, so it reads them the same way.
        VtreeBase::Goatd { .. } => {
            reading = read_reading(&mut params)?;
            SpecParam::Goatd {
                refine: params.enum_value("refine", REFINEMENTS)?.unwrap_or(true),
                seed: params.number("seed", "an integer")?.unwrap_or(0),
            }
        }

        // The single-order elimination family: the base names the order and the
        // graph view, `ties` picks which of the order's two cores runs — where
        // it has two — and the seed drives the tie-breaking either of them
        // does. The seed is NOT inert on the deterministic core: it reaches the
        // elimination's own tie-breaking, and two seeds give two trees.
        VtreeBase::Elimination { name, .. } => {
            // An order with no sampling core never READS `ties`, so writing it
            // there is left for the unused-key refusal — the same answer the
            // help gives by not offering the key on that base.
            let jw_sample = crate::decompose::elimination_order_samples(name)
                && params.enum_value("ties", TIE_BREAKS)?.unwrap_or(false);
            let seed = params.number("seed", "an integer")?.unwrap_or(0);
            // The order is one decomposition, and how it is read is the same
            // question every other family answers, asked the same way.
            reading = read_reading(&mut params)?;
            SpecParam::Elimination { jw_sample, seed }
        }

        // FlowCutter: a search budget in one of two shapes, plus the three
        // conversion parameters that say how to read a vtree off what it found.
        VtreeBase::Flowcutter { .. } => {
            let budget = parse_fc_budget(&mut params, spec)?;
            reading = read_reading(&mut params)?;
            budget
        }

        // `guided-bisect`: a FlowCutter incidence decomposition guiding a
        // recursive bisection. It builds its own edges rather than binarizing the
        // decomposition's bags, so it takes the search budget and none of the
        // conversion parameters.
        VtreeBase::GuidedBisect => parse_fc_budget(&mut params, spec)?,

        // Multilevel bisection, hypergraph or primal: one knob, one default,
        // one validator arm.
        VtreeBase::HypergraphBisect | VtreeBase::PrimalBisect => {
            let v: f64 = params
                .number("imbalance", "a fraction in 0.0..=0.5")?
                .unwrap_or(crate::decompose::IMBALANCE_BALANCED);
            // A range comparison answers `false` for `nan` as well as for the
            // two infinities, so all three land here rather than travelling on
            // as a partition bound no bisection can meet.
            if !(0.0..=0.5).contains(&v) {
                return Err(invalid_token(
                    spec,
                    "imbalance",
                    &v.to_string(),
                    "a finite fraction in 0.0..=0.5",
                ));
            }
            SpecParam::Imbalance(v)
        }

        // Force-directed embedding: the tree-ifier and its eight axes.
        VtreeBase::Force => SpecParam::Force(parse_force_config(&mut params, spec)?),

        // Unrecognized base: nothing will read its parameters, so leave them
        // unchecked and let the caller's unknown-spec handler report the base.
        VtreeBase::Unknown => {
            return Ok(ParsedSpec {
                raw: spec,
                base,
                family,
                param: SpecParam::None,
                reading,
                written,
            });
        }
    };

    params.finish(family, base)?;

    Ok(ParsedSpec {
        raw: spec,
        base,
        family,
        param,
        reading,
        written,
    })
}

/// Strict single-pass validator for a *resolved* `--vtree` spec string: the ONE
/// parse with its value dropped, so validation and construction cannot disagree
/// about what a spec means.
///
/// Recognised specs, in dispatch order:
/// 1. **Named simple vtrees** — `balanced`, `linear`, `reverse-linear`, `random`.
/// 2. **TD-based vtrees** — `goatd-primal` / `goatd-incidence` and the
///    FlowCutter pair `flowcutter-primal` / `flowcutter-incidence`, each naming
///    the graph view it decomposes.
/// 3. **Portfolio** — `portfolio`.
/// 4. **Single elimination orders** — `minfill`, `mindegree` and
///    `nested-dissection` (`crate::decompose::elimination_spec_names`), each in
///    both graph views.
/// 5. **Single-configuration backends** — the bisection pair
///    `hypergraph-bisect` / `primal-bisect` and the force-directed embedding
///    `force`.
///
/// Every one of them takes its parameters as `:key=value`, comma separated;
/// [`spec_param_docs`] is the per-base list.
///
/// # Errors
///
/// [`VitriError::Spec`] naming the offending parameter and the accepting spec
/// form. `Ok(())` when every parameter is consumed by the spec's family — or
/// when the base is unrecognized, in which case the downstream unknown-spec
/// handler reports it.
pub fn validate_vtree_spec(spec: &str) -> Result<(), VitriError> {
    parse_vtree_spec(spec).map(|_| ())
}

/// Read a FlowCutter search budget out of the spec's parameters: timed
/// (`budget=<N>ms`, with `iters=` and `patience=`) or step-budgeted
/// (`budget=<N>steps`, with `iters=`).
///
/// An absent `budget=` is not the same as a written one: it resolves to the
/// timed defaults a bare spec has always meant, whose patience differs from the
/// written form's.
fn parse_fc_budget(params: &mut KeyedParams<'_>, spec: &str) -> Result<SpecParam, VitriError> {
    let written = params.take("budget");
    let iters_key = "iters";
    match written {
        Some(v) if v.ends_with("steps") => {
            let steps: i64 = v
                .trim_end_matches("steps")
                .parse()
                .map_err(|_| invalid_token(spec, "budget", v, "<N>steps"))?;
            let iters = params
                .number(iters_key, "an integer")?
                .unwrap_or(FC_DEFAULT_STEPS_ITERS);
            // Patience bounds a wall-clock search; the step budget has no clock
            // to bound, so naming it here would set nothing.
            if params.wrote("patience") {
                return Err(VitriError::spec(
                    spec,
                    "\"patience=\" bounds the timed search and has nothing to bound in the \
                     step-budgeted \"budget=<N>steps\" mode",
                ));
            }
            Ok(SpecParam::FcSteps { steps, iters })
        }
        Some(v) => {
            let timeout_ms: i64 = v
                .trim_end_matches("ms")
                .parse()
                .map_err(|_| invalid_token(spec, "budget", v, "<N>ms or <N>steps"))?;
            if !v.ends_with("ms") {
                return Err(invalid_token(spec, "budget", v, "<N>ms or <N>steps"));
            }
            Ok(SpecParam::FcTimed {
                timeout_ms,
                iters: params
                    .number(iters_key, "an integer")?
                    .unwrap_or(FC_DEFAULT_ITERS),
                patience_ms: params
                    .number("patience", "milliseconds")?
                    .unwrap_or(FC_PATIENCE_MS_PARAMETRIZED),
            })
        }
        None => Ok(SpecParam::FcTimed {
            timeout_ms: FC_BARE_TIMEOUT_MS,
            iters: params
                .number(iters_key, "an integer")?
                .unwrap_or(FC_DEFAULT_ITERS),
            patience_ms: params
                .number("patience", "milliseconds")?
                .unwrap_or(FC_PATIENCE_MS_BARE),
        }),
    }
}

/// Parse (and validate) a `force` spec into a
/// [`ForceConfig`](crate::decompose::ForceConfig) — the SINGLE place the axis
/// grammar is read, reached from [`parse_vtree_spec`] and therefore from both
/// the validator and the builder.
///
/// `root=`, `orient=`, `weights=` and `feedback=` reshape the MST, so they are
/// refused under `treeify=cut`, which has no MST to reshape; `clause-weight=`,
/// `dim=`, `restarts=` and `init=` apply to both tree-ifiers.
fn parse_force_config(params: &mut KeyedParams<'_>, spec: &str) -> Result<ForceConfig, VitriError> {
    let mode = params
        .enum_value("treeify", FORCE_TREEIFIERS)?
        .unwrap_or(ForceMode::Mst);
    // Reported before any axis is read, so a spec that names an MST axis under
    // `treeify=cut` is told about the tree-ifier rather than about the axis.
    if mode != ForceMode::Mst
        && let Some(key) = FORCE_MST_ONLY_KEYS.iter().find(|k| params.wrote(k))
    {
        return Err(VitriError::spec(
            spec,
            format!(
                "\"{key}=\" reshapes the MST and cannot combine with \"treeify=cut\", which \
                 selects the median-cut tree-ifier and has no MST to reshape"
            ),
        ));
    }
    let mut cfg = ForceConfig::new(mode);
    if let Some(v) = params.enum_value("root", FORCE_ROOTS)? {
        cfg.root = v;
    }
    if let Some(v) = params.enum_value("orient", FORCE_ORIENTS)? {
        cfg.orient = v;
    }
    if let Some(v) = params.enum_value("weights", FORCE_WEIGHTS)? {
        cfg.weight = v;
    }
    if let Some(v) = params.enum_value("clause-weight", FORCE_CLAUSE_WEIGHTS)? {
        cfg.clause_weight = v;
    }
    if let Some(v) = params.enum_value("init", FORCE_INITS)? {
        cfg.init = v;
    }
    if let Some(v) = params.number::<usize>("dim", &force_dim_range())? {
        if !(2..=crate::decompose::FORCE_MAX_DIM).contains(&v) {
            return Err(invalid_token(
                spec,
                "dim",
                &v.to_string(),
                &force_dim_range(),
            ));
        }
        cfg.dim = v;
    }
    if let Some(v) = params.number::<u8>("feedback", "an integer 0..=8")? {
        if v > 8 {
            return Err(invalid_token(
                spec,
                "feedback",
                &v.to_string(),
                "an integer 0..=8",
            ));
        }
        cfg.fb = v;
    }
    if let Some(v) = params.number::<u8>("restarts", "an integer 1..=16")? {
        if !(1..=16).contains(&v) {
            return Err(invalid_token(
                spec,
                "restarts",
                &v.to_string(),
                "an integer 1..=16",
            ));
        }
        cfg.seeds = v;
    }
    Ok(cfg)
}

/// The error for a base no backend builds — the one place its wording lives.
/// Both name lists come from the tables the parser matches against, so a name
/// added to either is offered here without a second edit.
pub(super) fn unknown_vtree_type(spec: &str) -> VitriError {
    let bases: Vec<&str> = VTREE_BASE_NAMES.iter().map(|b| b.name).collect();
    let orders: Vec<&str> = crate::decompose::elimination_spec_names().collect();
    VitriError::spec(
        spec,
        format!(
            "unknown vtree type, expected {}, or one of the elimination orders {}, each \
             written with the graph view it runs on ({})",
            one_of(bases),
            one_of(orders),
            one_of(
                crate::decompose::VIEW_SUFFIXES
                    .iter()
                    .map(|(suffix, _)| format!("\"<order>{suffix}\""))
            ),
        ),
    )
}
