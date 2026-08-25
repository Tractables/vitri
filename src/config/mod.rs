//! [`RunConfig`] — the explicit, call-site-visible configuration for this
//! crate's public entry points.
//!
//! An environment variable can't carry a per-call budget for a library someone
//! else embeds: it's invisible at the call site and can't describe two
//! concurrent runs with different budgets.
//!
//! `budget_ms` is the only budget input on this path. Every site that scales a
//! sub-budget from it is handed [`RunConfig::effective_budget_ms`] as an
//! argument — on the construction side through the build limits
//! [`crate::component::build_vtree`] assembles — so a run's budget travels with
//! the run rather than through process state.

use std::time::{Duration, Instant};

use crate::error::VitriError;
use crate::preprocess::ArjunOptions;
use crate::spec::DEFAULT_VTREE_SPEC;

/// Whether a formula is split into its independent components before vtree
/// construction.
///
/// [`ComponentPolicy::token`] spells each variant as the `--components` flag
/// writes it and [`ComponentPolicy::parse`] reads one back, so an embedder
/// offering the flag does not have to restate the vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentPolicy {
    /// Split the formula into connected components, build a vtree per component
    /// under a pro-rata share of the budget, and graft them into one whole-formula
    /// vtree. The default: a smaller graph gives each component a better
    /// decomposition.
    ///
    /// The numbering-only baselines ([`crate::spec::baseline_spec_names`])
    /// ignore this — they gain nothing from a per-component graph — as do
    /// single-component formulas.
    Split,
    /// Build ONE vtree over the whole formula, whatever its component
    /// structure. Required when an externally supplied vtree must span the
    /// entire variable space, and available to a consumer that wants a single
    /// monolithic tree.
    Whole,
}

impl ComponentPolicy {
    /// Every policy, in the order a message or a `--help` line offers them.
    ///
    /// The vocabulary itself is [`Self::token`]'s match, which the compiler
    /// keeps exhaustive; this fixes the ORDER and is what [`Self::names`] and
    /// [`Self::parse`] read.
    const ALL: &'static [ComponentPolicy] = &[ComponentPolicy::Split, ComponentPolicy::Whole];

    /// The `--components` token naming this policy, the exact inverse of
    /// [`Self::parse`].
    pub fn token(self) -> &'static str {
        match self {
            ComponentPolicy::Split => "split",
            ComponentPolicy::Whole => "whole",
        }
    }

    /// Parses a `--components` token: any [`Self::names`] entry. The inverse of
    /// [`Self::token`] by construction — it is that spelling looked up.
    pub fn parse(token: &str) -> Option<Self> {
        ComponentPolicy::ALL
            .iter()
            .copied()
            .find(|p| p.token() == token)
    }

    /// Every `--components` token, in table order — for a shell over this crate
    /// that offers the vocabulary it will accept rather than keeping a copy.
    pub fn names() -> impl Iterator<Item = &'static str> {
        ComponentPolicy::ALL.iter().map(|p| p.token())
    }

    /// Whether one vtree must span the whole variable space rather than one per
    /// component.
    pub fn is_whole(self) -> bool {
        matches!(self, ComponentPolicy::Whole)
    }
}

/// Which preprocessing stages [`crate::bundle::preprocess`] runs.
///
/// Turning a stage off does not select a different code path — it configures the
/// one path to do nothing at that step (a no-op simplify configuration,
/// a skipped Arjun call), so the bundle's record stays exactly as truthful about
/// what ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreprocessStages {
    /// This crate's own simplify chain, whose stages `docs/preprocessing.md`
    /// lists in order.
    pub simplify: bool,
    /// Arjun. Always linked in; this switch is the only way to skip the stage.
    pub arjun: bool,
}

impl Default for PreprocessStages {
    /// Everything on — the production configuration.
    fn default() -> Self {
        PreprocessStages {
            simplify: true,
            arjun: true,
        }
    }
}

impl PreprocessStages {
    /// Which of these toggles preprocessing for `mode` actually reads. A `false`
    /// field names a stage that mode's chain does not have, so setting it either
    /// way changes nothing.
    ///
    /// The command line is the caller of this: a stage flag the resolved mode
    /// would ignore is refused there rather than accepted and dropped. The
    /// answer is the chain's, read through `Chain`, so which stages a mode
    /// reads and which chain runs it are one statement.
    #[must_use]
    pub fn read_under(mode: crate::cnf::Mode) -> Self {
        Chain::for_mode(mode).stages_read()
    }
}

/// Which preprocessing chain a mode runs.
///
/// [`crate::bundle::preprocess`] has three of them, and the five modes partition
/// across them. That partition decides two things — which chain the instance
/// goes down, and which stage toggles are live on the way — and this is where it
/// is stated, so a chain that gains or loses a stage cannot leave a refusal
/// message describing the chain it used to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Chain {
    /// Count-preserving: `mc` and `wmc`.
    Count,
    /// Projection-preserving: `pmc` and `pwmc`.
    Projection,
    /// Function-preserving: `compile`, which is not a counting track at all.
    Compile,
}

impl Chain {
    /// The chain `mode` runs. Exhaustive, so a new mode fails to compile until
    /// it names the chain that answers for it.
    pub(crate) fn for_mode(mode: crate::cnf::Mode) -> Self {
        use crate::cnf::Mode;
        match mode {
            Mode::Mc | Mode::Wmc => Chain::Count,
            Mode::Pmc | Mode::Pwmc => Chain::Projection,
            Mode::Compile => Chain::Compile,
        }
    }

    /// The stage toggles this chain reads. The struct literals are exhaustive,
    /// so a new stage fails to compile until every chain answers for it.
    pub(crate) fn stages_read(self) -> PreprocessStages {
        match self {
            Chain::Count => PreprocessStages {
                simplify: true,
                arjun: true,
            },
            // The projected chain is Arjun's projection-set minimization and the
            // show-frozen projected reduction, and nothing else: the simplify
            // chain's `2^k` lift charges ×2 for a variable a projection retires
            // at ×1, so it has no place there.
            Chain::Projection => PreprocessStages {
                simplify: false,
                arjun: true,
            },
            // Arjun eliminates on the strength of an independent support, and a
            // reconstruction entry names a literal rather than a function, so
            // `compile` runs the simplify chain alone.
            Chain::Compile => PreprocessStages {
                simplify: true,
                arjun: false,
            },
        }
    }
}

/// How much of the run's remaining wall vtree construction may spend, or — for
/// [`Self::Deterministic`] — how much WORK it may do instead.
///
/// Construction is one phase of a run. A caller that hands this crate a
/// whole-run deadline is asking it to leave room for the phases either side of
/// construction; a caller that has already carved a construction window out of
/// its own wall is not — it is naming the window. Those are different requests,
/// and this is where they are told apart.
///
/// Whichever policy is chosen, the bound is SOFT and by more than one step: the
/// portfolio consults it between candidates, and FlowCutter checks it between
/// restart iterations and before each of its two greedy pre-passes. Whatever is
/// in flight when the bound passes runs to completion. The bound decides what is
/// *started*, not what is interrupted.
///
/// `#[non_exhaustive]`: a run bounded by something other than the clock is a
/// policy this enum should be able to gain without breaking a caller that
/// matches on it.
///
/// ```
/// use vitri::RunConfig;
/// use vitri::config::ConstructionBudget;
///
/// // The work a ninety-second construction is calibrated to do, asked for as a
/// // wall and stored as the work it converts to.
/// let config = RunConfig {
///     construction_budget: ConstructionBudget::for_wall_ms(90_000),
///     ..RunConfig::default()
/// };
/// config.validate()?;
///
/// assert_eq!(
///     config.construction_budget,
///     ConstructionBudget::Deterministic {
///         units: 90_000 * ConstructionBudget::UNITS_PER_MS,
///     },
/// );
/// # Ok::<(), vitri::VitriError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ConstructionBudget {
    /// Construction gets a SHARE of what is left when it starts: a third of the
    /// remaining wall, clamped to between 90 s and 900 s, and never past the run
    /// deadline.
    ///
    /// The default, and what a whole-run caller wants. Preprocessing has already
    /// spent part of the budget by the time construction starts, and the phases
    /// after construction still need some of what is left.
    #[default]
    Share,

    /// Construction may spend the whole remaining wall, up to the run deadline
    /// and no further.
    ///
    /// For a caller that has ALREADY decided how much of its wall construction
    /// gets and is passing that instant as [`RunConfig::deadline`]. Such a
    /// caller wants the deadline honoured as given; under [`Self::Share`] it
    /// would be divided a second time.
    WholeRemaining,

    /// Construction stops at this instant, or at the run deadline, whichever is
    /// sooner. It can only ever be tighter than [`Self::WholeRemaining`].
    ///
    /// For a caller whose construction window is neither the run deadline nor a
    /// fixed share of it.
    Until(Instant),

    /// Construction spends a fixed amount of WORK rather than a fixed amount of
    /// time.
    ///
    /// It counts the graph work it does — one unit is about one graph-element
    /// touch: a neighbour entry scanned, a hyperedge pin visited, a
    /// decomposition restart run — and makes every stopping decision against
    /// that count instead of against a clock. Every construction backend charges
    /// on that one scale, so a budget divided between them divides work rather
    /// than one backend's private counter. Two runs over the same formula at the
    /// same `units` therefore consider the same candidates in the same order and
    /// select the same vtree, on any machine, under any load, and whatever
    /// another thread is building beside them — the count belongs to the
    /// construction that spends it. None of the three policies above can promise
    /// that: which candidates a loaded machine gets through is what decides the
    /// tree.
    ///
    /// It bounds CONSTRUCTION and nothing else. The preprocessing ahead of it is
    /// budgeted on the clock as before, so a reproducible run needs those stages
    /// turned off as well.
    ///
    /// The count replaces the wall for construction entirely. This is the one
    /// policy that does not consult [`RunConfig::deadline`] — a deadline
    /// anchored before preprocessing leaves construction a different amount of
    /// time on every run, which is the dependence this variant exists to remove,
    /// and it applies to a run that declared no deadline at all. Size `units`
    /// for the wall you are willing to give construction with
    /// [`Self::units_for_wall_ms`], and expect a few percent more than that:
    /// charges are deliberately pessimistic, so a build finishes inside its
    /// budget rather than past it.
    Deterministic {
        /// Work units construction may spend, in the unit
        /// [`ConstructionBudget::UNITS_PER_MS`] converts. Must be positive.
        units: u64,
    },
}

impl ConstructionBudget {
    /// Work units one millisecond of construction is calibrated at.
    ///
    /// A calibration constant, not a law: it was fitted by regressing charged
    /// work against measured milliseconds over a set of construction runs, so a
    /// machine faster or slower than that one does more or less real work per
    /// unit. Reproducibility does not depend on it — the same unit budget buys
    /// the same decisions everywhere — only the wall those decisions take does.
    pub const UNITS_PER_MS: u64 = crate::decompose::meter::UNITS_PER_MS;

    /// The work `ms` milliseconds of construction is calibrated to do.
    ///
    /// The conversion is [`Self::UNITS_PER_MS`], exposed so a caller keeps the
    /// choice of stating work or stating the wall it converts from rather than
    /// having it made for them.
    pub fn units_for_wall_ms(ms: u64) -> u64 {
        ms.saturating_mul(Self::UNITS_PER_MS)
    }

    /// [`Self::Deterministic`] sized for `ms` milliseconds of construction —
    /// [`Self::units_for_wall_ms`] and the variant in one call, which is how a
    /// caller converting an existing wall-clock budget usually wants it.
    pub fn for_wall_ms(ms: u64) -> Self {
        ConstructionBudget::Deterministic {
            units: Self::units_for_wall_ms(ms),
        }
    }
}

/// Configuration for a preprocess-and-build-a-vtree run.
///
/// `Default` is the production configuration: no budget limit,
/// [`DEFAULT_VTREE_SPEC`], every preprocessing stage on, per-component vtrees.
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

    /// Which preprocessing stages run before the vtree is built. All on by default;
    /// turning one off changes the formula the vtree is built over.
    pub stages: PreprocessStages,

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
            construction_budget: ConstructionBudget::default(),
            vtree_spec: DEFAULT_VTREE_SPEC.to_string(),
            stages: PreprocessStages::default(),
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
        // Only an EXPLICIT mode can be judged here; a detected one is not known
        // until the instance's headers have been read, and
        // `crate::bundle::preprocess` applies the same rule to it there.
        if let Some(mode) = self.mode {
            self.refuse_inert(mode)?;
        }
        if self.construction_budget == (ConstructionBudget::Deterministic { units: 0 }) {
            return Err(VitriError::config(
                "a deterministic construction budget of 0 work units leaves construction \
                 nothing to spend, so no vtree could be built — pass the work a construction \
                 should be allowed to do, which ConstructionBudget::for_wall_ms converts from \
                 a wall in milliseconds",
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
    /// Refuse every request this run makes that `mode` has no stage to answer:
    /// a stage switched OFF that `mode`'s preprocessing does not have, and a
    /// learnt-clause export no stage of it could fill. Each names what was
    /// asked for, the mode, and — when the mode was detected rather than
    /// declared — the fact that it was detected, so a user is not left looking
    /// for a `--mode` they never typed.
    ///
    /// `mode` is the mode that will actually run ([`Self::resolve_mode`]), which
    /// is the only point at which the declared and the detected route have the
    /// same answer. Asking for something the chain cannot do is a mistake in the
    /// request rather than a no-op, so it is refused before any budget is spent
    /// on the run.
    pub(crate) fn refuse_inert(&self, mode: crate::cnf::Mode) -> Result<(), VitriError> {
        let read = PreprocessStages::read_under(mode);
        for (off, reads, flag, stage) in [
            (
                !self.stages.simplify,
                read.simplify,
                "--no-simplify",
                "simplify",
            ),
            (!self.stages.arjun, read.arjun, "--no-arjun", "Arjun"),
        ] {
            if off && !reads {
                let how = if self.mode.is_some() {
                    String::new()
                } else {
                    " (detected from the instance's own headers — no --mode was given)".to_string()
                };
                return Err(VitriError::config(format!(
                    "{flag} does nothing under mode {}{how}: that mode's preprocessing has no \
                     {stage} stage to skip. Drop the flag, or run a mode whose preprocessing has one",
                    mode.token(),
                )));
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
        let declares_weights = meta.mode.is_weighted() || meta.declared_weights().is_some();
        // Detection asks a wider question than "does the file carry a show set":
        // a `c t pmc` header asks for a projected count even before the
        // `c p show` line that must accompany it is read.
        let declares_show = meta.mode.is_projected() || meta.declared_show_vars().is_some();
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

    /// The instant vtree construction will stop at, resolved against `now`: the
    /// run deadline of [`Self::resolved_deadline`] narrowed by
    /// [`Self::construction_budget`]. `None` when the run has no cutoff at all
    /// and none of its own — construction cannot be bounded by a share of
    /// nothing.
    ///
    /// This is the value construction enforces, not a second derivation of it,
    /// so a caller sizing its own downstream phases reads it here rather than
    /// recomputing the policy and hoping the two agree.
    ///
    /// Pass the instant construction starts at. Under
    /// [`ConstructionBudget::Share`] the answer depends on it: the share is of
    /// what is still left, so a run that spent most of its wall preprocessing
    /// gets a smaller construction window than the same run measured at its
    /// start.
    ///
    /// [`ConstructionBudget::Deterministic`] is the one policy that divides
    /// nothing: it names its own window in work, so it answers whether or not
    /// the run has a deadline, and never narrows to one. The instant it returns
    /// is on the construction meter's clock rather than the wall — which is what
    /// makes it a bound on work — so it is only meaningful while that meter is
    /// armed, and [`crate::component::build_vtree`] arms it at exactly this
    /// `now`.
    pub fn construction_deadline(&self, now: Instant) -> Option<Instant> {
        match self.construction_budget {
            ConstructionBudget::Deterministic { units } => {
                crate::budget::deterministic_deadline(units, now)
            }
            ConstructionBudget::Share => Some(crate::budget::vtree_share_deadline(
                self.resolved_deadline(now)?,
                now,
            )),
            ConstructionBudget::WholeRemaining => self.resolved_deadline(now),
            ConstructionBudget::Until(t) => Some(t.min(self.resolved_deadline(now)?)),
        }
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

/// The one precondition a projected mode carries: its preprocessing preserves a
/// projection, so the instance must declare the set to project onto.
///
/// Checked against [`CnfMeta::declared_show_vars`](crate::cnf::CnfMeta::declared_show_vars)
/// — the set the chain will actually use — rather than the wider "does this
/// file ask for a projected count" that mode detection reads. The two come
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
