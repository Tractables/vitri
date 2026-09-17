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

mod params;
mod vocabulary;

use crate::decompose::{
    BINARIZATIONS, FC_BARE_TIMEOUT_MS, FC_DEFAULT_ITERS, FC_DEFAULT_STEPS_ITERS,
    FC_PATIENCE_MS_BARE, FC_PATIENCE_MS_PARAMETRIZED, ForceConfig, ForceMode, PLACES, ROOTS,
    Reading,
};
use crate::error::VitriError;

use params::KeyedParams;
use vocabulary::{
    FORCE_CLAUSE_WEIGHTS, FORCE_INITS, FORCE_MST_ONLY_KEYS, FORCE_ORIENTS, FORCE_ROOTS,
    FORCE_TREEIFIERS, FORCE_WEIGHTS, REFINEMENTS, SPEC_PARAM_KEYS, TIE_BREAKS, VTREE_BASE_NAMES,
    candidate_range, split_vtree_spec,
};

pub use vocabulary::vtree_spec_bases;
pub(crate) use vocabulary::{
    BALANCED_SPEC, VtreeBase, baseline_spec_names, classify_base, decomposition_spec_names,
    spec_has_candidates, standalone_spec_names,
};

/// The `dim=` range, spelled once from the constant the layout enforces.
fn force_dim_range() -> String {
    format!("an integer 2..={}", crate::decompose::FORCE_MAX_DIM)
}

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
pub(crate) fn one_of<T: std::fmt::Display>(names: impl IntoIterator<Item = T>) -> String {
    let names: Vec<String> = names.into_iter().map(|n| n.to_string()).collect();
    match names.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
    }
}

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

/// One `--vtree` parameter as `--help` prints it.
///
/// Rendered from the parameter table this module matches against, so the help
/// text cannot advertise a key the parser does not accept, or a default it does
/// not apply.
#[derive(Clone, PartialEq, Eq)]
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
    /// The goatd schedule: whether it ends in the refinement pass, the RNG
    /// seed its tie-breaking and sampling draw on, and which of its
    /// decompositions to convert.
    Goatd {
        /// Run the refined schedule rather than one unrefined slot.
        refine: bool,
        /// The RNG seed. Absent means seed 0.
        seed: u64,
        /// Which decomposition of the refined schedule becomes the tree: 0 is
        /// the winner, refined; `n` above 0 is the schedule's `n`th runner-up,
        /// unrefined, in its order of width and then total bag size.
        candidate: u32,
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

    /// Which of the goatd schedule's decompositions the spec names; 0 for
    /// the winner and for every other family.
    pub(crate) fn candidate(&self) -> u32 {
        match *self {
            SpecParam::Goatd { candidate, .. } => candidate,
            _ => 0,
        }
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
            let refine = params.enum_value("refine", REFINEMENTS)?.unwrap_or(true);
            let candidate = params.number("candidate", &candidate_range())?.unwrap_or(0);
            if candidate >= crate::decompose::MAX_GOATD_CANDIDATES {
                return Err(invalid_token(
                    spec,
                    "candidate",
                    &candidate.to_string(),
                    &candidate_range(),
                ));
            }
            // The runners-up are the refined schedule's; the unrefined slot
            // has one decomposition and no list to index.
            if candidate > 0 && !refine {
                return Err(VitriError::spec(
                    spec,
                    "\"candidate=\" names a runner-up of the refined schedule and has \
                     nothing to name under \"refine=off\"",
                ));
            }
            SpecParam::Goatd {
                refine,
                seed: params.number("seed", "an integer")?.unwrap_or(0),
                candidate,
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
/// [`vtree_spec_bases`](super::vtree_spec_bases) is the vocabulary of base
/// names and [`spec_param_docs`] the per-base parameter list.
///
/// # Errors
///
/// [`VitriError::Spec`] naming the offending parameter and the accepting spec
/// form, or naming an unrecognized base and the public construction
/// vocabulary. `Ok(())` only when the base is recognized and every parameter
/// is consumed by its family.
pub fn validate_vtree_spec(spec: &str) -> Result<(), VitriError> {
    let parsed = parse_vtree_spec(spec)?;
    if matches!(parsed.family, VtreeBase::Unknown) {
        return Err(unknown_vtree_type(spec));
    }
    Ok(())
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
