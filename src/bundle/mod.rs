//! Export bundle: the reduced formula, the arithmetic that lifts a count over it
//! back to the original, and the vtree — the artifacts a third-party knowledge
//! compiler needs in order to compile this instance.
//!
//! # Five modes, one identity
//!
//! The bundle supports the four MCC counting problems plus [`Mode::Compile`],
//! and every one of them lifts through the SAME equation:
//!
//! ```text
//! count(original) == count(reduced) × 2^count_lift_pow2 × weight_lift
//! ```
//!
//! where `count` means the mode's own count (plain, weighted, projected,
//! projected-weighted). The two factors are disjoint by construction: an
//! unweighted mode puts everything in the power of two and leaves `weight_lift`
//! at `1`, a weighted mode puts everything in the exact rational `weight_lift`
//! and leaves the exponent at `0`. A consumer applies both, unconditionally, and
//! never has to branch on the mode.
//!
//! # Three chains, because one chain cannot serve all five
//!
//! - **count-preserving** (`mc`, `wmc`) — this crate's own `simplify` chain,
//!   whose stages `docs/preprocessing.md` lists in order, then **Arjun** on
//!   what it produced. Under `wmc` the same
//!   stages run with the unequal-weight variables FROZEN out of DVE, and every
//!   factor the integer chain writes as `×2` or `×1` becomes an exact rational
//!   (the `weighted_lift` policy).
//! - **projection-preserving** (`pmc`, `pwmc`) — Arjun's projection-set
//!   minimization, then show-frozen strengthening and projected BVE
//!   (the `projected` stages). The count-preserving stages do not
//!   preserve a projected count and do not run.
//! - **function-preserving** (`compile`) — only the stages whose effect the
//!   record reconstructs: forced-literal propagation and free-variable removal.
//!   Weaker than either counting chain, and the only chain whose output recovers
//!   the original function rather than just its count.
//!
//! The mode itself is [`RunConfig::mode`], defaulting to detection from the
//! CNF's own `c t` / `c p show` / `c p weight` headers.
//!
//! # Numbering conventions
//!
//! Every emitted file is 1-based DIMACS, matching `reduced.cnf` itself — there
//! is exactly one convention in play:
//! - **`preprocess.json`** — 1-based DIMACS throughout. Every field name
//!   carrying variable ids ends in `_dimacs`.
//! - **`reduced.cnf`** — self-describing: it carries its own `c t <track>`
//!   header, its own `c p show` line (reduced ids) and its own `c p weight` lines
//!   (reduced ids, exact rationals), so a consumer that only reads the CNF still
//!   solves the right problem. Under `compile` there is no `c t` line, since
//!   `compile` is not a track a `c t` line can name; the mode is in
//!   `preprocess.json`.
//! - **`vtree.vtree`** — 1-based, the standard SDD library's text format,
//!   numbering the same variables as `reduced.cnf`.
//! - **`components.json`** and everything under `components/` — a second
//!   space, per component and 1-based, on top of the one above. [`components`]
//!   states the whole rule; the short version is that a component's own files
//!   are in its local numbering, not the reduced one, and
//!   `local_to_reduced_dimacs` is the way back.

pub mod components;

use std::path::{Path, PathBuf};

use num_rational::BigRational;
use serde::{Deserialize, Serialize};

use crate::component::VtreeBuild;
use crate::vtree::VarId;

use crate::cnf::{
    Clause, CnfFormula, CnfMeta, Literal, Mode, Original, Reduced, ShowSet, Weights,
    rational_string,
};
use crate::config::{Chain, RunConfig};
use crate::diagnostics::diag;
use crate::error::VitriError;
use crate::preprocess::arjun::{
    ArjunKeep, ArjunProjResult, ArjunResult, ArjunWeightedProjResult, ArjunWeightedResult,
    arjun_keep_reduction, run_arjun_anytime, run_arjun_projected_anytime,
    run_arjun_weighted_anytime, run_arjun_weighted_projected_anytime,
};
use crate::preprocess::projected::{ProjectedReduction, strengthen_and_bve};
use crate::preprocess::simplify::{
    DveBudget, OriginalFate, SimplifiedFormula, SimplifyConfig, SimplifyPurpose, simplify,
};
use crate::preprocess::weighted_lift::{self, DveVerdict};
use crate::preprocess::{OriginalMap, VarMap};

mod compile_chain;
mod count_chain;
mod plumbing;
mod projection_chain;
mod stage;
use compile_chain::compile_preserving_bundle;
use count_chain::count_preserving_bundle_with_stage1;
// What the three chains and the component writer reach for, named rather than
// globbed: this list is `plumbing`'s reach into the rest of the crate, so an
// item added there is shared deliberately instead of by being written down.
use plumbing::{
    DotFor, ensure_dir, original_weights, preprocess_config, refuted, to_json_pretty, weight_table,
    write_file, write_vtree_files,
};
use projection_chain::projection_preserving_bundle;

/// File names inside an output bundle directory. Named so the CLI, the README
/// and the tests cannot drift apart on what gets written where.
pub const REDUCED_CNF_NAME: &str = "reduced.cnf";

/// The lift record — 1-based DIMACS ids throughout, every var-id field
/// suffixed `_dimacs` (see "Numbering conventions" above).
pub const PREPROCESS_RECORD_NAME: &str = "preprocess.json";

/// The vtree, in the standard SDD library's 1-based text format, numbering
/// the same variables as `reduced.cnf`.
pub const VTREE_NAME: &str = "vtree.vtree";

/// One literal's weight, as written by a `c p weight <lit> <w> 0` line.
///
/// Defined beside the weight table it is a row of; re-exported here because a
/// consumer meets it as a [`PreprocessRecord`] field.
pub use crate::cnf::weights::LiteralWeight;

/// `#[serde(with = ...)]` for [`PreprocessRecord::mode`]: the mode is written as
/// the token [`Mode::token`] names it by, the same token `reduced.cnf` carries
/// on its `c t` line, so the two files cannot spell one mode two ways.
mod mode_token {
    use crate::cnf::Mode;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    /// Write the mode as its token.
    pub(super) fn serialize<S: Serializer>(mode: &Mode, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(mode.token())
    }

    /// Read it back through [`Mode::parse_mode`].
    ///
    /// # Errors
    ///
    /// The format's own error, naming the token, when it is not one this
    /// version of the crate has a mode for.
    pub(super) fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Mode, D::Error> {
        let token = String::deserialize(de)?;
        Mode::parse_mode(&token).ok_or_else(|| D::Error::custom(format!("unknown mode {token:?}")))
    }
}

/// The count-lift record: everything needed to turn a count over `reduced.cnf`
/// back into a count over the original CNF, plus the variable correspondence
/// between the two files.
///
/// **All variable ids and literals in this record are 1-based DIMACS**, matching
/// the `.cnf` files it accompanies and the `vtree.vtree` sibling's own
/// numbering.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreprocessRecord {
    /// Format tag; bump when the on-disk contract changes.
    pub format: String,
    /// What preprocessing preserved — the resolved mode ([`RunConfig::mode`],
    /// defaulting to detection from the input's headers). `reduced.cnf` carries
    /// the same token on its own `c t` line, except under `compile`, which no
    /// `c t` line can name.
    #[serde(with = "mode_token")]
    pub mode: Mode,

    /// The cardinality half of the lift:
    /// `count(original) == count(reduced) × 2^count_lift_pow2 × weight_lift`.
    ///
    /// Assembled by the same `SimplifiedFormula::count_lift` composition a
    /// consumer lifting a count applies, plus Arjun's own exponent. Variables
    /// that are *determined* rather than free (backbone, equivalences,
    /// DVE-defined, Arjun-eliminated-because-defined) contribute a factor of 1
    /// and appear in neither term.
    ///
    /// **Always 0 under a weighted mode**, where a cardinality factor is
    /// meaningless: an eliminated variable's contribution is `w⁻ + w⁺`, not 2, so
    /// the whole lift lives in `weight_lift` instead.
    pub count_lift_pow2: u32,

    /// The weighted half of the lift, exact, as `"numerator/denominator"` in
    /// lowest terms — `"1/1"` under an unweighted mode, where the whole lift is
    /// the power of two above.
    ///
    /// A single scalar covering every weighted factor the chain owes: the forced
    /// literals' own weights, `(w⁻ + w⁺)` per free variable, `w⁺` per
    /// equal-weight defined variable, and Arjun's rational multiplier. Factors
    /// that are not scalars — an equivalence-eliminated variable's weights — fold
    /// into the surviving representative and show up in
    /// [`Self::reduced_weights`] instead.
    pub weight_lift: String,

    /// `p cnf` variable count of the ORIGINAL input.
    pub original_num_vars: u32,

    /// The reduced→original variable correspondence, one entry per reduced
    /// variable (`reduced_to_original_dimacs[r - 1]` describes reduced variable
    /// `r`; one entry per variable of `reduced.cnf`).
    ///
    /// Source space is `reduced.cnf`, target space is the original CNF, and
    /// [`VarMap`] states the encoding: a **signed** 1-based DIMACS literal per
    /// reduced variable, `null` where preprocessing introduced one. Serializes as
    /// that bare array.
    ///
    /// Its length is `reduced.cnf`'s own `p cnf` variable count, so the reduced
    /// formula's size is read off the CNF or off this array rather than restated
    /// here.
    pub reduced_to_original_dimacs: VarMap<Reduced, Original>,

    /// The original→reduced correspondence, one entry per ORIGINAL variable
    /// (`original_to_reduced_dimacs[o - 1]` describes original variable `o`);
    /// present under `compile`, absent otherwise.
    ///
    /// The direction that survives elimination. The map above is indexed by the
    /// reduced formula's variables, so it can only name what survived; this one
    /// is indexed by the original's, so it also names each variable the
    /// preprocessing dropped — a backbone literal as the constant it was fixed to, a
    /// free variable as `null`, and an equivalence partner as the signed literal
    /// of the reduced variable its representative became. [`OriginalMap`] states
    /// the per-entry encoding; entries are fully resolved, so nothing here is a
    /// link to follow.
    ///
    /// This is what makes `compile` reconstructible: assign the reduced model,
    /// then read every original variable off it in one lookup. The counting
    /// modes do not write it — each of them eliminates variables whose value a
    /// reduced model does not determine, so no total map over the original
    /// variables exists to write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_to_reduced_dimacs: Option<OriginalMap>,

    /// Forced (backbone) literals as signed 1-based DIMACS literals over the
    /// ORIGINAL variable space: each is fixed to that polarity in every model, so
    /// each contributes its own factor to the lift and none is left for the
    /// consumer to account for. A consumer that wants full models rather than a
    /// count re-attaches these.
    ///
    /// Populated by the count-preserving and function-preserving chains (this
    /// crate's stripping stage plus the backbone Arjun proved). The
    /// projection-preserving chain records none: its stages report eliminations,
    /// not forced polarities.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forced_literals_original_dimacs: Vec<i32>,

    /// Variables this crate's stages removed as free — zero occurrences left
    /// after the simplify chain, or free at DVE — as original 1-based DIMACS
    /// ids. Listed separately from the exponent so a consumer enumerating models
    /// knows which variables are free rather than merely absent.
    ///
    /// Under an unweighted mode each contributes a factor of 2 and is already
    /// summed into `count_lift_pow2`, which also carries Arjun's own aggregate
    /// exponent — Arjun reports a number, not a list of variables, so
    /// `count_lift_pow2` can exceed this list's length. Under a weighted mode
    /// each contributes `(w⁻ + w⁺)` inside `weight_lift` instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub free_vars_original_dimacs: Vec<u32>,

    /// True when preprocessing already proved the instance unsatisfiable. The
    /// count is 0 and no compilation is needed.
    ///
    /// `reduced.cnf` then holds an explicit contradiction (`x` and `¬x` over the
    /// original variable count) with the identity variable map, not the empty
    /// clause: DIMACS has no portable spelling of the empty clause — a lone `0`
    /// line is read back as a stray terminator by most parsers, which would
    /// silently turn UNSAT into a nonzero count. The contradiction is
    /// count-equivalent (0 either way) and unambiguous.
    pub unsat: bool,

    /// The show set, when the mode is projected; absent otherwise. Under
    /// `compile` it is the input's own declared show set, renumbered but not
    /// otherwise touched.
    ///
    /// **This is the set the reduced count must be taken over, and it is not the
    /// input's set**: Arjun's projection-set minimization REWRITES the show set
    /// (dropping variables that are free or determined given the others) and
    /// renumbers it, and the strengthening stage drops any show variable it
    /// proved equivalent to another counted one. Counting `reduced.cnf` over the
    /// input's own show ids would be a silent miscount, which is why the same set
    /// is also written as a `c p show` line INSIDE `reduced.cnf` — read it from
    /// either, never re-derive it from the input.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::cnf::show_set::dimacs"
    )]
    pub show_vars_reduced_dimacs: Option<ShowSet<Reduced>>,

    /// The literal weights the reduced count must be taken under, in REDUCED
    /// numbering, when the mode is weighted; absent otherwise. Also written as
    /// `c p weight` lines inside `reduced.cnf`.
    ///
    /// Not the input's weights: an equivalence-eliminated variable's weights are
    /// folded into its surviving representative (with the polarities swapped when
    /// the two are anti-equivalent), so a survivor's weight here is the product
    /// over its whole class. Every literal of the reduced formula is listed —
    /// including the weight-1 ones — so a consumer never has to know the default.
    /// Under `compile` these are the input's own weights, renumbered but not
    /// folded into the lift.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduced_weights: Option<Vec<LiteralWeight>>,
}

/// Which half of the lift carries it, before the record splits it into
/// [`PreprocessRecord::count_lift_pow2`] and [`PreprocessRecord::weight_lift`].
///
/// The two fields are disjoint by construction, and this is where that is
/// decided: naming one half names the other as neutral, so a chain building a
/// record says which mode it is in once instead of writing the neutral value
/// out by hand.
enum RecordLift {
    /// An unweighted mode: the whole lift is `2^k`.
    Pow2(u32),
    /// A weighted mode: the whole lift is this exact rational.
    Weight(BigRational),
}

impl RecordLift {
    /// The `weight_lift` of a record whose lift is the CARDINAL half — the
    /// rational `1`, as [`rational_string`] writes it.
    ///
    /// It is the marker as much as the value. The halves are disjoint, and a
    /// weighted lift of exactly `1` is the neutral one, so a record carrying
    /// this text is a record whose weighted half stands idle — which is how
    /// [`PreprocessRecord::lift`] reads back what a chain wrote here.
    const NEUTRAL_WEIGHT: &'static str = "1/1";

    /// The lift that changes nothing — `2^0`, which is also the rational `1`,
    /// so it serialises the same under either mode.
    fn neutral() -> Self {
        RecordLift::Pow2(0)
    }

    /// `(count_lift_pow2, weight_lift)`, with the half this lift is not
    /// filled in neutral.
    fn into_fields(self) -> (u32, String) {
        match self {
            RecordLift::Pow2(k) => (k, RecordLift::NEUTRAL_WEIGHT.to_string()),
            RecordLift::Weight(w) => (0, rational_string(&w)),
        }
    }
}

/// Format tag written into every [`PreprocessRecord`]. Bumped whenever a field is
/// added, removed, or changes meaning; a consumer should refuse a tag it does not
/// know.
pub const RECORD_FORMAT_TAG: &str = "vitri-preprocess-v1";

impl PreprocessRecord {
    /// The record a finished chain starts from: the four things every chain
    /// settles for itself, with every other field at the value that says "this
    /// chain has nothing to report there".
    ///
    /// A chain fills in what it does have with struct-update syntax, so its own
    /// record names exactly what it decided, and the neutral values — no total
    /// map over the original variables, no forced literals, no free variables,
    /// no show set, no weight table — are spelled once here instead of at each
    /// chain's tail. `unsat` is among them: a refuted instance never reaches a
    /// chain's own record, because [`refuted`] answers first.
    fn new(
        mode: Mode,
        lift: RecordLift,
        original_num_vars: u32,
        reduced_to_original_dimacs: VarMap<Reduced, Original>,
    ) -> Self {
        let (count_lift_pow2, weight_lift) = lift.into_fields();
        PreprocessRecord {
            format: RECORD_FORMAT_TAG.to_string(),
            mode,
            count_lift_pow2,
            weight_lift,
            original_num_vars,
            reduced_to_original_dimacs,
            original_to_reduced_dimacs: None,
            forced_literals_original_dimacs: Vec::new(),
            free_vars_original_dimacs: Vec::new(),
            unsat: false,
            show_vars_reduced_dimacs: None,
            reduced_weights: None,
        }
    }

    /// The lift as one factor: `count(original) = count(reduced) * lift`.
    ///
    /// Both halves of the split are always present in the record, and exactly
    /// one of them is ever live — a weighted mode records no cardinality lift
    /// at all, since each free variable contributes a rational instead, and
    /// every unweighted mode, `compile` included, records the identity
    /// rational. Which half that is is a property of the record rather than of
    /// the caller's idea of the mode, so it is answered here, beside the writer
    /// that fills the two fields in the first place.
    pub fn lift(&self) -> String {
        if self.weight_lift == RecordLift::NEUTRAL_WEIGHT {
            format!("2^{}", self.count_lift_pow2)
        } else {
            self.weight_lift.clone()
        }
    }
}

/// What one preprocessing stage did.
///
/// Reported so a caller can act on the difference rather than read it in a log.
/// The distinction that carries the most weight is [`GaveUp`](Self::GaveUp)
/// against [`Discarded`](Self::Discarded): a stage that ran out of budget may
/// well produce something on a different budget, while a stage whose result was
/// rejected produces the same rejection again. A caller deciding whether to
/// preprocess the same formula a second time is deciding between those two.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StageOutcome {
    /// The stage ran and its result was kept.
    Ran,
    /// The stage did not run at all.
    Skipped(SkipReason),
    /// The stage produced nothing to keep inside the budget it was given.
    ///
    /// Not a failure — it is the anytime contract working. A budgeted stage
    /// stops STARTING work at its deadline and hands back the soundest
    /// checkpoint it has reached; this covers both a stage that had reached
    /// none and one whose checkpoint came back so late that it was dropped as
    /// bought with budget the caller no longer has. Either way a caller with
    /// more wall can call again and expect a different answer.
    ///
    /// How late is too late is per mode, because the modes do not lose the same
    /// thing by being strict: `mc` and `wmc` run the reduction in a forked child
    /// killed shortly after the deadline and discard a late result (`mc` keeps
    /// it if `VITRI_ARJUN_KEEP_OVERRUN` asks, which also runs it in this
    /// process), while `pmc` and `pwmc` keep their checkpoint however late,
    /// Arjun being their first stage and the rest of their chain cheap.
    GaveUp,
    /// The stage produced a result and it was then rejected.
    ///
    /// Calling again on the same formula with the same configuration produces
    /// the same rejection.
    Discarded(DiscardReason),
}

/// Why a stage did not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SkipReason {
    /// The configuration did not ask for it — see
    /// [`PreprocessStages`](crate::config::PreprocessStages), or
    /// [`ArjunOptions::sbva`](crate::preprocess::ArjunOptions::sbva) for
    /// bounded variable addition.
    NotRequested,
    /// There was nothing left for it to work on.
    NothingToDo,
}

/// Why a stage's result was rejected after it had been produced.
///
/// Every one of these is a property of the formula and the configuration, not
/// of the wall clock: the same call makes the same judgement again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiscardReason {
    /// The reduction grew the clause count. Fewer variables does not make a
    /// larger formula the better one to hand a compiler.
    NotSmaller,
    /// A projection-preserving reduction left the projection exactly as it
    /// found it, so it bought no counting benefit and can compile worse than
    /// the formula it was given.
    NoProjectionGain,
    /// A weighted reduction that dropped weighted mass its multiplier does not
    /// carry, or left the formula unchanged.
    WeightedUnusable,
    /// The variable map the reduction reported was not injective, so counts
    /// taken over the reduced formula could not be lifted back soundly. A
    /// refusal on correctness grounds rather than on quality.
    NonInjectiveMap,
}

impl DiscardReason {
    /// The phrase this reason is reported by, so what a caller reads and what
    /// the diagnostic line says are one string.
    pub(super) fn phrase(self) -> &'static str {
        match self {
            DiscardReason::NotSmaller => "it grew the clause count",
            DiscardReason::NoProjectionGain => "it did not minimize the projection",
            DiscardReason::WeightedUnusable => "lossy or inert",
            DiscardReason::NonInjectiveMap => "non-injective variable map",
        }
    }
}

/// What each stage of the chain did. See [`StageOutcome`].
///
/// A `None` field is a stage this mode's chain does not have — `compile` runs
/// no Arjun, and bounded variable addition has nothing to report when the
/// reduction around it never ran.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StageReport {
    /// This crate's own simplify chain.
    pub simplify: Option<StageOutcome>,
    /// The Arjun reduction.
    pub arjun: Option<StageOutcome>,
    /// Bounded variable addition, as part of the Arjun reduction.
    pub sbva: Option<StageOutcome>,
}

/// The cardinality lift, attributed to the stage that earned it.
///
/// A caller lifting one final count wants [`total_pow2`](Self::total_pow2) and
/// can ignore the split. A caller that re-reduces a formula *derived* from the
/// one this run reduced — a cofactor, a component, a conditioned branch — needs
/// the split: the reduction it is about to run applies to the formula the Arjun
/// stage was given ([`PreprocessBundle::arjun_input`]), so only the Arjun
/// stage's own exponent is the one to reconcile against. Folding in the simplify
/// chain's share would count it once per derived formula instead of once for the
/// run.
///
/// Both halves are zero under a weighted mode, where each eliminated variable
/// contributes an exact rational rather than a factor of two and the whole lift
/// is [`PreprocessRecord::weight_lift`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CountLift {
    /// Earned by this crate's own simplify chain.
    pub simplify_pow2: u32,
    /// Earned by the Arjun reduction.
    pub arjun_pow2: u32,
}

impl CountLift {
    /// The whole exponent: `count(original) == count(reduced) × 2^total_pow2`,
    /// which is [`PreprocessRecord::count_lift_pow2`].
    pub fn total_pow2(self) -> u32 {
        self.simplify_pow2 + self.arjun_pow2
    }
}

/// Wall-clock and probing telemetry from one preprocessing call.
///
/// A phase duration is `None` when that phase was not attempted. `Some(0)`
/// means it was attempted and completed in less than one millisecond. These
/// measurements describe work performed by the call, including a reduction
/// whose result was later discarded; [`PreprocessBundle::stages`] describes
/// the outcome of the stage instead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PreprocessTelemetry {
    /// Total wall time spent in preprocessing, including all enabled phases.
    pub total_ms: u64,
    /// The crate's own simplify chain, when that stage was attempted.
    pub simplify_ms: Option<u64>,
    /// SAT backbone probing inside the simplify chain, when attempted.
    pub backbone_ms: Option<u64>,
    /// SAT equivalence probing inside the simplify chain, when attempted.
    pub equivalence_ms: Option<u64>,
    /// Definability elimination inside the simplify chain, when attempted.
    pub dve_ms: Option<u64>,
    /// Arjun's opaque native reduction call, including SBVA when it participated.
    pub arjun_ms: Option<u64>,
    /// Backbone literals proved by the probing phase.
    pub backbone_found: usize,
    /// Backbone probes completed by the probing phase.
    pub backbone_probes: usize,
}

impl PreprocessTelemetry {
    /// Publish simplify's private measurements under the public stage-presence
    /// contract. The identity simplify call used for a disabled stage is not an
    /// attempted phase, even though it shares the same internal code path.
    fn from_simplified(simplified: &SimplifiedFormula, attempted: bool) -> Self {
        let measured = simplified.telemetry;
        PreprocessTelemetry {
            simplify_ms: attempted.then_some(measured.total_ms),
            backbone_ms: measured.backbone_ms,
            equivalence_ms: measured.equivalence_ms,
            dve_ms: measured.dve_ms,
            backbone_found: measured.backbone_found,
            backbone_probes: measured.backbone_probes,
            ..PreprocessTelemetry::default()
        }
    }
}

/// A reduced formula paired with the record that lifts counts over it back to
/// the original — the two halves that must always travel together.
#[derive(Clone, Debug)]
pub struct PreprocessBundle {
    /// The reduced formula, in preprocessing's own (post-elimination)
    /// variable numbering — the CNF a third-party compiler actually compiles.
    pub reduced: CnfFormula,
    /// The lift and provenance: the variable maps back to the original
    /// numbering and the count-lift factors, in the form that is written to
    /// disk. What each stage DID is [`Self::stages`], which is about this call
    /// rather than about lifting a count and so is not part of the record.
    pub record: PreprocessRecord,
    /// What each stage of the chain did — ran, was skipped, gave up, or had its
    /// result discarded. See [`StageReport`].
    ///
    /// The reason it is a returned value rather than a log line: a caller that
    /// preprocesses a formula again on a bigger budget needs to know whether
    /// the first attempt ran out of time or was refused on quality, and only
    /// one of those is worth retrying.
    pub stages: StageReport,
    /// The cardinality lift, split across the stages that earned it. See
    /// [`CountLift`]; the total is [`PreprocessRecord::count_lift_pow2`].
    pub count_lift: CountLift,
    /// Measurements and probing counts from the work this call attempted.
    pub telemetry: PreprocessTelemetry,
    /// The formula the Arjun stage was given, retained when the caller asked
    /// for it.
    ///
    /// The chain is `input → simplify → this → arjun → reduced`. A caller that
    /// re-reduces a formula DERIVED from this one — conditioning it, splitting
    /// it, taking a cofactor — starts from this rather than from the input,
    /// because the simplify chain's eliminations are already banked into
    /// [`CountLift::simplify_pow2`] and earning them again would count them
    /// twice.
    ///
    /// `None` unless
    /// [`RunConfig::retain_arjun_input`](crate::config::RunConfig::retain_arjun_input)
    /// asked for it: it is a second whole formula held in memory, which a
    /// caller that does not need it should not pay for. `None` too when the
    /// mode has no Arjun stage.
    pub arjun_input: Option<CnfFormula>,
    /// Arjun's independent support for the exported plain-MC reduction, in
    /// [`Self::reduced`]'s 0-based variable space.
    ///
    /// `Some`, including `Some(empty)`, only when the plain unweighted Arjun
    /// result is the formula this bundle exports. `None` for every other mode
    /// and whenever that stage was skipped, gave up, or was discarded. This is
    /// an in-process hint, not projection metadata: it is not written to
    /// `reduced.cnf` or `preprocess.json` and is never mapped back through the
    /// record, because SBVA may have introduced variables with no original id.
    pub independent_support_reduced: Option<crate::cnf::ShowSet<crate::cnf::Reduced>>,
    /// Redundant clauses Arjun's internal solver derived while preprocessing — each
    /// one implied by [`Self::reduced`], so a consumer can hand them to its own
    /// solver as a head start without changing what the instance means.
    ///
    /// **1-based DIMACS in `reduced`'s own variable space**, the same numbering
    /// as `reduced.cnf` and `vtree.vtree` — not the original CNF's. A clause
    /// mentioning a variable preprocessing eliminated is dropped rather than
    /// mapped, so every literal here names a variable of `reduced`.
    ///
    /// Empty unless [`ArjunOptions::export_learned_clauses`](crate::preprocess::ArjunOptions::export_learned_clauses)
    /// asked for the harvest, and empty then too when the Arjun stage produced
    /// nothing to keep. Not part of [`Self::write_to_dir`]'s output: the
    /// clauses are a hint for the process holding this value, and a consumer
    /// that wants them on disk writes them itself.
    pub learnt_clauses_reduced_dimacs: Vec<Vec<i32>>,
}

/// Paths written by [`PreprocessBundle::write_to_dir`].
#[derive(Debug)]
pub struct BundlePaths {
    /// Where `reduced.cnf` ([`REDUCED_CNF_NAME`]) landed inside the target directory.
    pub reduced_cnf: PathBuf,
    /// Where `preprocess.json` ([`PREPROCESS_RECORD_NAME`]) landed inside the target directory.
    pub record: PathBuf,
}

/// Run the crate's preprocessing stages on `formula` and package the result for
/// export.
///
/// `meta` is the header metadata from [`CnfFormula::from_dimacs`]; it supplies
/// the show set and the weights, and — with `RunConfig::mode` unset — decides
/// which of the four counting problems preprocessing preserves.
/// `RunConfig::default()` resolves to every stage on, no budget, mode detected
/// from the headers.
///
/// The config decides the mode ([`RunConfig::mode`]), which stages run
/// ([`crate::config::PreprocessStages`]), and the budget they share
/// ([`RunConfig::budget_ms`] / [`RunConfig::deadline`], anchored once — here
/// when this is the whole call, in [`run`] when it is not). Stage order,
/// per-stage budget defaults, and the Arjun keep-or-discard gates are fixed,
/// not configurable per call.
///
/// Each header declaration the resolved mode does not use
/// ([`crate::config::ResolvedMode::notices`]) is reported through
/// [`crate::diagnostics`], which is quiet unless the caller opted in.
///
/// # Errors
///
/// [`VitriError::Input`] for a formula with no variables, which no bundle can
/// describe: the reduced formula would have nothing in it, and the record's
/// lift would be the whole answer with no file to apply it to.
/// [`VitriError::Config`] when an explicit [`RunConfig::mode`] needs data the
/// instance does not carry (see [`RunConfig::resolve_mode`]) or when
/// [`ArjunOptions::export_learned_clauses`](crate::preprocess::ArjunOptions::export_learned_clauses)
/// asks a run whose stages cannot
/// harvest, and [`VitriError::Env`] for a `VITRI_*` variable preprocessing
/// reads. Otherwise preprocessing always produces a bundle: a stage that finds
/// nothing to do, or runs out of budget, weakens the result rather than
/// failing.
pub fn preprocess(
    formula: &CnfFormula,
    meta: &CnfMeta,
    config: &RunConfig,
) -> Result<PreprocessBundle, VitriError> {
    config.validate()?;
    // Called on its own, this call IS the run, so it starts the clock. Reached
    // through [`run`], the clock is already running and the anchored config
    // arrives below.
    preprocess_anchored(formula, meta, &config.anchored(std::time::Instant::now()))
}

/// [`preprocess`] on a config that has been validated and whose budget is
/// already anchored ([`RunConfig::anchored`]) — the body of the public entry,
/// and what [`run`] calls so that the two halves of a run divide one budget.
fn preprocess_anchored(
    formula: &CnfFormula,
    meta: &CnfMeta,
    config: &RunConfig,
) -> Result<PreprocessBundle, VitriError> {
    preprocess_anchored_with_checkpoint(formula, meta, config).map(|outcome| outcome.bundle)
}

/// The single anchored preprocessing result, optionally carrying the owned
/// count-stage checkpoint a frontend session may reuse for a later attempt.
struct PreprocessOutcome {
    bundle: PreprocessBundle,
    count_stage1: Option<count_chain::CountStage1>,
}

fn preprocess_anchored_with_checkpoint(
    formula: &CnfFormula,
    meta: &CnfMeta,
    config: &RunConfig,
) -> Result<PreprocessOutcome, VitriError> {
    let started = std::time::Instant::now();
    if formula.num_vars == 0 {
        return Err(VitriError::input(
            "the formula declares no variables — nothing to build a vtree over",
        ));
    }
    let resolved = config.resolve_mode(meta)?;
    let mode = resolved.mode;
    // Before the notices, because these refusals are the caller's own — asking
    // for something the mode has no stage to answer is settled before the run
    // says anything about the instance.
    config.refuse_inert(mode)?;
    for n in &resolved.notices {
        diag!("{n}");
    }
    let (mut bundle, count_stage1) = match Chain::for_mode(mode) {
        Chain::Compile => (compile_preserving_bundle(formula, meta, config), None),
        Chain::Projection => (
            projection_preserving_bundle(formula, meta, config, mode)?,
            None,
        ),
        Chain::Count => {
            let (bundle, stage1) =
                count_preserving_bundle_with_stage1(formula, meta, config, mode)?;
            (bundle, Some(stage1))
        }
    };
    bundle.telemetry.total_ms = started.elapsed().as_millis() as u64;
    Ok(PreprocessOutcome {
        bundle,
        count_stage1,
    })
}

/// The wall assigned by an embedding compiler to one preprocessing retry.
///
/// Vitri owns what the retry does; the compiler owns how much of its compile
/// cascade the retry may spend. The absolute deadline bounds preprocessing,
/// vtree construction, and the caller's later compile of the returned run.
/// `arjun_budget` is the exact part of that window Arjun may consume, clamped
/// by `deadline`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryBudget {
    deadline: std::time::Instant,
    arjun_budget: std::time::Duration,
}

impl RetryBudget {
    /// Create a non-empty retry budget ending at `deadline`.
    ///
    /// # Errors
    ///
    /// [`VitriError::Config`] when `arjun_budget` is zero. An already-expired
    /// deadline is not a malformed request; the session simply declines the
    /// retry when it is attempted.
    pub fn new(
        deadline: std::time::Instant,
        arjun_budget: std::time::Duration,
    ) -> Result<Self, VitriError> {
        if arjun_budget.is_zero() {
            return Err(VitriError::config(
                "a frontend retry needs a non-zero Arjun budget",
            ));
        }
        Ok(Self {
            deadline,
            arjun_budget,
        })
    }

    /// The absolute deadline shared by the returned vtree and its caller's
    /// compile attempt.
    pub fn deadline(self) -> std::time::Instant {
        self.deadline
    }

    /// The exact Arjun allowance inside [`Self::deadline`].
    pub fn arjun_budget(self) -> std::time::Duration {
        self.arjun_budget
    }
}

/// A validated, anchored full-pipeline run that has not prepared its primary
/// attempt yet.
///
/// The session borrows the raw formula and its metadata, and owns the cloned
/// [`RunConfig`], construction context, and raw
/// [`StructureProfile`](crate::score::StructureProfile) that every attempt over
/// that input must share. Its deadline is anchored when the session is created,
/// so time between [`frontend`] and [`Self::prepare`] remains part of the run
/// budget.
///
/// A session prepares one primary attempt. Calling [`Self::prepare`] again is
/// refused explicitly. After an embedding compiler reports that attempt did
/// not compile, [`Self::retry_without_sbva`] may finish one new Arjun attempt
/// from the exact simplify checkpoint retained by the primary run; it never
/// repeats or clones simplification.
pub struct FrontendSession<'a> {
    formula: &'a CnfFormula,
    meta: &'a CnfMeta,
    config: RunConfig,
    selection: crate::decompose::SelectionCtx,
    source_profile: crate::score::StructureProfile,
    count_stage1: Option<count_chain::CountStage1>,
    prepared: bool,
    retry_attempted: bool,
}

impl std::fmt::Debug for FrontendSession<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrontendSession")
            .field("formula", self.formula)
            .field("meta", self.meta)
            .field("config", &self.config)
            .field("selection", &self.selection)
            .field("source_profile", &self.source_profile)
            .field("has_count_stage1", &self.count_stage1.is_some())
            .field("prepared", &self.prepared)
            .field("retry_attempted", &self.retry_attempted)
            .finish()
    }
}

fn retain_count_stage1(
    source_profile: crate::score::StructureProfile,
    mode: Mode,
    sbva: Option<&StageOutcome>,
) -> bool {
    mode == Mode::Mc && source_profile.coloring_like && sbva == Some(&StageOutcome::Ran)
}

impl FrontendSession<'_> {
    fn build_run(
        &self,
        preprocessed: PreprocessBundle,
        config: &RunConfig,
    ) -> Result<VitriRun, VitriError> {
        if preprocessed.reduced.num_vars == 0 {
            return Ok(VitriRun {
                source_profile: self.source_profile,
                preprocessed,
                vtree: RunVtree::FullyResolved,
            });
        }
        let selection = run_selection(
            &self.selection,
            self.source_profile,
            preprocessed.record.show_vars_reduced_dimacs.as_ref(),
            preprocessed.reduced.num_vars,
        );
        let built =
            crate::component::build_vtree_anchored(&preprocessed.reduced, config, &selection)?;
        Ok(VitriRun {
            source_profile: self.source_profile,
            preprocessed,
            vtree: RunVtree::Built(built),
        })
    }

    /// Preprocess the borrowed input and build the vtree over what remains.
    ///
    /// # Errors
    ///
    /// Whatever [`preprocess`] and
    /// [`component::build_vtree`](crate::component::build_vtree) return. A
    /// second call returns [`VitriError::Config`] instead of repeating work.
    pub fn prepare(&mut self) -> Result<VitriRun, VitriError> {
        if self.prepared {
            return Err(VitriError::config(
                "FrontendSession::prepare may be called at most once",
            ));
        }
        // An attempted preparation consumes the session even when a phase
        // fails: silently replaying preprocessing after an error would be a
        // second attempt with no policy saying that is what the caller wanted.
        self.prepared = true;

        let outcome = preprocess_anchored_with_checkpoint(self.formula, self.meta, &self.config)?;
        let preprocessed = outcome.bundle;
        if retain_count_stage1(
            self.source_profile,
            preprocessed.record.mode,
            preprocessed.stages.sbva.as_ref(),
        ) {
            self.count_stage1 = outcome.count_stage1;
        }
        self.build_run(preprocessed, &self.config)
    }

    /// Re-run Arjun with bounded variable addition disabled after the primary
    /// compile attempt failed, reusing the primary attempt's exact simplify
    /// checkpoint and building a new vtree over the result.
    ///
    /// Returns `Ok(None)` when the primary run did not make this retry useful:
    /// only plain model counting over a coloring-like raw input whose primary
    /// Arjun attempt actually ran SBVA retains the checkpoint. An expired
    /// [`RetryBudget`] also declines without starting work.
    ///
    /// The compiler decides whether a failed compile has enough wall to ask;
    /// Vitri owns every preprocessing and construction operation after that
    /// request. The retry may be attempted at most once.
    ///
    /// # Errors
    ///
    /// [`VitriError::Config`] when called before [`Self::prepare`] or more than
    /// once. Other errors come from Arjun or vtree construction.
    pub fn retry_without_sbva(
        &mut self,
        budget: RetryBudget,
    ) -> Result<Option<VitriRun>, VitriError> {
        if !self.prepared {
            return Err(VitriError::config(
                "FrontendSession::retry_without_sbva requires a completed primary prepare",
            ));
        }
        if self.retry_attempted {
            return Err(VitriError::config(
                "FrontendSession::retry_without_sbva may be called at most once",
            ));
        }
        self.retry_attempted = true;

        let Some(stage1) = self.count_stage1.take() else {
            return Ok(None);
        };
        let now = std::time::Instant::now();
        let deadline = self
            .config
            .deadline
            .map_or(budget.deadline, |run_deadline| {
                run_deadline.min(budget.deadline)
            });
        if deadline <= now {
            return Ok(None);
        }

        let mut retry_config = self.config.clone();
        retry_config.deadline = Some(deadline);
        retry_config.arjun_budget = crate::config::ArjunBudget::Exact(budget.arjun_budget);
        retry_config.arjun.sbva = crate::preprocess::ArjunSbva::Off;
        let preprocessed = count_chain::finish_count_preserving_attempt(&stage1, &retry_config)?;
        self.build_run(preprocessed, &retry_config).map(Some)
    }
}

/// Create a full-pipeline session over one raw input.
///
/// Configuration is validated, the raw structural profile is measured, and a
/// relative [`RunConfig::budget_ms`] is anchored to an absolute deadline before
/// this function returns. [`FrontendSession::prepare`] therefore spends the
/// budget that remains from session creation rather than starting a new one.
///
/// # Errors
///
/// [`VitriError::Config`] for an invalid configuration.
pub fn frontend<'a>(
    formula: &'a CnfFormula,
    meta: &'a CnfMeta,
    config: &RunConfig,
    selection: &crate::decompose::SelectionCtx,
) -> Result<FrontendSession<'a>, VitriError> {
    frontend_at(formula, meta, config, selection, std::time::Instant::now())
}

/// The deterministic clock seam beneath [`frontend`].
fn frontend_at<'a>(
    formula: &'a CnfFormula,
    meta: &'a CnfMeta,
    config: &RunConfig,
    selection: &crate::decompose::SelectionCtx,
    now: std::time::Instant,
) -> Result<FrontendSession<'a>, VitriError> {
    config.validate()?;
    Ok(FrontendSession {
        formula,
        meta,
        config: config.anchored(now),
        selection: selection.clone(),
        source_profile: crate::score::StructureProfile::measure(formula),
        count_stage1: None,
        prepared: false,
        retry_attempted: false,
    })
}

/// What one run of this crate over one instance produced: the preprocessing
/// bundle, and the vtree over what preprocessing left.
///
/// [`run`] and [`FrontendSession::prepare`] build one through the same session
/// path: the two halves are produced in that order, over that formula, and
/// pairing a bundle with a vtree built over anything else is the mistake this
/// type exists to prevent.
#[derive(Debug)]
pub struct VitriRun {
    /// Structural profile of the raw input formula, measured before any
    /// preprocessing changed it.
    ///
    /// The full [`run`] entry point owns this measurement and supplies the same
    /// value to vtree selection. A profile present on the caller's selection
    /// context is deliberately ignored: it cannot override what this run saw.
    pub source_profile: crate::score::StructureProfile,
    /// The reduced formula and its count-lift record.
    pub preprocessed: PreprocessBundle,
    /// The vtree over [`Self::preprocessed`]'s reduced formula, or why there is
    /// none.
    pub vtree: RunVtree,
}

/// The vtree half of a [`VitriRun`].
#[derive(Debug)]
pub enum RunVtree {
    /// The vtree over the reduced formula, and everything construction reported
    /// about it.
    Built(VtreeBuild),
    /// Preprocessing resolved every variable — forced, determined, or folded
    /// into the lift — so `count(reduced)` is 1 by definition, `count(original)`
    /// is the lift itself, and there is nothing left to build a vtree over.
    /// An outcome, not a failure: the record alone is the answer.
    FullyResolved,
}

impl VitriRun {
    /// The vtree construction, when this run built one.
    pub fn built(&self) -> Option<&VtreeBuild> {
        match &self.vtree {
            RunVtree::Built(b) => Some(b),
            RunVtree::FullyResolved => None,
        }
    }

    /// Write every file this run can name into `dir`: the two halves,
    /// [`PreprocessBundle::write_to_dir`] then
    /// [`VtreeBuild::write_to_dir`](crate::component::VtreeBuild::write_to_dir),
    /// in that order.
    ///
    /// A run that built no vtree writes the bundle alone — there is no vtree to
    /// point a manifest at.
    ///
    /// # Errors
    ///
    /// [`VitriError::Io`] naming the file or directory that could not be
    /// written, and [`VitriError::Mismatch`] for a build that does not belong
    /// to this run's formula.
    pub fn write_to_dir(
        &self,
        dir: &Path,
        options: components::ComponentWriteOptions,
    ) -> Result<RunPaths, VitriError> {
        let bundle = self.preprocessed.write_to_dir(dir)?;
        let Some(build) = self.built() else {
            return Ok(RunPaths {
                bundle,
                vtree: None,
            });
        };
        let vtree = build.write_to_dir(
            dir,
            &self.preprocessed.reduced,
            self.preprocessed.record.show_vars_reduced_dimacs.as_ref(),
            options,
        )?;
        Ok(RunPaths {
            bundle,
            vtree: Some(vtree),
        })
    }
}

/// What [`VitriRun::write_to_dir`] wrote.
#[derive(Debug)]
pub struct RunPaths {
    /// `reduced.cnf` and `preprocess.json`, which every run writes.
    pub bundle: BundlePaths,
    /// The vtree files, absent for a run that built no vtree — the same
    /// distinction [`RunVtree`] draws, and drawn once here rather than repeated
    /// across the files that arrive together.
    pub vtree: Option<VtreeFiles>,
}

/// What the vtree half of a run wrote: the vtree itself, its picture, and the
/// component split underneath it.
#[derive(Debug)]
pub struct VtreeFiles {
    /// Where `vtree.vtree` ([`VTREE_NAME`]) landed.
    pub vtree: PathBuf,
    /// Its Graphviz picture, present only when
    /// [`ComponentWriteOptions::dot`](components::ComponentWriteOptions::dot)
    /// asked for one.
    pub dot: Option<PathBuf>,
    /// The component manifest and the files it names, written whatever the
    /// split turned out to be.
    pub components: ComponentFiles,
}

/// The component split as written: what the manifest says, and where its files
/// landed.
#[derive(Debug)]
pub struct ComponentFiles {
    /// The manifest, as written to `components.json`.
    pub manifest: components::ComponentsManifest,
    /// Where the manifest and the files it names landed.
    pub paths: components::ComponentPaths,
}

/// Preprocess `formula` and build the vtree over what preprocessing left — the
/// whole pipeline, in the one order it runs in.
///
/// [`preprocess`] then [`component::build_vtree`](crate::component::build_vtree),
/// with the step between them that a caller would otherwise have to know about:
/// the vtree is built over the REDUCED formula, and selection is made show-aware
/// from the record's show set, which is already in that formula's space.
///
/// `selection` carries the caller's construction knobs; its objective is filled
/// in from the instance, since only the record can say what the reduced show set
/// is. Its `source_profile` field is ignored: this full-pipeline entry measures
/// the raw input exactly once, reports that measurement on [`VitriRun`], and
/// unconditionally supplies the same value to vtree selection.
///
/// The budget is anchored once by [`frontend`], and both halves stop at that
/// instant: what preprocessing spends, construction does not get. `run` creates
/// the session and immediately prepares it.
///
/// # Errors
///
/// Whatever [`preprocess`] and
/// [`component::build_vtree`](crate::component::build_vtree) return.
pub fn run(
    formula: &CnfFormula,
    meta: &CnfMeta,
    config: &RunConfig,
    selection: &crate::decompose::SelectionCtx,
) -> Result<VitriRun, VitriError> {
    frontend(formula, meta, config, selection)?.prepare()
}

/// Resolve the construction context owned by a full [`run`]. Kept as one
/// production path so the internal regression test can prove the value handed
/// to selection is the value the run reports.
fn run_selection(
    selection: &crate::decompose::SelectionCtx,
    source_profile: crate::score::StructureProfile,
    show: Option<&crate::cnf::ShowSet<crate::cnf::Reduced>>,
    num_vars: u32,
) -> crate::decompose::SelectionCtx {
    let mut selection = selection.clone().with_show(show, num_vars);
    selection.source_profile = Some(source_profile);
    selection
}

#[cfg(test)]
mod tests;
