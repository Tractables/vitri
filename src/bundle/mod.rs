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
//!   what it produced. Under `wmc` the same stages run with the unequal-weight
//!   variables FROZEN out of DVE, and every factor the integer chain writes as
//!   `×2` or `×1` becomes an exact rational.
//! - **projection-preserving** (`pmc`, `pwmc`) — Arjun's projection-set
//!   minimization, then show-frozen strengthening and projected BVE. The
//!   count-preserving stages do not preserve a projected count and do not run.
//! - **function-preserving** (`compile`) — only the stages whose effect the
//!   record reconstructs: forced-literal propagation and free-variable removal.
//!   It preprocesses less than either counting chain, and its output recovers
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
//!   solves the right problem.
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
use num_traits::One;
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
mod export;
mod plumbing;
mod projection_chain;
mod record;
mod report;
mod session;
mod stage;
use compile_chain::compile_preserving_bundle;
use count_chain::count_preserving_bundle_with_stage1;
// What the three chains and the component writer reach for, named rather than
// globbed, so an item shared with them is shared deliberately rather than by
// being written down.
use export::{DotFor, write_vtree_files};
use plumbing::{original_weights, preprocess_config, refuted, to_json_pretty, weight_table};
// The one destination type: `request` writes a run to a directory or into
// memory through the same call, so there is no second write path to keep in
// step with this one.
pub(crate) use export::Sink;
use projection_chain::projection_preserving_bundle;
use record::RecordLift;
use session::PreprocessOutcome;

pub use export::{
    BundleFile, BundlePaths, ComponentFiles, RunFiles, RunPaths, VtreeFiles, write_files,
};
pub use record::{PreprocessRecord, RECORD_FORMAT_TAG};
pub use report::{
    CountLift, DiscardReason, DveDecisionTrace, PreprocessDecisionTrace, PreprocessPhase,
    PreprocessPhaseTrace, PreprocessTelemetry, ProbeDecisionCounts, SkipReason, StageOutcome,
    StageReport,
};
pub use session::{FrontendRetryConfig, FrontendSession, RetryBudget, frontend, run};

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
    /// Reproducible preprocessing decisions when the run selected the
    /// deterministic preprocessing clock; `None` in wall-clock mode.
    pub decision_trace: Option<PreprocessDecisionTrace>,
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
    /// [`Self::reduced`]'s variable space.
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
    finish_bundle(&mut bundle, config, started, 0);
    Ok(PreprocessOutcome {
        bundle,
        count_stage1,
    })
}

/// The two closing values every route into preprocessing owes its bundle: the
/// wall it took, and, under a deterministic clock, a decision trace even when
/// no stage left one, so an empty trace and no trace stay different answers.
///
/// `earlier_ms` is preprocessing this bundle inherited rather than repeated,
/// the simplify checkpoint a retry starts from. It is 0 for a run that did all
/// of its own work.
fn finish_bundle(
    bundle: &mut PreprocessBundle,
    config: &RunConfig,
    started: std::time::Instant,
    earlier_ms: u64,
) {
    if matches!(
        config.preprocess_clock,
        crate::config::PreprocessClock::Deterministic { .. }
    ) && bundle.decision_trace.is_none()
    {
        bundle.decision_trace = Some(PreprocessDecisionTrace::default());
    }
    bundle.telemetry.total_ms = earlier_ms.saturating_add(started.elapsed().as_millis() as u64);
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
    /// Preprocessing refuted the instance, so `count(original)` is 0 and
    /// nothing has to be compiled to say so. The bundle still exports the
    /// synthetic contradiction the record describes, but no vtree is built over
    /// it: it stands for a formula whose count is already known, and a vtree
    /// over it would be construction spent on an answered instance.
    /// An outcome, not a failure: the record alone is the answer.
    Refuted,
}

impl VitriRun {
    /// The vtree construction, when this run built one.
    pub fn built(&self) -> Option<&VtreeBuild> {
        match &self.vtree {
            RunVtree::Built(b) => Some(b),
            RunVtree::FullyResolved | RunVtree::Refuted => None,
        }
    }
}

#[cfg(test)]
mod tests;
