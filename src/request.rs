//! One run of the whole pipeline, asked for and answered with plain values —
//! the entry point the command line shares with the Python, C and WebAssembly
//! builds.
//!
//! A [`Request`] holds the settings the `vitri` binary takes as flags, and the
//! binary builds one from its arguments. [`prepare`] runs a request over DIMACS
//! bytes and returns every file `vitri -o DIR` writes, held in memory, with a
//! [`Summary`] of the run. [`prepare_json`] and [`capabilities_json`] are the
//! same calls with JSON on both sides, for a host that has no Rust types.
//!
//! Each JSON shape carries a format tag: [`REQUEST_FORMAT`], [`RESULT_FORMAT`]
//! and [`CAPABILITIES_FORMAT`]. A key can be added under the same tag; a key
//! whose meaning changes gets a new tag.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::bundle::components::ComponentWriteOptions;
use crate::bundle::{self, BundleFile, RunPaths, RunVtree, Sink, StageOutcome, VitriRun};
use crate::candidates::MAX_CANDIDATES;
use crate::cnf::{CnfFormula, CnfMeta, Mode};
use crate::config::{self, ComponentPolicy, PreprocessStages, RunConfig, SwitchableStage};
use crate::decompose::SelectionCtx;
use crate::error::VitriError;
use crate::spec::{DEFAULT_VTREE_SPEC, one_of};

/// The tag a JSON [`Request`] may carry in its `format` key.
pub const REQUEST_FORMAT: &str = "vitri-request-v1";

/// The tag in every [`Summary`]'s `format` key.
pub const RESULT_FORMAT: &str = "vitri-result-v1";

/// The tag in the [`Capabilities`]' `format` key.
pub const CAPABILITIES_FORMAT: &str = "vitri-capabilities-v1";

/// Every key a JSON [`Request`] may carry.
pub const REQUEST_KEYS: &[&str] = &[
    "format",
    "mode",
    "vtree",
    "budget_ms",
    "components",
    "candidates",
    "simplify",
    "arjun",
    "dot",
];

/// This crate's version, as the JSON shapes report it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The settings of one run: the command line's flags, as values.
///
/// A field left `None` keeps the [`RunConfig`] value it would edit; `dot`,
/// which edits the writer's options instead, is off unless set. The JSON
/// form, read by [`Request::from_json`], is an object with any of these keys
/// ([`REQUEST_KEYS`]), where `null` is the same as leaving the key out:
///
/// | key | value | sets |
/// | --- | --- | --- |
/// | `format` | `"vitri-request-v1"` | nothing; another tag is refused |
/// | `mode` | a [`Mode::names`] token | [`RunConfig::mode`] |
/// | `vtree` | a spec string | [`RunConfig::vtree_spec`] |
/// | `budget_ms` | integer | [`RunConfig::budget_ms`] |
/// | `components` | a [`ComponentPolicy::names`] token | [`RunConfig::components`] |
/// | `candidates` | integer | [`RunConfig::candidates`] |
/// | `simplify` | boolean | [`PreprocessStages::simplify`] |
/// | `arjun` | boolean | [`PreprocessStages::arjun`] |
/// | `dot` | boolean | [`ComponentWriteOptions::dot`] |
///
/// Any other key is refused.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Request {
    /// What preprocessing must preserve; `None` detects it from the input's
    /// headers.
    pub mode: Option<Mode>,
    /// The vtree spec string.
    pub vtree: Option<String>,
    /// The wall-clock budget for the whole run, in milliseconds.
    pub budget_ms: Option<u64>,
    /// Whether each component gets its own vtree.
    pub components: Option<ComponentPolicy>,
    /// How many ranked vtree candidates to keep per built vtree.
    pub candidates: Option<u64>,
    /// Whether the simplify chain runs.
    pub simplify: Option<bool>,
    /// Whether the Arjun stage runs.
    pub arjun: Option<bool>,
    /// Whether a Graphviz `.dot` is written beside every `.vtree`.
    pub dot: bool,
}

impl Request {
    /// Read a request from its JSON form, described on [`Request`].
    ///
    /// # Errors
    ///
    /// [`VitriError::Config`] for text that is not a JSON object, a key outside
    /// [`REQUEST_KEYS`], a value of the wrong type or outside its vocabulary,
    /// and a `format` other than [`REQUEST_FORMAT`].
    pub fn from_json(text: &str) -> Result<Self, VitriError> {
        let value: Value = serde_json::from_str(text)
            .map_err(|e| VitriError::config(format!("the request is not JSON: {e}")))?;
        let Value::Object(map) = value else {
            return Err(VitriError::config(format!(
                "the request must be a JSON object, got {value}"
            )));
        };
        let mut request = Request::default();
        for (key, value) in &map {
            let key = key.as_str();
            match key {
                "format" => {
                    if let Some(tag) = text_value(key, value)?
                        && tag != REQUEST_FORMAT
                    {
                        return Err(VitriError::config(format!(
                            "format expects {REQUEST_FORMAT:?}, got {tag:?}"
                        )));
                    }
                }
                "mode" => {
                    request.mode = text_value(key, value)?
                        .map(|token| parse_mode(key, token))
                        .transpose()?;
                }
                "vtree" => request.vtree = text_value(key, value)?.map(str::to_string),
                "budget_ms" => request.budget_ms = integer_value(key, value)?,
                "components" => {
                    request.components = text_value(key, value)?
                        .map(|token| parse_components(key, token))
                        .transpose()?;
                }
                "candidates" => request.candidates = integer_value(key, value)?,
                "simplify" => request.simplify = bool_value(key, value)?,
                "arjun" => request.arjun = bool_value(key, value)?,
                "dot" => request.dot = bool_value(key, value)?.unwrap_or(false),
                _ => {
                    return Err(VitriError::config(format!(
                        "unknown request key {key:?}, expected {}",
                        one_of(REQUEST_KEYS.iter().copied()),
                    )));
                }
            }
        }
        Ok(request)
    }

    /// Set the fields of `config` this request names, leaving the rest as they
    /// are.
    ///
    /// The command line applies its request over
    /// [`RunConfig::from_env_defaults`]; [`prepare`] applies it over
    /// [`RunConfig::default`].
    pub fn apply_to(&self, config: &mut RunConfig) {
        if self.mode.is_some() {
            config.mode = self.mode;
        }
        if let Some(spec) = &self.vtree {
            config.vtree_spec.clone_from(spec);
        }
        if self.budget_ms.is_some() {
            config.budget_ms = self.budget_ms;
        }
        if let Some(policy) = self.components {
            config.components = policy;
        }
        if let Some(n) = self.candidates {
            // Past the address space is past the ceiling too, which
            // `RunConfig::validate` refuses with the ceiling in the message.
            config.candidates = usize::try_from(n).unwrap_or(usize::MAX);
        }
        if let Some(on) = self.simplify {
            config.stages.simplify = on;
        }
        if let Some(on) = self.arjun {
            config.stages.arjun = on;
        }
    }

    /// The writer options this request names.
    #[must_use]
    pub fn write_options(&self) -> ComponentWriteOptions {
        ComponentWriteOptions { dot: self.dot }
    }
}

/// `token` as a [`Mode`], or the refusal naming `key` — the flag or request key
/// it came from — and every mode.
///
/// # Errors
///
/// [`VitriError::Config`] for a token outside [`Mode::names`].
pub fn parse_mode(key: &str, token: &str) -> Result<Mode, VitriError> {
    parse_token(key, token, Mode::parse_mode, Mode::names())
}

/// `token` as a [`ComponentPolicy`], or the refusal naming `key` — the flag or
/// request key it came from — and every policy.
///
/// # Errors
///
/// [`VitriError::Config`] for a token outside [`ComponentPolicy::names`].
pub fn parse_components(key: &str, token: &str) -> Result<ComponentPolicy, VitriError> {
    parse_token(key, token, ComponentPolicy::parse, ComponentPolicy::names())
}

/// One vocabulary's token as its value, or the refusal naming `key`, the flag
/// or request key it came from, and every token the vocabulary offers.
///
/// Both settings that are spelled as a token come through here, so a caller
/// gets the same sentence whichever of them it got wrong.
fn parse_token<T>(
    key: &str,
    token: &str,
    parse: impl FnOnce(&str) -> Option<T>,
    names: impl Iterator<Item = &'static str>,
) -> Result<T, VitriError> {
    parse(token).ok_or_else(|| {
        VitriError::config(format!("{key} expects {}, got {token:?}", one_of(names)))
    })
}

/// `text` as an integer setting such as `budget_ms` or `candidates`, or the
/// refusal naming `key` — the flag or request key it came from.
///
/// # Errors
///
/// [`VitriError::Config`] for text that is not a non-negative integer that
/// fits in 64 bits.
pub fn parse_integer(key: &str, text: &str) -> Result<u64, VitriError> {
    text.parse()
        .map_err(|_| refuse_integer(key, format!("{text:?}")))
}

/// The refusal of `got`, which is not a non-negative integer, as the value of
/// the setting `key`. The one wording for every way a setting reaches this
/// crate: text, JSON, or a host's own integer.
#[must_use]
pub fn refuse_integer(key: &str, got: impl std::fmt::Display) -> VitriError {
    VitriError::config(format!("{key} expects a non-negative integer, got {got}"))
}

fn text_value<'v>(key: &str, value: &'v Value) -> Result<Option<&'v str>, VitriError> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => Ok(Some(text)),
        _ => Err(VitriError::config(format!(
            "{key} expects a string, got {value}"
        ))),
    }
}

fn integer_value(key: &str, value: &Value) -> Result<Option<u64>, VitriError> {
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| refuse_integer(key, value))
}

fn bool_value(key: &str, value: &Value) -> Result<Option<bool>, VitriError> {
    match value {
        Value::Null => Ok(None),
        Value::Bool(on) => Ok(Some(*on)),
        _ => Err(VitriError::config(format!(
            "{key} expects true or false, got {value}"
        ))),
    }
}

/// What [`prepare`] returns.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Prepared {
    /// What the run did, in the shape [`Summary`] describes.
    pub summary: Summary,
    /// Every file of the bundle, as [`VitriRun::to_files`] returns them.
    pub files: Vec<BundleFile>,
}

impl Prepared {
    /// Write every file under `dir`, as [`VitriRun::write_to_dir`] would have:
    /// directories are created as needed, a file already at one of the paths
    /// is replaced, and any other file under `dir` is left alone.
    ///
    /// # Errors
    ///
    /// [`VitriError::Io`] naming the file or directory that could not be
    /// written.
    pub fn write_to_dir(&self, dir: &Path) -> Result<(), VitriError> {
        bundle::write_files(dir, &self.files)
    }
}

/// What one run did, as [`prepare`] reports it. Serializes to the JSON object
/// tagged [`RESULT_FORMAT`].
#[derive(Clone, Debug, Serialize)]
#[non_exhaustive]
pub struct Summary {
    /// [`RESULT_FORMAT`].
    pub format: &'static str,
    /// The version of this crate that ran.
    pub vitri_version: &'static str,
    /// Which [`RunVtree`] the run ended with.
    pub status: RunStatus,
    /// The mode preprocessing preserved, after detection.
    pub mode: &'static str,
    /// The formula as parsed.
    pub input: FormulaSize,
    /// The formula preprocessing left, which `reduced.cnf` holds.
    pub reduced: FormulaSize,
    /// How a count over the reduced formula becomes a count over the input.
    pub lift: LiftSummary,
    /// What each preprocessing stage did.
    pub stages: StageSummary,
    /// The vtree, present exactly when `status` is `built`.
    pub vtree: Option<VtreeSummary>,
    /// The settings the run used, after defaults and detection.
    pub request: EffectiveRequest,
    /// The paths of [`Prepared::files`], in the same order.
    pub files: Vec<String>,
}

/// Which [`RunVtree`] a run ended with, as [`Summary::status`] reports it.
/// Serializes to its [`token`](Self::token).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    /// [`RunVtree::Built`]: there is a vtree, and [`Summary::vtree`] describes
    /// it.
    Built,
    /// [`RunVtree::FullyResolved`]: preprocessing left no variable to build
    /// one over, and the lift is the whole count.
    FullyResolved,
    /// [`RunVtree::Refuted`]: preprocessing refuted the instance, so the count
    /// is 0.
    Refuted,
}

impl RunStatus {
    /// `built`, `fully_resolved` or `refuted`.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            RunStatus::Built => "built",
            RunStatus::FullyResolved => "fully_resolved",
            RunStatus::Refuted => "refuted",
        }
    }
}

impl Serialize for RunStatus {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.token())
    }
}

/// The size of a formula.
#[derive(Clone, Copy, Debug, Serialize)]
#[non_exhaustive]
pub struct FormulaSize {
    /// Its declared variable count.
    pub variables: u32,
    /// Its clause count.
    pub clauses: usize,
}

/// The count lift, as `preprocess.json` records it.
#[derive(Clone, Debug, Serialize)]
#[non_exhaustive]
pub struct LiftSummary {
    /// [`PreprocessRecord::count_lift_pow2`](bundle::PreprocessRecord::count_lift_pow2).
    pub count_lift_pow2: u32,
    /// [`PreprocessRecord::weight_lift`](bundle::PreprocessRecord::weight_lift).
    pub weight_lift: String,
    /// The two as one factor, [`PreprocessRecord::lift`](bundle::PreprocessRecord::lift).
    pub factor: String,
}

/// [`StageReport`](bundle::StageReport) as tokens: `ran`, `skipped`, `gave_up`
/// or `discarded` for a stage the run reached, `null` for one it did not —
/// a stage the mode's chain does not have, or one after preprocessing had
/// already refuted the instance.
#[derive(Clone, Copy, Debug, Serialize)]
#[non_exhaustive]
pub struct StageSummary {
    /// The simplify chain.
    pub simplify: Option<&'static str>,
    /// The Arjun reduction.
    pub arjun: Option<&'static str>,
    /// Bounded variable addition, inside the Arjun reduction.
    pub sbva: Option<&'static str>,
}

/// The size of the vtree a run built.
#[derive(Clone, Copy, Debug, Serialize)]
#[non_exhaustive]
pub struct VtreeSummary {
    /// Its leaf count, which is the reduced formula's variable count.
    pub leaves: u32,
    /// Its node count.
    pub nodes: usize,
    /// How many components `components.json` lists.
    pub components: usize,
    /// How many variables `components.json` lists as free.
    pub free_variables: usize,
}

/// The settings a run used, with every [`Request`] key filled in.
#[derive(Clone, Debug, Serialize)]
#[non_exhaustive]
pub struct EffectiveRequest {
    /// The mode, after detection.
    pub mode: &'static str,
    /// The vtree spec string.
    pub vtree: String,
    /// The budget in milliseconds; `None` is unbounded.
    pub budget_ms: Option<u64>,
    /// The component policy.
    pub components: &'static str,
    /// How many candidates were kept per built vtree.
    pub candidates: usize,
    /// Whether the simplify chain was switched on.
    pub simplify: bool,
    /// Whether the Arjun stage was switched on.
    pub arjun: bool,
    /// Whether `.dot` files were written.
    pub dot: bool,
}

impl Summary {
    fn of(
        input: &CnfFormula,
        config: &RunConfig,
        options: ComponentWriteOptions,
        run: &VitriRun,
        paths: &RunPaths,
        files: &[String],
    ) -> Self {
        let preprocessed = &run.preprocessed;
        let record = &preprocessed.record;
        let status = match run.vtree {
            RunVtree::Built(_) => RunStatus::Built,
            RunVtree::FullyResolved => RunStatus::FullyResolved,
            RunVtree::Refuted => RunStatus::Refuted,
        };
        let vtree = match (&run.vtree, &paths.vtree) {
            (RunVtree::Built(build), Some(files)) => Some(VtreeSummary {
                leaves: build.vtree.num_leaves(),
                nodes: build.vtree.num_nodes(),
                components: files.components.manifest.components.len(),
                free_variables: files.components.manifest.free_vars_reduced_dimacs.len(),
            }),
            _ => None,
        };
        let stages = &preprocessed.stages;
        Summary {
            format: RESULT_FORMAT,
            vitri_version: VERSION,
            status,
            mode: record.mode.token(),
            input: FormulaSize::of(input),
            reduced: FormulaSize::of(&preprocessed.reduced),
            lift: LiftSummary {
                count_lift_pow2: record.count_lift_pow2,
                weight_lift: record.weight_lift.clone(),
                factor: record.lift(),
            },
            stages: StageSummary {
                simplify: stages.simplify.as_ref().map(outcome_token),
                arjun: stages.arjun.as_ref().map(outcome_token),
                sbva: stages.sbva.as_ref().map(outcome_token),
            },
            vtree,
            request: EffectiveRequest {
                mode: record.mode.token(),
                vtree: config.vtree_spec.clone(),
                budget_ms: config.budget_ms,
                components: config.components.token(),
                candidates: config.candidates,
                simplify: config.stages.simplify,
                arjun: config.stages.arjun,
                dot: options.dot,
            },
            files: files.to_vec(),
        }
    }

    /// This summary as its JSON object.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("a summary always serializes")
    }
}

impl FormulaSize {
    fn of(formula: &CnfFormula) -> Self {
        FormulaSize {
            variables: formula.num_vars,
            clauses: formula.clauses.len(),
        }
    }
}

fn outcome_token(outcome: &StageOutcome) -> &'static str {
    match outcome {
        StageOutcome::Ran => "ran",
        StageOutcome::Skipped(_) => "skipped",
        StageOutcome::GaveUp => "gave_up",
        StageOutcome::Discarded(_) => "discarded",
    }
}

/// Refuse `simplify` or `arjun` set, either way, under a mode whose
/// preprocessing has no such stage.
///
/// [`RunConfig::validate`] applies the same rule and reports it in the same
/// words, but it sees only the setting: `true` is its default, so a request
/// that ASKED for a stage the mode does not have looks like a default there.
/// That is what this reads, and the refusal itself is the configuration's.
fn refuse_absent_stage(request: &Request, mode: Mode) -> Result<(), VitriError> {
    let read = PreprocessStages::read_under(mode);
    for (asked, stage) in [
        (request.simplify, SwitchableStage::SIMPLIFY),
        (request.arjun, SwitchableStage::ARJUN),
    ] {
        if let Some(on) = asked
            && !stage.set_in(&read)
        {
            return config::refuse_absent_stage(
                &format!("{}={on}", stage.key),
                stage,
                mode,
                request.mode.is_some(),
            );
        }
    }
    Ok(())
}

/// Run `request` over the DIMACS text in `dimacs`: parse it, [`bundle::run`]
/// it, and write the bundle into memory.
///
/// The run starts from [`RunConfig::default`] and [`SelectionCtx::plain`], so
/// no `VITRI_*` variable changes the settings; the variables the vendored stack
/// reads itself still apply, as `docs/env.md` lists. The
/// [process model](crate#process-model) says when the budgeted Arjun stage
/// runs in a child process and what a host with threads has to observe.
///
/// # Errors
///
/// [`VitriError::Config`] or [`VitriError::Spec`] for settings
/// [`RunConfig::validate`] refuses, [`VitriError::Input`] for DIMACS that does
/// not parse, and anything [`bundle::run`] reports.
pub fn prepare(dimacs: &[u8], request: &Request) -> Result<Prepared, VitriError> {
    let mut config = RunConfig::default();
    request.apply_to(&mut config);
    // Before the configuration's own check, so the refusal names the request's
    // key rather than the flag.
    if let Some(mode) = request.mode {
        refuse_absent_stage(request, mode)?;
    }
    config.validate()?;
    let (formula, meta) = CnfFormula::from_dimacs(dimacs)?;
    if request.mode.is_none() {
        refuse_absent_stage(request, config.resolve_mode(&meta)?.mode)?;
    }
    let mut files = Vec::new();
    let (summary, _) = prepare_with(
        &formula,
        &meta,
        &config,
        &SelectionCtx::plain(),
        &mut Sink::in_memory(&mut files),
        request.write_options(),
    )?;
    Ok(Prepared { summary, files })
}

/// [`bundle::run`] over a parsed formula, written into `dir`, and reported as
/// one [`Summary`] beside the paths it wrote.
///
/// What `vitri -o DIR` does, as a call: [`prepare`] answers with the files in
/// memory, this one puts them on disk. Both take the same run apart into the
/// same summary.
///
/// Unlike [`prepare`], the settings are the caller's own [`RunConfig`] and
/// [`SelectionCtx`], already validated: this is the entry for a caller that
/// fills those from somewhere other than a [`Request`], the `VITRI_*` variables
/// included.
///
/// # Errors
///
/// Anything [`bundle::run`] reports, and [`VitriError::Io`] or
/// [`VitriError::Mismatch`] from writing the bundle.
pub fn prepare_to_dir(
    formula: &CnfFormula,
    meta: &CnfMeta,
    config: &RunConfig,
    selection: &SelectionCtx,
    dir: &Path,
    options: ComponentWriteOptions,
) -> Result<(Summary, RunPaths), VitriError> {
    prepare_with(
        formula,
        meta,
        config,
        selection,
        &mut Sink::at(dir),
        options,
    )
}

/// [`bundle::run`] over a parsed formula, written wherever `sink` points, and
/// reported as one [`Summary`].
///
/// [`prepare`] is this call with a sink in memory and [`prepare_to_dir`] the
/// same call with a sink on a directory. Both read the run off this one
/// summary, so a number the tool prints is the number the JSON carries.
///
/// # Errors
///
/// Anything [`bundle::run`] reports, and [`VitriError::Io`] or
/// [`VitriError::Mismatch`] from writing the bundle.
pub(crate) fn prepare_with(
    formula: &CnfFormula,
    meta: &CnfMeta,
    config: &RunConfig,
    selection: &SelectionCtx,
    sink: &mut Sink<'_>,
    options: ComponentWriteOptions,
) -> Result<(Summary, RunPaths), VitriError> {
    let run = bundle::run(formula, meta, config, selection)?;
    let paths = run.write_to(sink, options)?;
    let summary = Summary::of(formula, config, options, &run, &paths, sink.written());
    Ok((summary, paths))
}

/// [`prepare`] with JSON on both sides: `request` is a [`Request`] in its JSON
/// form, and the answer is one JSON object.
///
/// On success, `{"ok": true, "summary": …, "files": {path: text}}`, where
/// `summary` is the [`Summary`] object and `files` maps each path to the file's
/// text. On failure, `{"ok": false, "error": {"kind": …, "message": …}}` with
/// the error's kind as [`ErrorKind::token`](crate::error::ErrorKind::token)
/// spells it and its message.
#[must_use]
pub fn prepare_json(dimacs: &[u8], request: &str) -> String {
    let answer = Request::from_json(request).and_then(|request| prepare(dimacs, &request));
    let envelope = match answer {
        Ok(prepared) => {
            let files: Map<String, Value> = prepared
                .files
                .into_iter()
                .map(|file| {
                    let text = String::from_utf8_lossy(&file.contents).into_owned();
                    (file.path, Value::String(text))
                })
                .collect();
            serde_json::json!({ "ok": true, "summary": prepared.summary, "files": files })
        }
        Err(error) => serde_json::json!({
            "ok": false,
            "error": { "kind": error.kind().token(), "message": error.to_string() },
        }),
    };
    envelope.to_string()
}

/// What this build accepts, as [`capabilities`] reports it. Serializes to the
/// JSON object tagged [`CAPABILITIES_FORMAT`].
#[derive(Clone, Debug, Serialize)]
#[non_exhaustive]
pub struct Capabilities {
    /// [`CAPABILITIES_FORMAT`].
    pub format: &'static str,
    /// The version of this crate.
    pub vitri_version: &'static str,
    /// [`REQUEST_FORMAT`].
    pub request_format: &'static str,
    /// [`RESULT_FORMAT`].
    pub result_format: &'static str,
    /// [`REQUEST_KEYS`].
    pub request_keys: Vec<&'static str>,
    /// [`Mode::names`].
    pub modes: Vec<&'static str>,
    /// [`ComponentPolicy::names`].
    pub components: Vec<&'static str>,
    /// [`DEFAULT_VTREE_SPEC`].
    pub default_vtree: &'static str,
    /// [`vtree_spec_bases`](crate::spec::vtree_spec_bases).
    pub vtree_bases: Vec<String>,
    /// [`MAX_CANDIDATES`].
    pub max_candidates: usize,
    /// The stages a request can switch, each on unless it is switched off.
    pub stages: StageSwitches,
    /// For each mode, which of those stages its preprocessing has. A request
    /// that sets a stage the resolved mode lacks is refused.
    pub mode_stages: BTreeMap<&'static str, StageSwitches>,
}

/// Which preprocessing stages are on.
#[derive(Clone, Copy, Debug, Serialize)]
#[non_exhaustive]
pub struct StageSwitches {
    /// The simplify chain.
    pub simplify: bool,
    /// The Arjun stage.
    pub arjun: bool,
}

/// What this build accepts: the request keys and their vocabularies.
#[must_use]
pub fn capabilities() -> Capabilities {
    let stages = PreprocessStages::default();
    Capabilities {
        format: CAPABILITIES_FORMAT,
        vitri_version: VERSION,
        request_format: REQUEST_FORMAT,
        result_format: RESULT_FORMAT,
        request_keys: REQUEST_KEYS.to_vec(),
        modes: Mode::names().collect(),
        components: ComponentPolicy::names().collect(),
        default_vtree: DEFAULT_VTREE_SPEC,
        vtree_bases: crate::spec::vtree_spec_bases(),
        max_candidates: MAX_CANDIDATES,
        stages: StageSwitches {
            simplify: stages.simplify,
            arjun: stages.arjun,
        },
        mode_stages: Mode::names()
            .filter_map(Mode::parse_mode)
            .map(|mode| {
                let read = PreprocessStages::read_under(mode);
                let switches = StageSwitches {
                    simplify: read.simplify,
                    arjun: read.arjun,
                };
                (mode.token(), switches)
            })
            .collect(),
    }
}

/// [`capabilities`] as its JSON object.
#[must_use]
pub fn capabilities_json() -> String {
    serde_json::to_string(&capabilities()).expect("the capabilities always serialize")
}
