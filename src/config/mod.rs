//! [`RunConfig`] — the explicit, call-site-visible configuration for this
//! crate's public entry points.
//!
//! An environment variable can't carry a per-call budget for a library someone
//! else embeds: it's invisible at the call site and can't describe two
//! concurrent runs with different budgets.
//!
//! `budget_ms` is the run-wide budget input on this path. Every site that scales
//! a sub-budget from it is handed [`RunConfig::effective_budget_ms`] as an
//! argument — on the construction side through the build limits
//! [`crate::component::build_vtree`] assembles — so a run's budget travels with
//! the run rather than through process state. A caller that has already carved
//! out Arjun's share can name that one stage's duration with
//! [`RunConfig::arjun_budget`] instead of deriving it a second time.
//! [`RunConfig::simplify`] similarly configures Vitri's one simplify path;
//! callers tune its work without taking ownership of preprocessing.

use std::time::{Duration, Instant};

use crate::error::VitriError;
use crate::preprocess::ArjunOptions;
use crate::spec::DEFAULT_VTREE_SPEC;

mod budget;
mod policies;

pub use budget::ConstructionBudget;
pub(crate) use policies::Chain;
pub use policies::{
    ArjunBudget, ArjunClauseGrowth, ComponentPolicy, DvePolicy, PreprocessClock, PreprocessStages,
    ProjectionNoGain, ProjectionPolicy, SimplifyPolicy,
};

/// One setting of [`RunConfig::conditional_knobs`]: how a refusal names it,
/// what it needs, and the value that asks for nothing.
struct ConditionalKnob {
    /// The knob and its value, as a refusal opens with it.
    label: String,
    /// What has to be there before the knob changes anything.
    needs: Requirement,
    /// The setting that makes the knob inert on purpose, phrased to follow
    /// "or".
    instead: &'static str,
}

/// What a knob needs before it can change anything.
///
/// Two of these are a stage, which both routes can read off a
/// [`PreprocessStages`]; the other two are a chain, because the knob acts on
/// something only that chain produces even when the Arjun stage is on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Requirement {
    /// The simplify chain.
    Simplify,
    /// The Arjun stage.
    Arjun,
    /// The count-preserving chain, which is where the clause-growth gate is.
    CountChain,
    /// The projected chain, which is where projected Arjun is.
    ProjectionChain,
}

impl Requirement {
    /// The stage the knob acts inside, which has to be switched on for it to
    /// do anything. Both chains run their part of the work in Arjun's stage.
    fn stage(self) -> SwitchableStage {
        match self {
            Requirement::Simplify => SwitchableStage::SIMPLIFY,
            Requirement::Arjun | Requirement::CountChain | Requirement::ProjectionChain => {
                SwitchableStage::ARJUN
            }
        }
    }

    /// Whether `mode`'s preprocessing has it.
    fn met_by(self, mode: crate::cnf::Mode) -> bool {
        match self {
            Requirement::Simplify | Requirement::Arjun => {
                self.stage().set_in(&PreprocessStages::read_under(mode))
            }
            Requirement::CountChain => Chain::for_mode(mode) == Chain::Count,
            Requirement::ProjectionChain => Chain::for_mode(mode) == Chain::Projection,
        }
    }

    /// What a mode that does not meet this is missing, as a refusal says it.
    fn missing(self) -> &'static str {
        match self {
            Requirement::Simplify => "no simplify stage",
            Requirement::Arjun => "no Arjun stage",
            Requirement::CountChain => "no count-preserving chain, and so no clause-growth gate",
            Requirement::ProjectionChain => "no projected chain",
        }
    }

    /// The modes that do meet it, as `mc/wmc`, read off the modes themselves
    /// so a refusal cannot name a list the chains have moved on from.
    fn modes(self) -> String {
        crate::cnf::Mode::ALL
            .iter()
            .filter(|&&mode| self.met_by(mode))
            .map(|mode| mode.token())
            .collect::<Vec<_>>()
            .join("/")
    }
}

/// One switchable preprocessing stage, with the two spellings a caller has for
/// it: the command line's flag and the request key.
///
/// The spellings sit beside the stage so each route hands
/// [`refuse_absent_stage`] the one its own caller used.
#[derive(Clone, Copy)]
pub(crate) struct SwitchableStage {
    /// The stage, as a message names it.
    pub(crate) name: &'static str,
    /// The flag that switches it off.
    pub(crate) flag: &'static str,
    /// The [`Request`](crate::request::Request) key that sets it either way.
    pub(crate) key: &'static str,
    /// This stage's field of a [`PreprocessStages`], which is read both as the
    /// caller's switches and as the stages a mode has.
    field: fn(&PreprocessStages) -> bool,
}

impl SwitchableStage {
    pub(crate) const SIMPLIFY: Self = SwitchableStage {
        name: "simplify",
        flag: "--no-simplify",
        key: "simplify",
        field: |stages| stages.simplify,
    };
    pub(crate) const ARJUN: Self = SwitchableStage {
        name: "Arjun",
        flag: "--no-arjun",
        key: "arjun",
        field: |stages| stages.arjun,
    };
    /// Both of them, in the order preprocessing runs them.
    pub(crate) const ALL: [Self; 2] = [Self::SIMPLIFY, Self::ARJUN];

    /// This stage's field of `stages`.
    pub(crate) fn set_in(self, stages: &PreprocessStages) -> bool {
        (self.field)(stages)
    }
}

/// Configuration for a preprocess-and-build-a-vtree run.
///
/// `Default` is the production configuration: no budget limit,
/// [`DEFAULT_VTREE_SPEC`], every conversion dimension searched, every
/// preprocessing stage on, per-component vtrees.
///
/// Comparable, like every other configuration type here: a caller that keeps a
/// baseline configuration beside the one it is about to run can ask whether it
/// changed anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunConfig {
    /// Wall-clock budget for the whole run, in ms, measured from the moment the
    /// entry point is called. `None` = unbounded.
    ///
    /// For the WHOLE run: [`crate::run`] anchors the budget once and both
    /// halves stop at that one instant, so preprocessing spending most of it
    /// leaves construction the rest. Calling a half on its own makes that call
    /// the run.
    ///
    /// This is both the hard-ish cutoff (the preprocessing stages and vtree
    /// construction hand back what they have) and the SCALE that sub-budgets are
    /// derived from — a bigger budget spends proportionally more on each phase,
    /// so the same CNF can yield a different (better) vtree.
    pub budget_ms: Option<u64>,

    /// An absolute deadline, for a caller whose budget started before this call
    /// (e.g. a driver that already spent time parsing). Takes precedence over
    /// `budget_ms` as the cutoff; `budget_ms` still supplies the scale when both
    /// are set. `None` = derive the deadline from `budget_ms`.
    pub deadline: Option<Instant>,

    /// The clock used by preprocessing's budget decisions. Existing callers
    /// remain wall-clock based; embedding compilers that require reproducible
    /// preprocessing select [`PreprocessClock::Deterministic`] explicitly.
    pub preprocess_clock: PreprocessClock,

    /// How much wall-clock time the Arjun stage may spend.
    ///
    /// [`ArjunBudget::Derived`] preserves the ordinary run-wide budget policy.
    /// [`ArjunBudget::Exact`] is for a caller that has already divided its own
    /// wall and must not have this stage's share divided, floored or capped
    /// again. Both policies are clamped to [`Self::deadline`].
    pub arjun_budget: ArjunBudget,

    /// Whether a sound Arjun result may be kept when it grew the clause count.
    ///
    /// [`ArjunClauseGrowth::Reject`] is the default and compares a candidate
    /// with the formula handed to Arjun. [`ArjunClauseGrowth::KeepSound`]
    /// bypasses that gate only. [`ArjunClauseGrowth::RejectAgainst`] compares
    /// against an embedding caller's count-preserving formula instead. Neither
    /// policy ever bypasses a correctness discard. Non-default policies require
    /// the enabled count-preserving (`mc`/`wmc`) Arjun stage.
    pub arjun_clause_growth: ArjunClauseGrowth,

    /// How much of the projection-preserving chain runs, and whether its Arjun
    /// stage may keep a sound result that did not minimize the projection.
    ///
    /// [`ProjectionPolicy::Full`] is the default and preserves the complete
    /// projection chain. [`ProjectionPolicy::ArjunOnly`] is for an embedding
    /// caller that needs Arjun's own checkpoint without the projected tail.
    pub projection_policy: ProjectionPolicy,

    /// How much of what the run has left vtree construction may spend, or how
    /// much work it may do.
    ///
    /// [`ConstructionBudget::Share`] by default, which is the behaviour every
    /// caller had before this field existed. Read the result back with
    /// [`Self::construction_deadline`].
    ///
    /// Preprocessing is unaffected whichever policy is set: this bounds
    /// construction alone.
    pub construction_budget: ConstructionBudget,

    /// `--vtree` spec string, e.g. `portfolio`, `flowcutter-primal`, `minfill`.
    /// Defaults to [`DEFAULT_VTREE_SPEC`].
    pub vtree_spec: String,

    /// How a tree decomposition is read off as a vtree: the dimensions this
    /// names are fixed for every family the run builds with, and the ones it
    /// leaves open are searched.
    ///
    /// `Default` leaves all three open, which is the whole search. A spec
    /// string that names a dimension itself wins over this for that dimension
    /// and that spec, so this is the run-wide default the spec refines rather
    /// than a second place to set the same thing.
    pub reading: crate::decompose::Reading,

    /// Which preprocessing stages run before the vtree is built. All on by default;
    /// turning one off changes the formula the vtree is built over.
    pub stages: PreprocessStages,

    /// Budgets and optional count-only work for the enabled simplify stage.
    ///
    /// This configures the same path selected by [`Self::stages`]; it never
    /// selects a second implementation. A non-default policy is refused when
    /// that mode has no simplify stage or the stage was switched off.
    pub simplify: SimplifyPolicy,

    /// Whether the formula's components each get their own vtree, or one vtree
    /// spans all of them.
    pub components: ComponentPolicy,

    /// How many ranked vtree candidates to retain and export per built vtree —
    /// "the best vtree, or the best set of vtrees".
    ///
    /// `1` (the default): the portfolio scores several candidates, returns the
    /// winner, and drops the rest. `N > 1` keeps up to `N` distinct candidates
    /// with their scores, so a consumer with a different cost model can re-rank
    /// them. See [`crate::candidates`] for the ordering/dedup rules and
    /// [`crate::candidates::MAX_CANDIDATES`] for the ceiling.
    ///
    /// Retention never changes which candidate wins — the emitted vtree is
    /// always the candidate set's rank-0 entry.
    ///
    /// Only a portfolio spec has a candidate set to retain.
    pub candidates: usize,

    /// What preprocessing must preserve.
    ///
    /// `None` (the default) detects it from the CNF's own headers; set
    /// explicitly, it wins over the headers. See [`Self::resolve_mode`].
    pub mode: Option<crate::cnf::Mode>,

    /// Whether the bundle retains the formula the Arjun stage was given, on
    /// [`PreprocessBundle::arjun_input`](crate::bundle::PreprocessBundle::arjun_input).
    ///
    /// Off by default: it is a second whole formula held in memory, and a
    /// caller that only wants the reduced formula and its lift never reads it.
    /// A caller that re-reduces formulas DERIVED from this run's — cofactors,
    /// components, conditioned branches — starts from that formula rather than
    /// from the input, and turns this on.
    pub retain_arjun_input: bool,

    /// What the Arjun stage is configured with — effort, bounded variable
    /// addition, the oracle ceilings, what to do with an overrun, the seed, and
    /// the learnt-clause harvest.
    ///
    /// Only the count-preserving [`Mc`](crate::cnf::Mode::Mc) chain's Arjun
    /// stage harvests learnt clauses onto
    /// [`PreprocessBundle::learnt_clauses_reduced_dimacs`](crate::bundle::PreprocessBundle::learnt_clauses_reduced_dimacs);
    /// asking for it under another mode, or with the Arjun stage off, is refused
    /// by [`crate::bundle::preprocess`] rather than answered with an empty list.
    pub arjun: ArjunOptions,
}

/// What [`RunConfig::resolve_mode`] settled on, plus what it had to ignore to
/// get there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMode {
    /// The mode preprocessing will actually preserve — the explicit
    /// [`RunConfig::mode`] if there was one, otherwise what the headers declared.
    pub mode: crate::cnf::Mode,

    /// One line per header declaration this mode's preprocessing does not use —
    /// weights under an unweighted mode, a `c p show` set under an unprojected
    /// one. Each is already `c `-prefixed, so a caller can print it straight to
    /// stderr beside DIMACS comment output. Empty unless [`RunConfig::mode`]
    /// was set explicitly.
    pub notices: Vec<String>,
}

impl Default for RunConfig {
    fn default() -> Self {
        RunConfig {
            budget_ms: None,
            deadline: None,
            preprocess_clock: PreprocessClock::default(),
            arjun_budget: ArjunBudget::default(),
            arjun_clause_growth: ArjunClauseGrowth::default(),
            projection_policy: ProjectionPolicy::default(),
            construction_budget: ConstructionBudget::default(),
            vtree_spec: DEFAULT_VTREE_SPEC.to_string(),
            reading: crate::decompose::Reading::default(),
            stages: PreprocessStages::default(),
            simplify: SimplifyPolicy::default(),
            components: ComponentPolicy::Split,
            candidates: 1,
            mode: None,
            retain_arjun_input: false,
            arjun: ArjunOptions::default(),
        }
    }
}

impl RunConfig {
    /// [`Default`], with the knobs that have a `VITRI_*` variable filled from
    /// the process environment.
    ///
    /// For this crate's own command-line tool. An embedded caller normally uses
    /// [`Default`] and sets only the fields it cares about, so its behaviour
    /// can't change because of a variable exported in the launching shell.
    ///
    /// The construction-side knobs live on
    /// [`SelectionCtx`](crate::decompose::SelectionCtx), filled by
    /// [`SelectionCtx::with_env_defaults`](crate::decompose::SelectionCtx::with_env_defaults).
    ///
    /// The preprocessing knobs are read inside preprocessing, each beside the
    /// parser that owns its accepted spellings, and handed back here as one
    /// [`ArjunOptions`].
    ///
    /// [`Self::budget_ms`] is filled from `VITRI_BUDGET_MS` — THE one place that
    /// variable is read. It is the only tolerant knob here: a value that is not
    /// a `u64` leaves the run unbounded rather than failing, as `docs/env.md`
    /// records, so a stale export cannot stop a run that never asked for a
    /// budget.
    ///
    /// # Errors
    ///
    /// [`VitriError::Env`] naming the offending variable and the form it
    /// expects.
    pub fn from_env_defaults() -> Result<Self, VitriError> {
        Ok(RunConfig {
            budget_ms: budget_hint_ms(crate::env::env_opt("VITRI_BUDGET_MS").as_deref()),
            arjun: crate::preprocess::env_defaults()?,
            ..Self::default()
        })
    }

    /// Reject configurations that cannot do what they say — checked once, here,
    /// so the binary and any embedding caller fail identically on the same input.
    ///
    /// # Errors
    ///
    /// [`VitriError::Config`] for a field that contradicts another one, and
    /// [`VitriError::Spec`] naming [`Self::vtree_spec`] when the spec carries a
    /// token its family cannot honor: an inert token is a mistake in the request,
    /// so it is reported here rather than part-way through a build that has
    /// already spent its budget preprocessing the formula.
    pub fn validate(&self) -> Result<(), VitriError> {
        crate::spec::validate_vtree_spec(&self.vtree_spec)?;
        if let Some(dve) = self.simplify.dve
            && (dve.rounds == 0 || dve.budget_ms == 0)
        {
            return Err(VitriError::config(format!(
                "simplify.dve is armed with rounds={} and budget_ms={}: both must be positive; \
                 use simplify.dve=None to disable DVE",
                dve.rounds, dve.budget_ms,
            )));
        }
        if self.simplify.backbone_budget_ms.is_none()
            && self.simplify.equivalence_budget_ms.is_some()
        {
            return Err(VitriError::config(
                "simplify.equivalence_budget_ms is inert when \
                 simplify.backbone_budget_ms=None because SAT equivalence probing belongs to \
                 the backbone prefix: set simplify.equivalence_budget_ms=None or provide a \
                 backbone budget",
            ));
        }
        for knob in self.conditional_knobs() {
            let stage = knob.needs.stage();
            if !stage.set_in(&self.stages) {
                return Err(VitriError::config(format!(
                    "{} is inert because the {} stage is off: switch the stage on, or {}",
                    knob.label, stage.name, knob.instead,
                )));
            }
        }
        // Only an EXPLICIT mode can be judged here; a detected one is not known
        // until the instance's headers have been read, and
        // `crate::bundle::preprocess` applies the same rule to it there.
        if let Some(mode) = self.mode {
            self.refuse_inert(mode)?;
        }
        if self.construction_budget == (ConstructionBudget::Deterministic { units: 0 }) {
            return Err(VitriError::config(
                "a deterministic construction budget of 0 work units asks construction to do \
                 no work at all — pass the work a construction should be allowed to do, which \
                 ConstructionBudget::for_wall_ms converts from a wall in milliseconds",
            ));
        }
        if self.candidates == 0 {
            return Err(VitriError::config(
                "candidates must be at least 1 (the selected vtree is always kept)",
            ));
        }
        if self.candidates > crate::candidates::MAX_CANDIDATES {
            return Err(VitriError::config(format!(
                "candidates is {} but the ceiling is {} — every retained candidate holds a \
                 live vtree over the formula being built, so the retained set is a peak-memory \
                 decision and is refused rather than silently truncated",
                self.candidates,
                crate::candidates::MAX_CANDIDATES,
            )));
        }
        if crate::candidates::retains_set(self.candidates)
            && !crate::spec::spec_has_candidates(&self.vtree_spec)
        {
            return Err(VitriError::config(format!(
                "candidates is {} but vtree spec {:?} builds a single vtree — only the \
                 portfolio spec ({}) scores several candidates and therefore has a candidate set to \
                 retain",
                self.candidates, self.vtree_spec, DEFAULT_VTREE_SPEC,
            )));
        }
        Ok(())
    }
    /// Every knob that only does something inside one stage, or on something
    /// only one chain produces: what it is set to, what it needs, and the
    /// setting that asks for nothing.
    ///
    /// One row per knob, read by both routes. [`Self::validate`] refuses a row
    /// whose stage is switched off, which it can judge without an instance;
    /// [`Self::refuse_inert`] refuses a row the mode that will run has nothing
    /// to answer with. A knob that is at its default produces no row, so a
    /// setting nobody asked for is never reported as inert.
    fn conditional_knobs(&self) -> Vec<ConditionalKnob> {
        let mut knobs = Vec::new();
        let mut push = |label: String, needs: Requirement, instead: &'static str| {
            knobs.push(ConditionalKnob {
                label,
                needs,
                instead,
            });
        };
        if self.simplify != SimplifyPolicy::default() {
            push(
                "a non-default simplify policy".to_string(),
                Requirement::Simplify,
                "use SimplifyPolicy::default()",
            );
        }
        if self.arjun_clause_growth.requires_count_arjun() {
            push(
                format!("arjun_clause_growth {:?}", self.arjun_clause_growth),
                Requirement::CountChain,
                "use ArjunClauseGrowth::Reject",
            );
        }
        if let ProjectionPolicy::ArjunOnly(no_gain) = self.projection_policy {
            push(
                format!("projection_policy ArjunOnly({no_gain:?})"),
                Requirement::ProjectionChain,
                "use ProjectionPolicy::Full",
            );
        }
        if let ArjunBudget::Exact(duration) = self.arjun_budget {
            push(
                format!("arjun_budget Exact({duration:?})"),
                Requirement::Arjun,
                "use ArjunBudget::Derived",
            );
        }
        knobs
    }

    /// Refuse every request this run makes that `mode` has nothing to answer
    /// with: a knob of [`Self::conditional_knobs`] whose stage or chain the
    /// mode's preprocessing does not have, a stage switched off that it does
    /// not have, and a learnt-clause export no stage of it could fill. Each
    /// names what was asked for, the mode, and, through [`detected_note`], how
    /// the mode was arrived at.
    ///
    /// `mode` is the mode that will actually run ([`Self::resolve_mode`]), which
    /// is the only point at which the declared and the detected route have the
    /// same answer. Asking for something the chain cannot do is a mistake in the
    /// request rather than a no-op, so it is refused before any budget is spent
    /// on the run.
    pub(crate) fn refuse_inert(&self, mode: crate::cnf::Mode) -> Result<(), VitriError> {
        let read = PreprocessStages::read_under(mode);
        let chain = Chain::for_mode(mode);
        let how = detected_note(self.mode.is_some());
        for knob in self.conditional_knobs() {
            if !knob.needs.met_by(mode) {
                return Err(VitriError::config(format!(
                    "{} does nothing under mode {}{how}: that mode's preprocessing has {}. \
                     Run {}, or {}",
                    knob.label,
                    mode.token(),
                    knob.needs.missing(),
                    knob.needs.modes(),
                    knob.instead,
                )));
            }
        }
        if chain == Chain::Compile && self.simplify.customizes_count_only() {
            return Err(VitriError::config(format!(
                "simplify.detect_gates and simplify.dve are count-only; changing either does \
                 nothing under mode {} because compile caps both stages off. Leave both at \
                 SimplifyPolicy::default(), or run mc/wmc",
                mode.token(),
            )));
        }
        for stage in SwitchableStage::ALL {
            if !stage.set_in(&self.stages) && !stage.set_in(&read) {
                return refuse_absent_stage(stage.flag, stage, mode, self.mode.is_some());
            }
        }
        // One source of learnt clauses exists: the Arjun stage of the
        // count-preserving unweighted chain. Under any other mode, or with that
        // stage switched off, an empty list would be indistinguishable from
        // "Arjun derived nothing", so the request is an error instead.
        if self.arjun.export_learned_clauses {
            if mode != crate::cnf::Mode::Mc {
                return Err(VitriError::config(format!(
                    "arjun.export_learned_clauses (VITRI_ARJUN_EXPORT_LEARNED_CLAUSES) does nothing under \
                     mode {}: the clauses come from the Arjun stage of the count-preserving chain, \
                     which only mode {} runs. Drop the request, or preprocess under {}",
                    mode.token(),
                    crate::cnf::Mode::Mc.token(),
                    crate::cnf::Mode::Mc.token(),
                )));
            }
            if !self.stages.arjun {
                return Err(VitriError::config(
                    "arjun.export_learned_clauses (VITRI_ARJUN_EXPORT_LEARNED_CLAUSES) does nothing with the \
                     Arjun stage off (--no-arjun): Arjun's own solver is what derives the clauses, and \
                     no other stage does. Drop the request, or let the Arjun stage run",
                ));
            }
        }
        Ok(())
    }

    /// The mode this run preprocesses for: [`Self::mode`] when set, otherwise detected
    /// from `meta`.
    ///
    /// Detection reads both the `c t <track>` header and the `c p` lines, so a
    /// file with a show set or weights but no declared track is still handled as
    /// projected/weighted rather than counted plainly.
    ///
    /// An explicit mode wins over the headers, including when it moves to a task
    /// the file's own headers subsume — a weighted instance reduced under `mc`,
    /// or any instance under `compile`. Each header declaration the chosen mode
    /// doesn't use produces one [`ResolvedMode::notices`] line.
    ///
    /// # Errors
    ///
    /// A mode whose preprocessing needs data the file lacks: a projected mode on a
    /// file with no `c p show` line — there is no show set to preserve, so the
    /// mode is inert rather than merely narrower. Checked on the mode that will
    /// actually run, so the detected route is covered too: a `c t pmc` header
    /// asks for a projected count as loudly as an explicit mode does, and
    /// neither can run without a show set. (The converse is fine: a weighted
    /// mode on a file with no weight lines is a legitimate all-weights-1
    /// instance.)
    pub fn resolve_mode(&self, meta: &crate::cnf::CnfMeta) -> Result<ResolvedMode, VitriError> {
        use crate::cnf::Mode;
        let declares_weights = meta.mode().is_weighted() || meta.declared_weights().is_some();
        // Detection asks a wider question than "does the file carry a show set":
        // a `c t pmc` header asks for a projected count even before the
        // `c p show` line that must accompany it is read.
        let declares_show = meta.mode().is_projected() || meta.declared_show_vars().is_some();
        let detected = match (declares_show, declares_weights) {
            (false, false) => Mode::Mc,
            (false, true) => Mode::Wmc,
            (true, false) => Mode::Pmc,
            (true, true) => Mode::Pwmc,
        };
        let Some(asked) = self.mode else {
            require_show_set(detected, meta)?;
            return Ok(ResolvedMode {
                mode: detected,
                notices: Vec::new(),
            });
        };
        require_show_set(asked, meta)?;
        // `compile` re-emits every declaration it was given rather than counting
        // under it, so it ignores nothing and has nothing to report.
        let mut notices = Vec::new();
        if asked != Mode::Compile {
            if declares_weights && !asked.is_weighted() {
                notices.push(format!(
                    "c note: ignoring weight declarations (mode {})",
                    asked.token(),
                ));
            }
            if declares_show && !asked.is_projected() {
                notices.push(format!(
                    "c note: ignoring the projection show set (mode {})",
                    asked.token(),
                ));
            }
        }
        Ok(ResolvedMode {
            mode: asked,
            notices,
        })
    }

    /// The instant this run must stop by: an explicit `deadline` wins, else
    /// `budget_ms` counted from `now`. `None` = no cutoff.
    pub fn resolved_deadline(&self, now: Instant) -> Option<Instant> {
        self.deadline
            .or_else(|| self.budget_ms.map(|ms| now + Duration::from_millis(ms)))
    }

    /// The budget the internal budget sites should scale their sub-budgets
    /// from. Derived from `deadline` when `budget_ms` is unset, so a
    /// deadline-only caller doesn't silently get *unbounded* sub-budget
    /// defaults while its hard cutoff truncates them mid-phase.
    pub fn effective_budget_ms(&self, now: Instant) -> Option<u64> {
        self.budget_ms.or_else(|| {
            self.deadline
                .map(|d| d.saturating_duration_since(now).as_millis() as u64)
        })
    }

    /// This configuration with both budget fields resolved against `now`: the
    /// instant the run must stop at, and the millisecond scale its sub-budgets
    /// are derived from.
    ///
    /// A run is several phases, and they share one budget only if the instant
    /// it ends at is decided ONCE. Anchoring here and handing the result to
    /// every phase is what makes [`Self::budget_ms`] a budget for the whole
    /// run: a phase that reads it back gets what is left of the original, not
    /// a fresh copy of it counted from its own start.
    pub(crate) fn anchored(&self, now: Instant) -> RunConfig {
        RunConfig {
            deadline: self.resolved_deadline(now),
            budget_ms: self.effective_budget_ms(now),
            ..self.clone()
        }
    }
}

/// The pure half of the `VITRI_BUDGET_MS` read, so the values it accepts can be
/// pinned without touching the process environment.
///
/// Anything that is not a `u64` — the empty string included — reads as unset and
/// leaves the run unbounded. This is the one knob that tolerates a value it
/// cannot read instead of refusing the run: it is a DEFAULT for a field the
/// caller usually sets itself, so a stale export must not stop a run that never
/// asked for a budget.
fn budget_hint_ms(raw: Option<&str>) -> Option<u64> {
    raw.and_then(|t| t.parse::<u64>().ok())
}

/// The clause a refusal adds after the mode's name when the mode was detected
/// rather than declared, so a user is not left looking for a `--mode` they
/// never typed.
fn detected_note(declared: bool) -> &'static str {
    if declared {
        ""
    } else {
        " (detected from the instance's own headers — no --mode was given)"
    }
}

/// Refuse `switch`, a stage switch as the caller spelt it — the flag
/// `--no-arjun`, or the request key `arjun=true` — under `mode`, whose
/// preprocessing has no `stage`. `declared` says whether the caller named the
/// mode. The one wording for the command line and the request API.
pub(crate) fn refuse_absent_stage(
    switch: &str,
    stage: SwitchableStage,
    mode: crate::cnf::Mode,
    declared: bool,
) -> Result<(), VitriError> {
    Err(VitriError::config(format!(
        "{switch} does nothing under mode {}{}: that mode's preprocessing has no {} \
         stage. Drop it, or run a mode whose preprocessing has one",
        mode.token(),
        detected_note(declared),
        stage.name,
    )))
}

/// The one precondition a projected mode carries: its preprocessing preserves a
/// projection, so the instance must declare the set to project onto.
///
/// Checked against [`CnfMeta::declared_show_vars`](crate::cnf::CnfMeta::declared_show_vars),
/// the set the chain will actually use, rather than the wider "does this file
/// ask for a projected count" that mode detection reads. The two come
/// apart on exactly one input: a `c t pmc`/`c t pwmc` header with no `c p show`
/// line beneath it, which asks for a projected count while declaring nothing to
/// project onto. That file is refused here, on whichever route chose the mode.
fn require_show_set(mode: crate::cnf::Mode, meta: &crate::cnf::CnfMeta) -> Result<(), VitriError> {
    if mode.is_projected() && meta.declared_show_vars().is_none() {
        return Err(VitriError::config(format!(
            "mode {} is projected, but the instance carries no `c p show` line — there is no \
             show set to preserve, so the mode is inert. Use {} for this file, or add \
             the show set",
            mode.token(),
            if mode.is_weighted() { "wmc" } else { "mc" },
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
