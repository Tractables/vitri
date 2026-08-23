//! The `--vtree` spec grammar: the vocabulary of construction names, the
//! `:param` and `/suffix` tokens each family accepts, and the typed
//! [`ParsedSpec`] every backend is handed.
//!
//! One reader. [`parse_vtree_spec`] is the only thing that looks at a spec
//! string; [`validate_vtree_spec`] is that parse with the value dropped, and
//! [`super::build_one_vtree_artifacts`] dispatches on what came out. A spec
//! that validates is therefore exactly a spec some backend can build.

use crate::decompose::{
    BagAssignment, ClauseWeight, FC_BARE_TIMEOUT_MS, FC_DEFAULT_ITERS, FC_DEFAULT_STEPS_ITERS,
    FC_PATIENCE_MS_BARE, FC_PATIENCE_MS_PARAMETRIZED, ForceConfig, ForceMode, InitMode,
    ItemOrdering, OrientRule, RootRule, TdRootStrategy, TdToVtreeConfig, VarOrderInBag, WeightRule,
};
use crate::error::VitriError;

/// Reject one token of `spec`: `what` it was read as, the token itself, and the
/// form that would have been accepted.
///
/// The one place this file's rejections are worded, in the house style
/// [`crate::error`]'s module doc fixes — so a grammar rule added below is
/// reported the way every other one already is.
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

/// Does `spec` name a construction that builds and scores several candidate
/// vtrees, and therefore has a candidate set to retain ([`crate::candidates`])?
///
/// True only for the portfolio — every other spec builds exactly one vtree, so
/// a retained candidate set could never hold more than that entry. The config
/// validator uses this to refuse an inert `candidates > 1`.
pub(crate) fn spec_has_candidates(spec: &str) -> bool {
    matches!(classify_base(vtree_spec_base(spec)), VtreeBase::Portfolio)
}

/// What one `/`-suffix does: it sets one field of a [`TdToVtreeConfig`], and
/// this is which field, carrying the value the suffix names.
#[derive(Clone, Copy)]
enum SuffixEffect {
    /// Sets `bag_assignment` — the only field the step-budgeted FlowCutter mode
    /// reads, since that mode assembles the vtree from the bag assignment alone
    /// and nothing else can reach it.
    Bag(BagAssignment),
    /// Sets `root_strategy`.
    Root(TdRootStrategy),
    /// Sets `var_order`.
    VarOrder(VarOrderInBag),
    /// Sets `item_ordering`. `/best` cannot be combined with one of these: it
    /// ranks pre-built TD candidates and ignores the parsed ordering, so the
    /// ordering would be silently dropped (the no-silent-no-op rule).
    /// Bag/root/affinity suffixes are tolerated alongside `/best`.
    Ordering(ItemOrdering),
}

impl SuffixEffect {
    /// Writes this suffix's setting into `cfg`.
    fn apply(self, cfg: &mut TdToVtreeConfig) {
        match self {
            SuffixEffect::Bag(v) => cfg.bag_assignment = v,
            SuffixEffect::Root(v) => cfg.root_strategy = v,
            SuffixEffect::VarOrder(v) => cfg.var_order = v,
            SuffixEffect::Ordering(v) => cfg.item_ordering = v,
        }
    }
}

/// One construction suffix: the name a spec writes after the `/`, and the field
/// writing it sets.
struct TdSuffix {
    /// The name, without the leading slash.
    name: &'static str,
    /// What naming it does.
    effect: SuffixEffect,
}

impl TdSuffix {
    /// One row of [`TD_SUFFIXES`].
    const fn new(name: &'static str, effect: SuffixEffect) -> Self {
        Self { name, effect }
    }
}

/// Every construction suffix, in the order the messages offer them. `/best` is
/// not one of them — it selects a ranking rather than setting a field, so it is
/// handled beside this table rather than in it.
///
/// The single source for this grammar: [`parse_vtree_spec`] rejects a name this
/// table does not hold and applies the effect of one it does, both by reading
/// it, as do the `/best` conflict rule, the step-budgeted mode's inert-suffix
/// check and every message that lists suffixes. A row added here therefore
/// teaches the whole grammar at once.
const TD_SUFFIXES: &[TdSuffix] = &[
    TdSuffix::new("shallow", SuffixEffect::Bag(BagAssignment::Shallowest)),
    TdSuffix::new("deep", SuffixEffect::Bag(BagAssignment::Deepest)),
    TdSuffix::new("centroid", SuffixEffect::Root(TdRootStrategy::Centroid)),
    TdSuffix::new("first-bag", SuffixEffect::Root(TdRootStrategy::FirstBag)),
    TdSuffix::new(
        "affinity",
        SuffixEffect::VarOrder(VarOrderInBag::ClauseAffinity),
    ),
    TdSuffix::new(
        "vars-first",
        SuffixEffect::Ordering(ItemOrdering::VariablesFirst),
    ),
    TdSuffix::new(
        "children-by-size",
        SuffixEffect::Ordering(ItemOrdering::ChildrenBySize),
    ),
    TdSuffix::new(
        "clause-split",
        SuffixEffect::Ordering(ItemOrdering::ClauseSplit),
    ),
    TdSuffix::new("left-deep", SuffixEffect::Ordering(ItemOrdering::LeftDeep)),
    TdSuffix::new(
        "largest-first",
        SuffixEffect::Ordering(ItemOrdering::LargestFirst),
    ),
    TdSuffix::new(
        "hypergraph-bisect",
        SuffixEffect::Ordering(ItemOrdering::HypergraphBisect),
    ),
    TdSuffix::new(
        "boundary-adjacent",
        SuffixEffect::Ordering(ItemOrdering::BoundaryAdjacent),
    ),
    // The TD-edge-aligned combiner realizes its separator-lifting only under
    // shallowest assignment — pair it with `/shallow`
    // (e.g. `flowcutter-incidence/td-edge/shallow`).
    TdSuffix::new(
        "td-edge",
        SuffixEffect::Ordering(ItemOrdering::TdEdgeAligned),
    ),
];

/// The suffix `name` names, or `None` when the grammar has no suffix by that
/// name.
fn td_suffix(name: &str) -> Option<&'static TdSuffix> {
    TD_SUFFIXES.iter().find(|s| s.name == name)
}

/// Does `name` name a suffix that sets the item ordering?
fn is_ordering_suffix(name: &str) -> bool {
    matches!(
        td_suffix(name).map(|s| s.effect),
        Some(SuffixEffect::Ordering(_))
    )
}

/// Does `name` name a suffix that sets the bag assignment?
fn is_bag_assignment_suffix(name: &str) -> bool {
    matches!(
        td_suffix(name).map(|s| s.effect),
        Some(SuffixEffect::Bag(_))
    )
}

/// The suffixes whose effect `keep` accepts, in table order, each written with
/// the leading slash a spec puts on it — how a message offers them.
fn suffix_names(keep: fn(SuffixEffect) -> bool) -> Vec<String> {
    TD_SUFFIXES
        .iter()
        .filter(|s| keep(s.effect))
        .map(|s| format!("/{}", s.name))
        .collect()
}

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
    VtreeBaseName::new("flowcutter-primal", VtreeBase::FlowcutterPrimal),
    VtreeBaseName::new("flowcutter-incidence", VtreeBase::FlowcutterIncidence),
    VtreeBaseName::new(
        "hybrid-flowcutter-incidence",
        VtreeBase::HybridFlowcutterIncidence,
    ),
    VtreeBaseName::new("goatd", VtreeBase::Goatd { incidence: true }),
    VtreeBaseName::new("goatd-primal", VtreeBase::Goatd { incidence: false }),
    VtreeBaseName::new("hypergraph-bisect", VtreeBase::HypergraphBisect),
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
        | VtreeBase::FlowcutterPrimal
        | VtreeBase::FlowcutterIncidence
        | VtreeBase::HybridFlowcutterIncidence
        | VtreeBase::HypergraphBisect => BaseGroup::Decomposition,
        VtreeBase::Elimination { .. } | VtreeBase::Unknown => return None,
    })
}

/// Base-name family of a `--vtree` spec, after its `:param` and `/suffix`
/// tokens have been stripped. One variant per base family that the CLI knows
/// how to validate and/or build. [`classify_base`] is the single place base
/// strings are matched, so every consumer agrees on which family a base
/// belongs to; the match on this enum is exhaustive with no wildcard arm, so
/// adding a variant here is a compile error until every consumer handles it.
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
    /// No `:seed` token is accepted.
    Random,
    /// Matches `portfolio`: several FlowCutter portfolio candidates plus
    /// goatd, keeping the best-scoring candidate.
    Portfolio,
    /// Matches `goatd` and `goatd-primal`: a scheduled, selected
    /// tree-decomposition-to-vtree construction; an optional `:<seed>` selects
    /// the RNG seed.
    Goatd {
        /// `goatd` runs on the incidence graph, `goatd-primal` on the primal
        /// one — which of the two base names was written, resolved here so no
        /// backend re-reads it.
        incidence: bool,
    },
    /// Matches the single-order elimination family — `minfill`, `mindegree` and
    /// their siblings, each also with an `-inc` suffix for the incidence graph
    /// ([`crate::decompose::elimination_spec_names`] is the name list). One
    /// fixed elimination order, no schedule and no refinement; an optional
    /// `:<seed>` selects the RNG seed and no `/suffix` is accepted.
    Elimination {
        /// The construction this base names, as
        /// [`crate::decompose::elimination_spec`] resolved it — a `'static`
        /// name from the construction table, so an error can quote it.
        name: &'static str,
        /// Whether the `-inc` suffix asked for the incidence graph.
        incidence: bool,
    },
    /// Matches `flowcutter-primal`: in-process FlowCutter run on the CNF's primal
    /// graph (variables only).
    FlowcutterPrimal,
    /// Matches `flowcutter-incidence`: in-process FlowCutter run on the incidence graph
    /// (variables and clauses both as vertices).
    FlowcutterIncidence,
    /// Matches `hybrid-flowcutter-incidence`: the incidence FlowCutter
    /// decomposition assembled by the hybrid decomposition + bisection rule
    /// rather than by a plain conversion. Takes the same `:param` effort token
    /// as `flowcutter-incidence` — which decomposition it combines is the only
    /// thing that token decides — and no `/suffix`.
    HybridFlowcutterIncidence,
    /// Matches `hypergraph-bisect`: multilevel hypergraph bisection. Takes an
    /// optional `:<imbalance>` float parameter (default when omitted); no
    /// `/suffix` is accepted.
    HypergraphBisect,
    /// Matches `force`: the force-directed embedding of the variables, tree-ified
    /// by MST or median cut. Owns its own `/key=value` suffix grammar
    /// ([`parse_force_config`]) rather than the shared TD suffixes, and takes an
    /// optional `:mst|:cut` tree-ifier parameter.
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
/// the base already stripped of any `:param` and `/suffix` (callers compute
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

/// Tokenize a `--vtree` spec into `(base, param, suffixes)` —
/// `<base>[:param](/suffix)*`. Bases and params never contain '/'.
fn split_vtree_spec(spec: &str) -> (&str, Option<&str>, Vec<&str>) {
    let (head, suffixes): (&str, Vec<&str>) = match spec.find('/') {
        Some(i) => (
            &spec[..i],
            spec[i..].split('/').filter(|s| !s.is_empty()).collect(),
        ),
        None => (spec, Vec::new()),
    };
    let (base, param) = match head.split_once(':') {
        Some((b, p)) => (b, Some(p)),
        None => (head, None),
    };
    (base, param, suffixes)
}

/// The base-name head of a spec, with `:param`/`/suffix` stripped.
pub(crate) fn vtree_spec_base(spec: &str) -> &str {
    split_vtree_spec(spec).0
}

/// A `--vtree` spec string after the one parse: its family, its typed `:param`,
/// and the [`TdToVtreeConfig`] its `/suffix` tokens set.
///
/// [`parse_vtree_spec`] is the only thing that reads the grammar.
/// [`validate_vtree_spec`] is that parse with the value dropped, and
/// [`build_one_vtree_artifacts`] hands this straight to the construction
/// backends — so no backend re-reads the string, and a spec that validates is
/// exactly a spec some backend can build.
pub(crate) struct ParsedSpec<'a> {
    /// The spec exactly as it was written, for the one message that names the
    /// whole string rather than an offending token.
    pub raw: &'a str,
    /// The base-name head, `:param` and `/suffix` tokens stripped.
    pub base: &'a str,
    /// Which family [`classify_base`] put `base` in.
    pub family: VtreeBase,
    /// The `:param`, checked and typed for the family.
    pub param: SpecParam,
    /// The TD→vtree options the `/suffix` tokens set.
    pub td_config: TdToVtreeConfig,
    /// `/best` was given: rank internally-built TD candidates instead of
    /// building the one configuration the suffixes describe.
    pub use_best: bool,
}

/// The typed `:param` of a spec, one variant per shape the grammar accepts.
/// A family that takes no parameter — and an unrecognized base, whose parameter
/// nothing will read — carries [`SpecParam::None`].
///
/// The parse types each family's parameter, so a build site is never handed a
/// variant its family does not carry: the accessors below answer such a variant
/// with that family's documented default, which is also what an absent `:param`
/// parses to, and the one accessor with no default to reach for
/// ([`SpecParam::fc_budget`]) reports against the base instead. Said once here,
/// so no accessor and no build site says it again.
pub(crate) enum SpecParam {
    /// No `:param` token, or a family that takes none.
    None,
    /// `goatd[-primal]:<u64>` and `<elimination-order>:<u64>` — the RNG seed for
    /// elimination tie-breaking and refinement sampling. Absent means seed 0.
    Seed(u64),
    /// `hypergraph-bisect:<f64>` — the partition imbalance, a fraction in
    /// `0.0..=1.0`.
    Imbalance(f64),
    /// FlowCutter timed mode: `:<N>ms[,<N>i][,<N>p]`, and also what a BARE
    /// `flowcutter-primal` / `flowcutter-incidence` (or either combiner spec
    /// over an incidence decomposition) resolves to — those defaults differ
    /// from the parametrized form's, see
    /// [`FC_PATIENCE_MS_BARE`](crate::decompose::FC_PATIENCE_MS_BARE).
    FcTimed {
        /// Wall-clock budget for the timed search.
        timeout_ms: i64,
        /// FlowCutter iteration cap.
        iters: i32,
        /// Milliseconds without an improvement before the search gives up.
        patience_ms: i64,
    },
    /// FlowCutter step-budgeted mode: `:<N>[,<iters>]steps`.
    FcSteps {
        /// Computation-step budget handed to FlowCutter.
        steps: i64,
        /// FlowCutter iteration count.
        iters: i32,
    },
    /// `force` — the whole configuration, read from both the `:mst|:cut`
    /// parameter and the `/key=value` suffixes by [`parse_force_config`]. That
    /// family's suffixes are axis settings rather than TD-config names, so its
    /// parse result travels here instead of in `ParsedSpec::td_config`.
    Force(crate::decompose::ForceConfig),
}

impl SpecParam {
    /// The seed for a family whose parameter is one; 0 otherwise.
    pub(crate) fn seed(&self) -> u64 {
        match *self {
            SpecParam::Seed(s) => s,
            _ => 0,
        }
    }

    /// The partition imbalance for the hypergraph-bisection family.
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
                "no FlowCutter budget, expected a timed \":<N>ms\" or step-budgeted \
                 \":<N>[,<iters>]steps\" parameter",
            )),
        }
    }
}

/// Whether the step-budgeted FlowCutter mode (`:<N>[,<iters>]steps`) honors
/// `/{suffix}`. That mode assembles from the bag assignment alone, so the
/// suffixes it reads are exactly the bag assignments and every other one —
/// `/best` included — would be silently dropped.
///
/// One reader for that rule: [`parse_vtree_spec`] rejects the suffixes this
/// returns `false` for, and [`honors_best_suffix`] answers for `/best` through
/// it rather than restating which mode ranks candidates.
fn step_budget_honors(suffix: &str) -> bool {
    is_bag_assignment_suffix(suffix)
}

/// Whether appending `/best` to `spec` — which must carry no `/suffix` of its
/// own — yields a spec this grammar honors.
///
/// The one owner of the question "can this spec take `/best`":
/// `component::spec_for_size` asks it before upgrading a bare TD-based
/// spec on a small formula, so the upgrade cannot construct a spec that
/// [`parse_vtree_spec`] then refuses as a silent no-op.
pub(crate) fn honors_best_suffix(spec: &str) -> bool {
    let (base, param, _) = split_vtree_spec(spec);
    match classify_base(base) {
        // goatd ranks its own candidates whatever the spec says, so `/best`
        // names what the family already does and is accepted as such.
        VtreeBase::Goatd { .. } => true,
        // FlowCutter ranks internally-built TD candidates in timed mode only.
        // A parameter this family cannot read is not this predicate's to
        // report — leave the upgrade alone and let the parse name the token.
        VtreeBase::FlowcutterPrimal | VtreeBase::FlowcutterIncidence => match param {
            Some(p) => match parse_fc_param(spec, base, p) {
                Ok(SpecParam::FcSteps { .. }) => step_budget_honors("best"),
                _ => true,
            },
            None => true,
        },
        // Every other family builds one fixed configuration, with no candidate
        // list for `/best` to rank.
        _ => false,
    }
}

/// The parser for a *resolved* `--vtree` spec string: one pass that tokenizes,
/// classifies the base, applies the `/suffix` tokens and types the `:param`,
/// rejecting anything the spec's family cannot honor.
///
/// Backend-independent: everything is decided from the string.
///
/// Returns [`VitriError::Spec`] naming the offending token and the accepting
/// spec form. An unrecognized base parses `Ok` with [`VtreeBase::Unknown`]; the
/// caller's unknown-spec handler reports it.
pub(crate) fn parse_vtree_spec(spec: &str) -> Result<ParsedSpec<'_>, VitriError> {
    let (base, param, suffixes) = split_vtree_spec(spec);
    let family = classify_base(base);

    // Any suffix that is neither a known config suffix nor `/best` is illegal
    // whatever the family, so this runs before the per-family rules below.
    //
    // `force` is the one family exempt from it: its suffixes are `key=value` axis
    // settings from a grammar of its own ([`parse_force_config`]), which would
    // otherwise be wrongly rejected here. That parser still rejects an unknown
    // key, a malformed suffix, and a duplicated one, by name.
    if family != VtreeBase::Force {
        for s in &suffixes {
            if *s != "best" && td_suffix(s).is_none() {
                return Err(invalid_token(
                    spec,
                    "suffix",
                    &format!("/{s}"),
                    &format!(
                        "/best or one of the td-vtree config suffixes {}",
                        one_of(suffix_names(|_| true)),
                    ),
                ));
            }
        }
    }

    // Apply the suffixes RIGHT TO LEFT so that when two of them set the same
    // field the LEFTMOST wins.
    let mut td_config = TdToVtreeConfig::default();
    let mut use_best = false;
    for s in suffixes.iter().rev() {
        if *s == "best" {
            use_best = true;
        } else if let Some(suffix) = td_suffix(s) {
            suffix.effect.apply(&mut td_config);
        }
        // Anything else is one of `force`'s `key=value` axis settings, which
        // the loop above exempts from rejection and [`parse_force_config`]
        // reads; every other unknown name was rejected there.
    }

    let no_param =
        |p: &str| VitriError::spec(spec, format!("{base:?} takes no parameter (got \":{p}\")"));
    let no_suffix =
        |s: &str| VitriError::spec(spec, format!("{base:?} takes no suffix (got \"/{s}\")"));

    // The `:<u64>` RNG seed both seeded families take, defaulting to 0 when the
    // token is absent. One reader, so the two arms cannot drift apart.
    let seed_param = |p: Option<&str>| match p {
        Some(p) => Ok(SpecParam::Seed(p.parse::<u64>().map_err(|_| {
            invalid_token(spec, "seed", &format!(":{p}"), ":<u64>")
        })?)),
        None => Ok(SpecParam::Seed(0)),
    };

    let param = match family {
        // Whole-formula strategies and the simple baselines: each builds one
        // fixed configuration, so neither a `:param` nor any `/suffix` —
        // `/best` included — can change what they produce.
        VtreeBase::Balanced
        | VtreeBase::Linear
        | VtreeBase::ReverseLinear
        | VtreeBase::Random
        | VtreeBase::Portfolio => {
            if let Some(p) = param {
                return Err(no_param(p));
            }
            if let Some(s) = suffixes.first() {
                return Err(no_suffix(s));
            }
            SpecParam::None
        }

        // goatd: one fixed configuration per base, so the only suffix that is
        // not a silent no-op is `/best` — which this family ignores, and which
        // exists because a caller may append it generically. The param is the
        // RNG `:<seed>`.
        VtreeBase::Goatd { .. } => {
            for s in &suffixes {
                if *s != "best" {
                    return Err(VitriError::spec(
                        spec,
                        format!(
                            "suffix \"/{s}\" is not honored by {base:?} — this family builds one \
                             fixed configuration and ignores item-ordering/bag suffixes (only \
                             \"/best\" is accepted, and it is the default). Use a \
                             \"flowcutter-*\" spec to apply construction suffixes",
                        ),
                    ));
                }
            }
            seed_param(param)?
        }

        // The single-order elimination family: the base names the one
        // elimination order it runs, so no `/suffix` at all — `/best` included,
        // since there is no candidate list to rank. The param is the RNG
        // `:<seed>`, which the `*-sample` constructions consume.
        VtreeBase::Elimination { .. } => {
            if let Some(s) = suffixes.first() {
                return Err(no_suffix(s));
            }
            seed_param(param)?
        }

        // FlowCutter: `flowcutter-{primal,incidence}[:param][/suffix...]`. `/best` is exclusive
        // with the item orderings (it ranks internally-built TD candidates and
        // ignores them), and the param has two shapes plus a bare default.
        VtreeBase::FlowcutterPrimal | VtreeBase::FlowcutterIncidence => {
            if use_best && let Some(ord) = suffixes.iter().find(|s| is_ordering_suffix(s)) {
                return Err(VitriError::spec(
                    spec,
                    format!(
                        "\"/best\" cannot be combined with the item-ordering suffix \"/{ord}\" — \
                         \"/best\" ranks internally-built TD candidates and ignores the ordering, \
                         so \"/{ord}\" would be silently dropped. Use exactly one"
                    ),
                ));
            }
            match param {
                None => SpecParam::FcTimed {
                    timeout_ms: FC_BARE_TIMEOUT_MS,
                    iters: FC_DEFAULT_ITERS,
                    patience_ms: FC_PATIENCE_MS_BARE,
                },
                Some(p) => {
                    let parsed = parse_fc_param(spec, base, p)?;
                    if let SpecParam::FcSteps { .. } = parsed
                        && let Some(inert) = suffixes.iter().find(|s| !step_budget_honors(s))
                    {
                        return Err(VitriError::spec(
                            spec,
                            format!(
                                "the step-budgeted \":{p}\" mode builds from the bag assignment \
                                 alone, so \"/{inert}\" has nothing to set and would be silently \
                                 dropped. Keep only {}, or use the timed \":<N>ms\" mode",
                                one_of(suffix_names(|e| matches!(e, SuffixEffect::Bag(_)))),
                            ),
                        ));
                    }
                    parsed
                }
            }
        }

        // The combiner over a FlowCutter incidence decomposition. Its `:param`
        // is the same effort token `flowcutter-incidence` takes: how hard
        // FlowCutter looks for the decomposition it then combines. Naming the
        // portfolio's own effort (`:150000,15steps`) reproduces the portfolio
        // candidate of this name exactly.
        //
        // No `/suffix` at all — the name is one fixed assembly rule, so a
        // bag/ordering/root suffix has nothing to set and `/best` has no
        // candidate list to rank (no-silent-no-op rule).
        VtreeBase::HybridFlowcutterIncidence => {
            if let Some(s) = suffixes.first() {
                return Err(no_suffix(s));
            }
            match param {
                None => SpecParam::FcTimed {
                    timeout_ms: FC_BARE_TIMEOUT_MS,
                    iters: FC_DEFAULT_ITERS,
                    patience_ms: FC_PATIENCE_MS_BARE,
                },
                Some(p) => parse_fc_param(spec, base, p)?,
            }
        }

        // Multilevel-hypergraph bisection: optional `:<imbalance>`, no suffix.
        VtreeBase::HypergraphBisect => {
            if let Some(s) = suffixes.first() {
                return Err(no_suffix(s));
            }
            match param {
                Some(p) => {
                    let v: f64 = p.parse().map_err(|_| {
                        invalid_token(spec, "imbalance", &format!(":{p}"), ":<f64>")
                    })?;
                    // A range comparison answers `false` for `nan` as well as
                    // for the two infinities, so all three land here rather
                    // than travelling on as a partition bound no bisection can
                    // meet.
                    if !(0.0..=1.0).contains(&v) {
                        return Err(invalid_token(
                            spec,
                            "imbalance",
                            &format!(":{p}"),
                            "a finite fraction in 0.0..=1.0",
                        ));
                    }
                    SpecParam::Imbalance(v)
                }
                None => SpecParam::Imbalance(crate::decompose::IMBALANCE_BALANCED),
            }
        }

        // Force-directed embedding: `force[:mst|:cut](/key=value)*`. The axis
        // grammar lives in one place, parsed here so the validator and the
        // builder agree.
        VtreeBase::Force => SpecParam::Force(parse_force_config(spec, param, &suffixes)?),

        // Unrecognized base: nothing will read its parameter, so leave it
        // unchecked and let the caller's unknown-spec handler report the base.
        VtreeBase::Unknown => SpecParam::None,
    };

    Ok(ParsedSpec {
        raw: spec,
        base,
        family,
        param,
        td_config,
        use_best,
    })
}

/// Strict single-pass validator for a *resolved* `--vtree` spec string: the ONE
/// parse with its value dropped, so validation and construction cannot disagree
/// about what a spec means.
///
/// Recognised specs, in dispatch order:
/// 1. **Named simple vtrees** — `balanced`, `linear`, `reverse-linear`, `random`.
/// 2. **TD-based vtrees** — `goatd`, `goatd-primal`, and the FlowCutter pair
///    `flowcutter-primal` / `flowcutter-incidence` (primal graph vs incidence
///    graph) `[:param][/suffix…]`, plus the combiner over an incidence
///    decomposition, `hybrid-flowcutter-incidence` `[:param]`.
/// 3. **Portfolio** — `portfolio`.
/// 4. **Single elimination orders** — `minfill`, `mindegree` and their siblings
///    (`crate::decompose::elimination_spec_names`), each also `-inc` and each
///    taking an optional `:<seed>`.
/// 5. **Single-configuration backends** — `hypergraph-bisect` and the
///    force-directed embedding `force[:mst|:cut][/<axis>=<value>…]`.
///
/// # Errors
///
/// [`VitriError::Spec`] naming the offending token and the accepting spec form.
/// `Ok(())` when every token is consumed by the spec's family — or when the base
/// is unrecognized, in which case the downstream unknown-spec handler reports it.
pub fn validate_vtree_spec(spec: &str) -> Result<(), VitriError> {
    parse_vtree_spec(spec).map(|_| ())
}

/// One `force` axis: the `/<key>=<value>` suffix that sets it, whether it
/// reshapes the MST, how a rejection names it, and the one function that reads
/// its value.
struct ForceAxis {
    /// The key, without the `/` or the `=`.
    key: &'static str,
    /// Reshapes the MST, so the median-cut tree-ifier has nothing for it to
    /// act on and naming it under `:cut` is an error.
    mst_only: bool,
    /// What a rejection calls this axis — "root rule", "embedding dimension".
    what: &'static str,
    /// The values it accepts, as a rejection spells them out.
    allowed: &'static str,
    /// Writes `val` into the config; `false` when `val` is not one of
    /// `allowed`, which is what turns into that rejection.
    apply: fn(&mut ForceConfig, &str) -> bool,
}

impl ForceAxis {
    /// An axis that reshapes the MST.
    const fn mst(
        key: &'static str,
        what: &'static str,
        allowed: &'static str,
        apply: fn(&mut ForceConfig, &str) -> bool,
    ) -> Self {
        Self {
            key,
            mst_only: true,
            what,
            allowed,
            apply,
        }
    }

    /// An axis both tree-ifiers read.
    const fn shared(
        key: &'static str,
        what: &'static str,
        allowed: &'static str,
        apply: fn(&mut ForceConfig, &str) -> bool,
    ) -> Self {
        Self {
            key,
            mst_only: false,
            what,
            allowed,
            apply,
        }
    }
}

/// Every `force` axis, in the order the messages and `--help` offer them.
///
/// The single source for this grammar: [`parse_force_config`] accepts exactly
/// these keys, refuses an MST-only one under `:cut`, reads each value through
/// the row's own setter, and lists the keys in its rejections — all from here.
const FORCE_AXES: &[ForceAxis] = &[
    ForceAxis::mst("root", "root rule", "merge, balance or hybrid", set_root),
    ForceAxis::mst("orient", "orientation rule", "x, small or big", set_orient),
    ForceAxis::mst("w", "edge weight", "euclid or co", set_weight),
    ForceAxis::shared("cw", "clause weight", "uniform or short", set_clause_weight),
    ForceAxis::shared("d", "embedding dimension", "2, 3 or 4", set_dim),
    ForceAxis::mst("fb", "feedback round count", "an integer 0..=8", set_fb),
    // Restarts apply to both tree-ifiers: the keep-best load objective is
    // computed from the finished vtree, not from the MST.
    ForceAxis::shared("seeds", "restart count", "an integer 1..=16", set_seeds),
    ForceAxis::shared("init", "layout initialization", "rand or force1d", set_init),
];

/// The `force` axis keys, in table order.
pub(crate) fn force_axis_names() -> impl Iterator<Item = &'static str> {
    FORCE_AXES.iter().map(|a| a.key)
}

fn set_root(cfg: &mut ForceConfig, val: &str) -> bool {
    cfg.root = match val {
        "merge" => RootRule::Merge,
        "balance" => RootRule::Balance,
        "hybrid" => RootRule::Hybrid,
        _ => return false,
    };
    true
}

fn set_orient(cfg: &mut ForceConfig, val: &str) -> bool {
    cfg.orient = match val {
        "x" => OrientRule::X,
        "small" => OrientRule::Small,
        "big" => OrientRule::Big,
        _ => return false,
    };
    true
}

fn set_weight(cfg: &mut ForceConfig, val: &str) -> bool {
    cfg.weight = match val {
        "euclid" => WeightRule::Euclid,
        "co" => WeightRule::Co,
        _ => return false,
    };
    true
}

fn set_clause_weight(cfg: &mut ForceConfig, val: &str) -> bool {
    cfg.clause_weight = match val {
        "uniform" => ClauseWeight::Uniform,
        "short" => ClauseWeight::Short,
        _ => return false,
    };
    true
}

fn set_dim(cfg: &mut ForceConfig, val: &str) -> bool {
    cfg.dim = match val {
        "2" => 2,
        "3" => 3,
        "4" => 4,
        _ => return false,
    };
    true
}

fn set_fb(cfg: &mut ForceConfig, val: &str) -> bool {
    match val.parse::<u8>() {
        Ok(v) if v <= 8 => cfg.fb = v,
        _ => return false,
    }
    true
}

fn set_seeds(cfg: &mut ForceConfig, val: &str) -> bool {
    match val.parse::<u8>() {
        Ok(v) if (1..=16).contains(&v) => cfg.seeds = v,
        _ => return false,
    }
    true
}

fn set_init(cfg: &mut ForceConfig, val: &str) -> bool {
    cfg.init = match val {
        "rand" => InitMode::Rand,
        "force1d" => InitMode::Force1d,
        _ => return false,
    };
    true
}

/// Parse (and validate) a `force` spec into a [`ForceConfig`](crate::decompose::ForceConfig)
/// — the SINGLE place the axis grammar is read, reached from [`parse_vtree_spec`]
/// and therefore from both the validator and the builder.
///
/// Grammar:
/// `force[:mst|:cut][/root=merge|balance|hybrid][/orient=x|small|big][/w=euclid|co]`
/// `[/cw=uniform|short][/d=2|3|4][/fb=0..8][/seeds=1..16][/init=rand|force1d]`.
///
/// `root=`, `orient=`, `w=` and `fb=` reshape the MST, so they are MST-mode-only.
/// `cw=`, `d=`, `seeds=` and `init=` apply to both tree-ifiers.
fn parse_force_config(
    spec: &str,
    param: Option<&str>,
    suffixes: &[&str],
) -> Result<ForceConfig, VitriError> {
    // The axis keys, in the order the messages list them, from the one table
    // this parser accepts.
    let keys = one_of(force_axis_names());

    let mode = match param {
        None | Some("mst") => ForceMode::Mst,
        Some("cut") => ForceMode::Cut,
        Some(p) => {
            return Err(invalid_token(
                spec,
                "force tree-ifier",
                &format!(":{p}"),
                &one_of(["mst", "cut"]),
            ));
        }
    };
    let mut cfg = ForceConfig::new(mode);
    // One flag per axis, indexed by its position in the table.
    let mut seen = [false; FORCE_AXES.len()];
    for s in suffixes {
        let (key, val) = s.split_once('=').ok_or_else(|| {
            VitriError::spec(
                spec,
                format!("force suffix \"/{s}\" must be key=value, with key {keys}"),
            )
        })?;
        let Some(i) = FORCE_AXES.iter().position(|a| a.key == key) else {
            return Err(invalid_token(
                spec,
                "force suffix key",
                &format!("/{key}="),
                &keys,
            ));
        };
        let axis = &FORCE_AXES[i];
        // Repetition is reported before the tree-ifier, so `force:cut` with the
        // same MST axis twice names the repeat rather than the mode.
        if seen[i] {
            return Err(VitriError::spec(
                spec,
                format!("duplicate force suffix key \"/{key}=\""),
            ));
        }
        seen[i] = true;
        if axis.mst_only && mode != ForceMode::Mst {
            return Err(VitriError::spec(
                spec,
                format!(
                    "force axis \"/{key}=\" reshapes the MST and cannot combine with \":cut\", \
                     which selects the median-cut tree-ifier and has no MST to reshape"
                ),
            ));
        }
        if !(axis.apply)(&mut cfg, val) {
            return Err(invalid_token(
                spec,
                &format!("force {}", axis.what),
                &format!("/{key}={val}"),
                axis.allowed,
            ));
        }
    }
    Ok(cfg)
}

/// Type a `flowcutter-{primal,incidence}` `:param` token: timed
/// `<N>ms[,<N>iters][,<N>patience]` or step-budgeted `<N>[,<iters>]steps`.
/// Numeric fields must parse — a typo'd budget errors instead of silently
/// defaulting.
fn parse_fc_param(spec: &str, base: &str, params: &str) -> Result<SpecParam, VitriError> {
    if params.ends_with("ms") || params.contains("ms,") {
        let ms_end = params.find("ms").unwrap();
        let timeout_ms: i64 = params[..ms_end]
            .parse()
            .map_err(|_| invalid_token(spec, "timeout", &params[..ms_end], "<N>ms"))?;
        let mut iters = FC_DEFAULT_ITERS;
        let mut patience_ms = FC_PATIENCE_MS_PARAMETRIZED;
        // Each timed field may be named once. A second one would overwrite the
        // first, dropping a value the caller typed (the no-silent-no-op rule).
        let mut seen_iters = false;
        let mut seen_patience = false;
        let repeated = |part: &str, field: &str| {
            VitriError::spec(
                spec,
                format!(
                    "the timed-mode suffix \"{part}\" names {field} a second time; the first \
                     value would be silently dropped. Give \",<N>iters\" and \",<N>patience\" \
                     at most once each"
                ),
            )
        };
        for part in params[ms_end + 2..].split(',').filter(|s| !s.is_empty()) {
            if let Some(n) = part.strip_suffix("iters") {
                if seen_iters {
                    return Err(repeated(part, "the iteration cap"));
                }
                seen_iters = true;
                iters = n
                    .parse()
                    .map_err(|_| invalid_token(spec, "iters suffix", part, "<N>iters"))?;
            } else if let Some(n) = part.strip_suffix("patience") {
                if seen_patience {
                    return Err(repeated(part, "the patience window"));
                }
                seen_patience = true;
                patience_ms = n
                    .parse()
                    .map_err(|_| invalid_token(spec, "patience suffix", part, "<N>patience"))?;
            } else {
                return Err(invalid_token(
                    spec,
                    "timed-mode suffix",
                    part,
                    &one_of([",<N>iters", ",<N>patience"]),
                ));
            }
        }
        Ok(SpecParam::FcTimed {
            timeout_ms,
            iters,
            patience_ms,
        })
    } else if let Some(steps_str) = params.strip_suffix("steps") {
        let parts: Vec<&str> = steps_str.split(',').collect();
        // The shape has two fields; a third one names nothing this mode reads,
        // so it is reported rather than dropped (the no-silent-no-op rule).
        if let Some(extra) = parts.get(2) {
            return Err(invalid_token(
                spec,
                "step-budgeted field",
                extra,
                "at most the two fields of \"<N>[,<iters>]steps\"",
            ));
        }
        let steps: i64 = parts[0].parse().map_err(|_| {
            invalid_token(
                spec,
                "step count",
                parts[0],
                "an integer in <N>[,<iters>]steps",
            )
        })?;
        let iters: i32 = match parts.get(1) {
            Some(p1) => p1.parse().map_err(|_| {
                invalid_token(spec, "iters", p1, "an integer in <N>[,<iters>]steps")
            })?,
            None => FC_DEFAULT_STEPS_ITERS,
        };
        Ok(SpecParam::FcSteps { steps, iters })
    } else {
        Err(invalid_token(
            spec,
            &format!("{base} parameter"),
            params,
            &one_of([
                format!("{base}:200ms (timed)"),
                format!("{base}:200ms,50iters (timed with an iteration cap)"),
                format!("{base}:100000,900steps (step-budgeted)"),
            ]),
        ))
    }
}

/// The error for a base no backend builds — the one place its wording lives.
/// Both name lists come from the tables the parser matches against, so a name
/// added to either is offered here without a second edit.
pub(super) fn unknown_vtree_type(spec: &str) -> VitriError {
    let bases: Vec<&str> = VTREE_BASE_NAMES.iter().map(|b| b.name).collect();
    let elimination: Vec<&str> = crate::decompose::elimination_spec_names().collect();
    VitriError::spec(
        spec,
        format!(
            "unknown vtree type, expected {}, or one of the elimination orders {} — each of \
             those also as \"<name>-inc\" on the incidence graph",
            one_of(bases),
            one_of(elimination),
        ),
    )
}
