//! In-process Arjun via the static C++ shim (`vendor/arjun/arjun_shim.{h,cpp}`).
//!
//! This is the only Arjun backend. It drives Arjun stage by stage in-process
//! and reads a sound checkpoint (reduced CNF + multiplier) off the
//! `SimplifiedCNF` after every stage, so a deadline hit between stages still
//! yields a usable partial reduction instead of nothing. Where the comments
//! below say "upstream Arjun" they mean the tool in `vendor/arjun/upstream/`,
//! whose reader semantics the shim reproduces.
//!
//! Budget enforcement, three layers, innermost first:
//!
//! 1. **In-process, inside Arjun** ([`ArjunLib::set_deadline`]). Arjun, CMS and
//!    CadiBack carry a local modification (applied to the source in `vendor/`)
//!    giving them a wall-clock deadline, checked between the steps of
//!    `elim_to_file`, at the top of the independent-support and extend loops,
//!    and at the existing budget-exhausted abort paths of the CMS oracle and
//!    the CadiBack backbone — both of which otherwise bound only *operations*,
//!    not time. A stage now returns at the deadline with a sound partial
//!    reduction instead of overrunning, landing a hair past it (the next poll
//!    site, plus read-back) — its own outcome class,
//!    [`BudgetClass::DeadlineCut`], kept, distinct from the uncontrolled
//!    [`BudgetClass::Overrun`] that is still discarded.
//! 2. **Between stages** (the [`budget::remaining`](crate::budget::remaining)
//!    checks below).
//! 3. **Out of process** (`fork_budget::run_forked_with_deadline`), which
//!    `SIGKILL`s the child at the deadline. A backstop: with layer 1 armed it
//!    should never fire, and it only covers what cooperative polling cannot —
//!    a genuine hang, or a code path with no poll site at all. It wraps the two
//!    UNPROJECTED entry points only: on the projected pair a result held past
//!    the deadline is the deliverable, so a kill there would be a behaviour
//!    change rather than an enforcement, and the one remaining overrun source
//!    is bounded by `PROJECTED_ORACLE_MAX_VARS_DEFAULT` instead.
//!    [`reduce_anytime_projected`] argues that in full.
//!
//! Soundness: Arjun's `get_multiplier_weight()` travels with the
//! `SimplifiedCNF`, so `(reduced clauses, multiplier)` is always a consistent
//! pair — neither is reconstructed separately and then matched up.

use crate::cnf::{CnfFormula, Literal, Reduced, ShowSet, Space, Weights};
use crate::diagnostics::diag;
use crate::error::VitriError;
use std::time::{Duration, Instant};

use super::arjun::{ArjunEffort, ArjunOptions, ArjunProjResult, ArjunResult};
use super::fork_budget::{ForkOutcome, run_forked_with_deadline};

mod budget_class;
mod shim;

use budget_class::keep_after_deadline;
pub(in crate::preprocess) use budget_class::keep_overrun_enabled;
use shim::{ArjunLib, validate_shim_env};

/// Default lite backbone/probing budget, in conflicts (Arjun's native unit for
/// `SimpConf::backbone_max_confl`). `-1` = Arjun's default (unlimited), so a
/// lite reduce that never overrides it is backbone-effort-identical to full.
///
/// A per-stage millisecond budget isn't plumbable at this layer — Arjun budgets
/// backbone/probing by conflicts, not time. This conflict cap is the closest
/// native knob for bounding backbone effort independently. Count-preserving:
/// bounds search effort only, never the count.
pub(super) const LITE_BACKBONE_MAX_CONFL_DEFAULT: i64 = -1;

/// Reference budget (ms) the full oracle (`oracle_mult = 1.0`) can burn in its
/// pathological worst case, before any scaling. Anchored to an observed ~30s
/// uninterruptible overrun on a small post-stage-1 formula. Arjun's oracle mems
/// budget scales linearly with `oracle_mult` (CryptoMiniSat `oracle_use.cpp`:
/// every pass budget is `const × oracle_mult`), so worst-case oracle budget ≈
/// `oracle_mult × ORACLE_FULL_WORSTCASE_MS`.
///
/// A conservative runaway-guard reference, not a tuned average: at
/// `remaining ≥ 30s` the oracle runs uncapped (`oracle_mult = 1.0`); scaling
/// only engages below that, to stop a pathological 30s-against-a-10s-budget
/// blow-up. A 2–3× worst-case overrun is acceptable (mems→budget is
/// instance-variable); a 30s×3 one is not.
pub(super) const ORACLE_FULL_WORSTCASE_MS: u128 = 30_000;

/// Floor for the scaled oracle effort. Purely a utility floor — soundness holds
/// at any value (smaller ⇒ fewer proven removals ⇒ larger-but-exact), so this
/// only stops the oracle being throttled to do essentially nothing. The 6000 ms
/// oracle pre-start gate means `remaining ≥ 6000` whenever this is evaluated,
/// so the floor is defensive (raw ≥ 0.2 there).
pub(super) const ORACLE_MULT_MIN: f64 = 0.05;

/// Size the heavy stage's `oracle_mult` from the budget still remaining
/// when the oracle is about to start. The oracle's worst-case budget scales
/// linearly with `oracle_mult`, so choosing `remaining / ORACLE_FULL_WORSTCASE_MS`
/// keeps that worst case near the remaining budget. Clamped to
/// `[ORACLE_MULT_MIN, 1.0]`: `remaining ≥ ORACLE_FULL_WORSTCASE_MS` ⇒ `1.0`
/// (uncapped, matches today exactly); less ⇒ proportionally smaller, floored.
/// Pure (no I/O, no env) so it is unit-testable in isolation.
pub(super) fn oracle_mult_for_budget(remaining_ms: u128) -> f64 {
    let raw = remaining_ms as f64 / ORACLE_FULL_WORSTCASE_MS as f64;
    raw.clamp(ORACLE_MULT_MIN, 1.0)
}

/// The default both projected pre-passes take for
/// [`OracleCaps::projected`](super::arjun::OracleCaps::projected) and its
/// weighted twin.
///
/// Both are single-lane and keep their checkpoint regardless of overrun, so on
/// a large formula an oracle overrun consumes the whole budget while the cheap
/// BVE/SBVA/autarky pipeline reaches the same reduction in a fraction of the
/// time. The cap skips the oracle on the class that overruns while keeping it
/// for small formulas, where it is cheap and cannot overrun.
pub(super) const PROJECTED_ORACLE_MAX_VARS_DEFAULT: u32 = 100_000;

/// What both `VITRI_*_ORACLE_MAX_VARS` knobs accept, in the words of whoever
/// sets one. Stated once, so the two knobs cannot come to describe themselves
/// differently.
pub(super) const ORACLE_MAX_VARS_FORM: &str =
    "a variable count, above which the reduce skips Arjun's oracle";

/// Read one projected pre-pass's oracle cap from its variable. The two differ
/// only in the name, so the default they fall back to and the form they accept
/// are settled here rather than at each call.
///
/// # Errors
///
/// [`VitriError::Env`] naming the variable.
pub(super) fn projected_oracle_max_vars(var: &'static str) -> Result<u32, VitriError> {
    crate::env::parse(var, PROJECTED_ORACLE_MAX_VARS_DEFAULT, ORACLE_MAX_VARS_FORM)
}

/// What `VITRI_ARJUN_EFFORT` accepts, quoted in both of its messages.
const ARJUN_EFFORT_FORMS: &str = "`full` (default) or `lite`";

/// Env-free parser for `VITRI_ARJUN_EFFORT` (kept pure so it is unit-testable
/// without touching the process environment). Absent ⇒ [`ArjunEffort::Full`]
/// (production default). Unknown value ⇒ `Err` naming the var and valid values.
pub(super) fn parse_arjun_effort(val: Option<&str>) -> Result<ArjunEffort, VitriError> {
    crate::env::from_forms(
        "VITRI_ARJUN_EFFORT",
        val,
        ArjunEffort::Full,
        &[("full", ArjunEffort::Full), ("lite", ArjunEffort::Lite)],
        ARJUN_EFFORT_FORMS,
    )
}

/// Reads `VITRI_ARJUN_EFFORT` into an [`ArjunEffort`]; absent ⇒ `Full`. A bad
/// value is a hard, fail-fast error naming the var and valid values.
///
/// # Errors
///
/// [`VitriError::Env`] naming `VITRI_ARJUN_EFFORT` and the valid values.
pub(crate) fn resolve_arjun_effort() -> Result<ArjunEffort, VitriError> {
    let raw = crate::env::env_raw("VITRI_ARJUN_EFFORT", ARJUN_EFFORT_FORMS)?;
    parse_arjun_effort(raw.as_deref())
}

/// What a give-up line says about the time behind it, which is as much as the
/// site reporting it knows: a read-back that produced nothing usable has no
/// duration to name, a stage that ran has an elapsed time, and one running to a
/// deadline also has the budget it was measured against.
pub(super) enum Spent {
    Unmeasured,
    Elapsed(Duration),
    ElapsedOfBudget(Duration, Duration),
}

/// The wording of a give-up line. Separate from [`giveup`] only so a test can
/// read it back; every emitter goes through `giveup`.
pub(super) fn giveup_line(label: &str, why: std::fmt::Arguments<'_>, spent: Spent) -> String {
    match spent {
        Spent::Unmeasured => format!("[{label}] give-up: {why}"),
        Spent::Elapsed(elapsed) => format!(
            "[{label}] give-up: {why} after {:.1}s",
            elapsed.as_secs_f64()
        ),
        Spent::ElapsedOfBudget(elapsed, budget) => format!(
            "[{label}] give-up: {why} after {:.1}s (budget {:.1}s)",
            elapsed.as_secs_f64(),
            budget.as_secs_f64()
        ),
    }
}

/// Report that a reduce path is abandoning its reduction, which returns its
/// caller to the unreduced formula. The one emitter of the `give-up:` line: the
/// sites that reach it — an unreadable multiplier or weight, a failed or
/// too-late stage, a discarded overrun, a killed or broken forked child — differ
/// only in what they know about the time spent, so they hand that in rather than
/// each wording the line themselves.
fn giveup(label: &str, why: std::fmt::Arguments<'_>, spent: Spent) {
    diag!("{}", giveup_line(label, why, spent));
}

/// The checkpoint's count multiplier, or `None` after reporting why it could
/// not be read. Every reduction is lifted by this multiplier, so a checkpoint
/// whose multiplier is unreadable is one no path may use: giving up returns the
/// caller to the unreduced formula, which still counts correctly.
fn multiplier_or_giveup(a: &ArjunLib) -> Option<String> {
    match a.cur_multiplier_decimal() {
        Ok(decimal) => Some(decimal),
        Err(e) => {
            giveup("arjun-anytime", format_args!("{e}"), Spent::Unmeasured);
            None
        }
    }
}

/// One literal's weight off the checkpoint, or `None` after reporting why it
/// could not be read. Substituting the default weight of 1 for an unreadable one
/// would carry a wrong weighted count all the way out, so an unreadable weight
/// abandons the reduction the same way an unreadable multiplier does.
fn lit_weight_or_giveup(a: &ArjunLib, lit: i32) -> Option<num_rational::BigRational> {
    let decimal = match a.lit_weight_decimal(lit) {
        Ok(decimal) => decimal,
        Err(e) => {
            giveup("arjun-anytime", format_args!("{e}"), Spent::Unmeasured);
            return None;
        }
    };
    match crate::cnf::parse_weight(decimal.trim()) {
        Ok(w) => Some(w),
        Err(e) => {
            giveup(
                "arjun-anytime",
                format_args!("literal {lit} weight {decimal:?} does not parse: {e}"),
                Spent::Unmeasured,
            );
            None
        }
    }
}

/// Convert Arjun's decimal multiplier to the exponent N in `2^N`. Returns `None`
/// if the multiplier is not an exact power of two (which it always is for
/// unweighted projected model counting — a non-power-of-two would signal a
/// weighted/unexpected reduction and must NOT be silently coerced).
pub(super) fn multiplier_decimal_to_exp(decimal: &str) -> Option<u32> {
    use num_bigint::BigUint;
    use num_traits::{One, Zero};
    let n: BigUint = decimal.trim().parse().ok()?;
    if n.is_zero() {
        return None;
    }
    let tz = n.trailing_zeros()?;
    if (BigUint::one() << tz) == n {
        u32::try_from(tz).ok()
    } else {
        None
    }
}

/// `VITRI_ARJUN_EXPORT_LEARNED_CLAUSES=1` (default off): harvest the
/// redundant/learnt clauses Arjun's internal solver derived during simplify.
/// Read in this one place, by
/// [`RunConfig::from_env_defaults`](crate::config::RunConfig::from_env_defaults),
/// which is what carries it to the reduce's `export_learned_clauses` argument —
/// so a caller that builds its own config decides the harvest itself and the
/// reduction below has one switch rather than two.
///
/// # Errors
///
/// [`VitriError::Env`] when the variable is set to neither an on nor an off
/// spelling.
pub(crate) fn export_learned_clauses_enabled() -> Result<bool, VitriError> {
    crate::env::env_flag("VITRI_ARJUN_EXPORT_LEARNED_CLAUSES")
}

/// Turn a fork-harness outcome into this module's `Option<T>` + give-up-line
/// contract; `label` is the log prefix of the calling reduce.
///
/// `Failed` (child panicked / died on a signal / delivered nothing decodable)
/// is reported and degraded to `None` — the caller's raw-formula fallback,
/// rather than taking the whole process down as a crash inside Arjun would
/// without the fork boundary.
fn finish_forked<T>(
    label: &str,
    outcome: ForkOutcome<Option<T>>,
    started: Instant,
    deadline: Instant,
) -> Option<T> {
    match outcome {
        ForkOutcome::Completed(r) => r,
        ForkOutcome::Killed { .. } => {
            giveup(
                label,
                format_args!("hard-killed at deadline"),
                Spent::ElapsedOfBudget(
                    started.elapsed(),
                    deadline.saturating_duration_since(started),
                ),
            );
            None
        }
        ForkOutcome::Failed(why) => {
            giveup(
                label,
                format_args!("forked arjun failed ({why})"),
                Spent::Elapsed(started.elapsed()),
            );
            None
        }
    }
}

// ── The shared stage skeleton ────────────────────────────────────────────────
//
// All four reduce paths — full count, projected, weighted, weighted projected —
// drive Arjun through [`run_stages`]; the axes they differ on are the fields of
// [`StageSpec`], so a change to the sequence lands on all four at once.
//
// Reading the checkpoint back is not shared: the four return different result
// types over different multiplier arithmetic, so each entry point does its own
// read-back off the handle this hands it.

/// Minimum budget (ms) that must remain when the heavy stage starts for its
/// oracle passes to run at all.
///
/// The oracle dominates that stage and every bound it carries counts
/// operations, not time, so starting it with too little runway spends every
/// remaining millisecond on a stage that is then killed or discarded, leaving
/// no checkpoint — whereas skipping it leaves the cheap pipeline's sound
/// reduction (BVE + SBVA + autarky, under a second). Instances that need the
/// oracle enter this stage with at least ~8 s of budget, while the ones it
/// merely starves enter with under ~5 s, so 6000 separates the two classes.
/// Count-preserving either way (the oracle only proves clause removals, so
/// skipping it yields a larger-but-exact reduction).
const ORACLE_MIN_RUNWAY_MS: u128 = 6000;

/// Which arithmetic the shim carries, and hence which constructor a reduction
/// uses: integer counts whose multiplier is a power of two, or exact rationals
/// with per-literal weights whose multiplier is a general rational.
#[derive(Clone, Copy)]
enum ShimField {
    /// [`ArjunLib::new`] — the unweighted counting field.
    Integer,
    /// [`ArjunLib::new_weighted`] — the FGenMpq rational field.
    Rational,
}

/// The sampling (independent-support / show) set a reduction hands Arjun.
///
/// The `all_indep` flag both stages are threaded with is a function of this
/// choice rather than a second knob: upstream Arjun's `read_in_a_file` sets
/// `all_indep` exactly when the input declares no `c p show` projection, and
/// threads that one value through both `minimize_indep` and `elim_to_file`.
/// Deriving it here stops the pair being set inconsistently.
enum Sampling<'a, S: Space> {
    /// Every variable, listed explicitly — an unprojected integer count.
    AllVarsListed,
    /// Every variable, through Arjun's own `clean_sampl`, which fills the
    /// sampling AND opt-sampling sets. This is what an unprojected WEIGHTED
    /// count needs: it makes an eliminated variable's mass fold into the
    /// multiplier instead of collapsing the multiplier to 1.
    AllVarsCleaned,
    /// A declared show set, over the space `S` the fed formula is written in.
    Projection(&'a ShowSet<S>),
}

impl<S: Space> Sampling<'_, S> {
    /// The `all_indep` value that travels with this sampling set: true exactly
    /// when there is no projection.
    fn all_indep(&self) -> bool {
        !matches!(self, Sampling::Projection(_))
    }

    /// Declare the set on `a`. `num_vars` is the fed formula's variable count,
    /// needed only to spell out the all-variables list.
    fn apply(&self, a: &mut ArjunLib, num_vars: u32) {
        match self {
            Sampling::AllVarsListed => {
                let all: Vec<u32> = (0..num_vars).collect();
                a.set_sampl(&all);
            }
            Sampling::AllVarsCleaned => a.clean_sampl(),
            Sampling::Projection(show) => a.set_sampl(show.as_zero_based()),
        }
    }
}

/// How a reduction gates the heavy stage's oracle passes — the uninterruptible
/// work that dominates that stage.
#[derive(Clone, Copy)]
enum Oracle {
    /// Off: the passes do not run and `oracle_mult` is inert.
    Off,
    /// On when the formula is small enough AND enough budget remains.
    Gated {
        /// Variable count above which the oracle is skipped; `u32::MAX` = no
        /// size gate. Each path resolves its own — see
        /// [`FULLCOUNT_ORACLE_MAX_VARS`] and [`PROJECTED_ORACLE_MAX_VARS_DEFAULT`].
        max_vars: u32,
        /// Whether `oracle_mult` is sized from the budget still remaining when the
        /// oracle starts, bounding its worst case near that budget
        /// ([`oracle_mult_for_budget`]).
        scale_mult: bool,
    },
}

/// What a reduction does with a checkpoint that arrives past its deadline.
#[derive(Clone, Copy)]
enum PastDeadline {
    /// Keep it, however late. The single-lane projected pre-passes: the
    /// checkpoint is the deliverable there, so discarding it would drop the
    /// caller to the raw projected path — a behavior change, not an enforcement.
    Keep,
    /// Run the shared acceptance policy ([`keep_after_deadline`]): in-budget and
    /// deadline-cut returns are kept, an uncontrolled overrun is discarded
    /// unless `keep_overrun`.
    Classify {
        /// Hand back an overrun checkpoint instead of discarding it.
        keep_overrun: bool,
    },
}

/// Everything a reduce path chooses about how the stages run. One value per
/// entry point, built at the top of its `*_inner`, so every difference between
/// the four paths is visible in a single literal instead of scattered through a
/// shared body as branches.
struct StageSpec<'a, S: Space> {
    /// Prefix for this path's diagnostic lines.
    label: &'static str,
    /// Whether the per-stage give-up lines are reported at all — a per-path
    /// choice, since some existing callers are silent about a stage that
    /// simply did not fit and reporting there would change what they see.
    report_giveups: bool,
    /// Integer or rational arithmetic — picks the shim constructor.
    field: ShimField,
    /// Seed for Arjun's own randomization — see
    /// [`ArjunOptions::seed`](super::arjun::ArjunOptions::seed).
    seed: u32,
    /// The sampling set, which also fixes `all_indep`.
    sampling: Sampling<'a, S>,
    /// Per-literal weights to ingest, as `(signed DIMACS literal, weight)`;
    /// empty on an integer path. Only sampling-set variables' weights are
    /// ingested — see [`run_stages`] for why a projected variable's weight must
    /// not reach the shim.
    weights: &'a [(i32, num_rational::BigRational)],
    /// Conflict cap for the heavy stage's Puura backbone/probing, or `None` to
    /// leave Arjun's own (unlimited) default in place.
    backbone_max_confl: Option<i64>,
    /// The heavy stage's oracle gate.
    oracle: Oracle,
    /// Disable SBVA in the heavy stage (count-preserving).
    no_sbva: bool,
    /// Disable BVE in the heavy stage (count-preserving).
    no_bve: bool,
    /// The budget this reduction runs against, absolute.
    deadline: Instant,
    /// What to do with a checkpoint that arrives past `deadline`.
    past_deadline: PastDeadline,
}

impl<S: Space> StageSpec<'_, S> {
    /// Report a give-up, if this path reports at all.
    fn giveup(&self, started: Instant, why: &str) {
        if self.report_giveups {
            giveup(
                self.label,
                format_args!("{why}"),
                Spent::Elapsed(started.elapsed()),
            );
        }
    }

    /// Report that the heavy stage reported failure. Not a give-up: the
    /// stage-1 checkpoint stands and the reduction goes on, so this is the one
    /// line saying the heavy stage was asked for and did not happen — a
    /// `VITRI_ARJUN_*` value the shim refuses reaches the caller this way and
    /// no other. Silent on the paths that report no stage lines at all.
    fn note_heavy_stage_failed(&self) {
        if self.report_giveups {
            diag!(
                "[{}] heavy stage failed; keeping the stage-1 reduction",
                self.label
            );
        }
    }

    /// The same, also naming the budget the path was working against.
    fn giveup_vs_budget(&self, started: Instant, why: &str) {
        if self.report_giveups {
            giveup(
                self.label,
                format_args!("{why}"),
                Spent::ElapsedOfBudget(
                    started.elapsed(),
                    self.deadline.saturating_duration_since(started),
                ),
            );
        }
    }
}

/// A completed run of the stages: the shim holding the most-reduced sound
/// checkpoint, when the stages started, and whatever the caller harvested
/// between the two stages.
struct StagedArjun<T> {
    /// The handle to read the checkpoint off. Every getter reads the one
    /// `s->cur`, so formula, sampling set, weights and multiplier are always a
    /// consistent tuple.
    shim: ArjunLib,
    /// When the stages started.
    started: Instant,
    /// The `after_minimize` closure's result.
    harvest: T,
}

/// Drive Arjun's two stages over `formula` per `spec`, and hand back the shim
/// holding the resulting checkpoint. `None` when there is nothing to hand back:
/// the shim could not be constructed, no budget remained for even the cheap
/// stage, that stage failed, or the checkpoint arrived too late for this path's
/// [`PastDeadline`] policy.
///
/// `after_minimize` runs between the two stages, the only point at which the
/// input variable space is still intact — the heavy stage renumbers. A path
/// with nothing to harvest there passes a closure returning `()`.
///
/// Weight ingestion follows one rule for every path: a weight is fed to the
/// shim only when its variable is in the sampling set. For the two
/// all-variables samplings that is every weight; for a projection it excludes
/// the projected-out variables — a soundness step, not an optimization: a
/// projected variable is existentially forgotten (weight 1), and letting Arjun
/// fold its mass into the multiplier when it eliminates the variable poisons
/// the count.
fn run_stages<T, S: Space>(
    formula: &CnfFormula,
    spec: &StageSpec<'_, S>,
    after_minimize: impl FnOnce(&ArjunLib) -> T,
) -> Option<StagedArjun<T>> {
    // A declared projection naming nothing is not a reduction any path can run;
    // the caller is expected to pass the instance's own show set.
    if matches!(&spec.sampling, Sampling::Projection(show) if show.is_empty()) {
        return None;
    }
    let started = Instant::now();
    let shim = match spec.field {
        ShimField::Integer => ArjunLib::new(spec.seed),
        ShimField::Rational => ArjunLib::new_weighted(spec.seed),
    };
    let mut a = match shim {
        Some(a) => a,
        None => {
            spec.giveup(started, "shim ctor failed (null)");
            return None;
        }
    };
    // Arm Arjun's own budget deadline once, before stage 1, so it covers both
    // stages — this is what turns the between-stage checks below from "don't
    // start a stage we can't finish" into a real bound: a stage that would
    // have overrun now returns at the deadline with its partial, sound
    // checkpoint.
    a.set_deadline(spec.deadline);
    if let Some(max_confl) = spec.backbone_max_confl {
        a.set_backbone_max_confl(max_confl);
    }
    a.new_vars(formula.num_vars);

    // Feed clauses as DIMACS (1-based, signed).
    let mut scratch: Vec<i32> = Vec::new();
    for cl in &formula.clauses {
        scratch.clear();
        for l in &cl.literals {
            scratch.push(l.to_dimacs());
        }
        a.add_clause_dimacs(&scratch);
    }

    // Per-literal weights, both polarities explicit, exactly as upstream Arjun
    // writes `c p weight <lit> <num>/<den> 0` lines, for sampling-set variables
    // only. Formatted as `num/den` so the field parser sees an exact rational
    // regardless of value.
    if !spec.weights.is_empty() {
        let projection = match &spec.sampling {
            Sampling::Projection(show) => Some(*show),
            // Every variable is in the sampling set, so no filter is needed.
            Sampling::AllVarsListed | Sampling::AllVarsCleaned => None,
        };
        for (lit, w) in spec.weights {
            let lit = Literal::from(*lit);
            if projection.is_some_and(|show| !show.contains(lit.var)) {
                continue;
            }
            if let Err(e) = a.set_lit_weight(lit, &format!("{}/{}", w.numer(), w.denom())) {
                spec.giveup(started, &format!("{e}"));
                return None;
            }
        }
    }

    spec.sampling.apply(&mut a, formula.num_vars);
    let all_indep = spec.sampling.all_indep();

    // Stage 1 (cheap). With no budget for even the minimize there is no
    // checkpoint better than raw, so the caller takes its own raw path.
    if crate::budget::remaining(spec.deadline).is_zero() {
        spec.giveup_vs_budget(started, "deadline passed before stage-1");
        return None;
    }
    if !a.stage_minimize_indep(all_indep) {
        spec.giveup(started, "stage-1 minimize failed");
        return None;
    }
    let harvest = after_minimize(&a);

    // Stage 2 (heavy: the full `elim_to_file` pipeline) only if there is still
    // time. Failure leaves the stage-1 checkpoint intact, which is still a sound
    // reduction, so the run continues — with a line saying the heavy stage did
    // not happen, since the reduction the caller gets is the weaker one.
    let left = crate::budget::remaining(spec.deadline);
    if !left.is_zero() {
        let remaining_ms = left.as_millis();
        let oracle = match spec.oracle {
            Oracle::Off => false,
            Oracle::Gated {
                max_vars,
                scale_mult,
            } => {
                let on = formula.num_vars <= max_vars && remaining_ms >= ORACLE_MIN_RUNWAY_MS;
                // Bound the oracle's actual SAT work when it runs. The runway
                // gate is a coarse go/no-go; it cannot stop an oracle that
                // passes it from then blowing tens of seconds uninterruptibly on
                // a small-but-hard formula. Sizing `oracle_mult` from the budget
                // remaining right now caps that worst case near it (linear
                // scaling, count-preserving at any value), yielding 1.0 —
                // Arjun's own uncapped behavior — at a long enough runway.
                if on && scale_mult {
                    a.set_oracle_mult(oracle_mult_for_budget(remaining_ms));
                }
                on
            }
        };
        if !a.stage_simplify(all_indep, oracle, spec.no_sbva, spec.no_bve) {
            spec.note_heavy_stage_failed();
        }
    }

    if let PastDeadline::Classify { keep_overrun } = spec.past_deadline
        && !keep_after_deadline(
            spec.label,
            Instant::now(),
            started,
            spec.deadline,
            a.deadline_armed(),
            keep_overrun,
        )
    {
        return None;
    }

    Some(StagedArjun {
        shim: a,
        started,
        harvest,
    })
}

/// Run Arjun in-process on `formula`, checking `deadline` between stages, and
/// return the most-reduced sound checkpoint as an [`ArjunResult`]. Returns `None`
/// if even the first (cheap) stage fails or the multiplier isn't a power of two.
///
/// `force_no_sbva`: disable SBVA in the heavy simplify stage for this call — the
/// revert target for a caller whose previous, SBVA-reduced formula blew up
/// downstream; `false` is the ordinary path, where the formula's own structure
/// still decides (the size-based skip condition). The caller resolves
/// `VITRI_ARJUN_SBVA` ([`crate::decompose::arjun_sbva_skip`]) and ORs its own
/// transient revert in before calling, so there is one flag here rather than a
/// second policy; the shim reads no environment of its own.
///
/// `deadline` is hard: the reduction runs in a forked child that is `SIGKILL`ed
/// once the deadline (plus the harness's small serialization grace) passes, so a
/// stage that overruns can no longer eat the caller's whole budget. Everything
/// the caller needs (reduced formula, multiplier, and the input-space
/// backbone/equivalence harvest that seeds the raw fallback) travels back
/// through the serialized [`ArjunResult`].
///
/// The kill costs nothing that would have been kept in the overrun class (it is
/// discarded below in any case). It does bound the deadline-cut class: a cut is
/// kept only if the child serialized it back before
/// `deadline + fork_budget::KILL_GRACE` — past that the parent kills and the
/// caller sees the hard-kill give-up line instead. See [`DEADLINE_CUT_GRACE`]
/// for why the two bounds are deliberately separate.
///
/// # Errors
///
/// [`VitriError::Env`] for a `VITRI_*` variable this path reads. A reduction
/// that fails or does not converge is not an error: it comes back as
/// `Ok(None)`.
pub(super) fn reduce_anytime(
    formula: &CnfFormula,
    deadline: Instant,
    arjun: ArjunOptions,
    force_no_sbva: bool,
) -> Result<Option<ArjunResult>, VitriError> {
    // The shim's own variables are checked here, in the parent, before any
    // reduction work starts: raised inside the forked child, a value this crate
    // cannot use would come back as nothing more than a failed reduction.
    validate_shim_env()?;
    // Keeping an overrun is precisely about keeping a reduction that finished
    // past its budget, so hard-killing at the deadline would delete the only
    // thing that path produces. Run it inline instead.
    if arjun.keep_overrun {
        return Ok(reduce_anytime_inner(
            formula,
            deadline,
            arjun,
            force_no_sbva,
        ));
    }
    let started = Instant::now();
    let outcome = run_forked_with_deadline(deadline, || {
        reduce_anytime_inner(formula, deadline, arjun, force_no_sbva)
    });
    Ok(finish_forked("arjun-anytime", outcome, started, deadline))
}

/// The reduction itself — the one implementation, run either in the forked
/// child ([`reduce_anytime`], the default) or inline (the keep-overrun debug
/// path). It states the full-count path's [`StageSpec`], hands it to
/// [`run_stages`], and reads the checkpoint back as an [`ArjunResult`]. Every
/// deadline check, oracle gate and overrun-discard rule in that spec is
/// unchanged by the fork: they stop a stage being *started* too late, while the
/// parent's `SIGKILL` bounds a stage that is already running.
pub(super) fn reduce_anytime_inner(
    formula: &CnfFormula,
    deadline: Instant,
    arjun: ArjunOptions,
    // The caller's whole no-SBVA decision, carried through unchanged.
    no_sbva_call: bool,
) -> Option<ArjunResult> {
    // The config selects the heavy stage's shape:
    //   Full — oracle budget-gated, SBVA on unless the caller's already-resolved
    //     no-SBVA decision says otherwise, BVE on.
    //   Lite — raw-equivalent (BCP + backbone/probing + equivalent-literal
    //     substitution): no SBVA, no BVE, oracle off (heavier simplification
    //     than the lite contract allows), plus its own conflict cap on the
    //     backbone/probing effort.
    // Those three shim-exposed heavy knobs are all the lite contract needs. The
    // remaining `elim_to_file` stages (extend-indep, autarky, renumber; BCE off
    // by default) stay on in both arms — cheap, count-preserving, and the shim
    // exposes no per-stage disable for them. Both arms keep the reduced CNF +
    // strictly-`2^N` multiplier contract; lite is count-preserving, only larger.
    let (oracle, no_sbva, no_bve, backbone_max_confl) = match arjun.effort {
        ArjunEffort::Full => (
            Oracle::Gated {
                max_vars: arjun.oracle_max_vars.plain.unwrap_or(u32::MAX),
                scale_mult: true,
            },
            no_sbva_call,
            false,
            None,
        ),
        ArjunEffort::Lite => (
            Oracle::Off,
            true,
            true,
            Some(LITE_BACKBONE_MAX_CONFL_DEFAULT),
        ),
    };
    // This path counts over every variable, which is exactly when upstream
    // Arjun sets `all_indep`; there is no projection, so the space marker only
    // has to name one.
    let spec = StageSpec::<Reduced> {
        label: "arjun-anytime",
        report_giveups: true,
        field: ShimField::Integer,
        seed: arjun.seed,
        sampling: Sampling::AllVarsListed,
        weights: &[],
        backbone_max_confl,
        oracle,
        no_sbva,
        no_bve,
        deadline,
        // Past-deadline returns split into two classes (see [`classify_budget`]).
        // Overrun is discarded by default — see [`keep_overrun_enabled`] for why.
        // `VITRI_ARJUN_KEEP_OVERRUN` opts in here and also opts out of the fork
        // in [`reduce_anytime`], since a hard kill would delete the very result
        // the knob exists to keep. Deadline-cut is the opposite trade and is
        // kept unconditionally.
        past_deadline: PastDeadline::Classify {
            keep_overrun: arjun.keep_overrun,
        },
    };

    // Harvest backbone + equivalences in the INPUT var space, between the two
    // stages — before the heavy simplify renumbers and eliminates. They let a
    // caller working on the un-Arjun'd input formula seed it with constraints
    // it would otherwise never see; an empty budget or a failed simplify still
    // keeps them, since they describe the input, not the reduced output.
    let StagedArjun {
        shim: a,
        started,
        harvest: (backbone, equiv),
    } = run_stages(formula, &spec, |a| (a.backbone(), a.eq_lits()))?;
    let multiplier_exp = match multiplier_decimal_to_exp(&multiplier_or_giveup(&a)?) {
        Some(e) => e,
        None => {
            giveup(
                "arjun-anytime",
                format_args!("multiplier not a power of two"),
                Spent::Elapsed(started.elapsed()),
            );
            return None;
        }
    };
    let full_formula = a.cur_formula();
    // Harvest the redundant/learnt clauses Arjun's internal solver derived (gated
    // — off by default). They come back in the REDUCED numbering (same var space
    // as `full_formula`), so we keep only clauses all of whose vars survived into
    // the reduced formula (index < num_vars); any clause mentioning an eliminated
    // var is dropped.
    let learnt_clauses: Vec<Vec<i32>> = if arjun.export_learned_clauses {
        let nv = full_formula.num_vars;
        a.red_clauses()
            .into_iter()
            .filter(|cl| {
                !cl.is_empty() && cl.iter().all(|&l| l.unsigned_abs().saturating_sub(1) < nv)
            })
            .collect()
    } else {
        Vec::new()
    };
    // Reads the same `s->cur` checkpoint as `full_formula`, so the two always
    // share one renumbering. Harvested after the stages, since the heavy stage
    // is what renumbers.
    let input_to_reduced_lit = a.orig_to_new_lits(formula.num_vars);
    Some(ArjunResult {
        formula: full_formula,
        multiplier_exp,
        backbone,
        equiv,
        learnt_clauses,
        input_to_reduced_lit,
    })
}

/// Projected analogue of [`reduce_anytime`]: drive Arjun's *projection-set*
/// minimization in-process and return the most-reduced sound checkpoint as an
/// [`ArjunProjResult`]. Every getter reads the one `s->cur` checkpoint, so a
/// non-convergent run still hands back its best partial reduction instead of
/// nothing — which is why this pre-pass can spend a large slice of the budget
/// without risking the whole window.
///
/// Soundness: `cur_formula()`, `cur_sampl()` and `cur_multiplier_decimal()` all
/// read the same `s->cur` SimplifiedCNF, so `(reduced formula, reduced show,
/// multiplier)` is a consistent triple at every checkpoint — the renumber in
/// `elim_to_file` rewrites formula and sampl in lock-step. Therefore
/// `count(reduced, reduced_show) << multiplier_exp == count(orig, show)` holds
/// whether stage 1 or stage 2 is where we stop (asserted by
/// `reduce_anytime_projected_soundness`).
///
/// Unlike [`reduce_anytime`], where an overrun is doomed and discarded, keeping
/// the checkpoint past the deadline is the entire point here, so there is no
/// discard-on-overrun. The oracle is still budget-gated (the uninterruptible
/// overrun source), bounding a non-convergent run's overrun to the cheap
/// pipeline.
///
/// This is also why the path is deliberately not wrapped in the hard-kill fork
/// harness [`reduce_anytime`] uses: the fork is free there only because an
/// overrun result is discarded anyway, but here the overrun result is the
/// deliverable — a `SIGKILL` at the deadline would drop the caller to the raw
/// projected path, a behavior change, not an enforcement. The remaining overrun
/// source, the oracle on large formulas, is bounded by
/// `PROJECTED_ORACLE_MAX_VARS_DEFAULT` instead. Same reasoning applies verbatim
/// to [`reduce_anytime_weighted_projected`].
///
/// # Errors
///
/// [`VitriError::Env`] for a `VITRI_*` variable this path reads. A reduction
/// that simply does not converge inside `deadline` is not an error: it comes
/// back as `Ok(None)`.
pub(super) fn reduce_anytime_projected<S: Space>(
    formula: &CnfFormula,
    show: &ShowSet<S>,
    deadline: Instant,
    arjun: ArjunOptions,
    force_no_sbva: bool,
) -> Result<Option<ArjunProjResult>, VitriError> {
    // Checked before any reduction work starts, so a value the run cannot use
    // surfaces to the caller rather than being spent against.
    validate_shim_env()?;
    Ok(reduce_anytime_projected_inner(
        formula,
        show,
        deadline,
        arjun,
        force_no_sbva,
    ))
}

fn reduce_anytime_projected_inner<S: Space>(
    formula: &CnfFormula,
    show: &ShowSet<S>,
    deadline: Instant,
    arjun: ArjunOptions,
    no_sbva_call: bool,
) -> Option<ArjunProjResult> {
    let spec = StageSpec {
        label: "arjun-anytime-pmc",
        report_giveups: false,
        field: ShimField::Integer,
        seed: arjun.seed,
        // There is a `c p show` projection, so `all_indep` is false — mirroring
        // upstream Arjun's read path, which threads that value through both
        // stages whenever a show set is present.
        sampling: Sampling::Projection(show),
        weights: &[],
        backbone_max_confl: None,
        // The oracle is the uninterruptible overrun source, and unlike the
        // full-count path this single lane keeps its checkpoint regardless of
        // overrun, so an overrun eats the caller's budget directly. Own knob, and
        // the projected default shared with the weighted projected path.
        oracle: Oracle::Gated {
            max_vars: arjun.oracle_max_vars.projected.unwrap_or(u32::MAX),
            scale_mult: true,
        },
        no_sbva: no_sbva_call,
        no_bve: false,
        deadline,
        // Keep the checkpoint regardless of overrun — the anytime value for a
        // single-lane projected pre-pass. Arjun's own budget deadline is what
        // bounds the overrun here, since there is no discard and no hard-kill
        // fork to fall back on.
        past_deadline: PastDeadline::Keep,
    };
    let StagedArjun { shim: a, .. } = run_stages(formula, &spec, |_| ())?;

    // Read formula + show + multiplier off the one checkpoint so the triple is
    // consistent.
    let multiplier_exp = multiplier_decimal_to_exp(&multiplier_or_giveup(&a)?)?;
    let reduced = a.cur_formula();
    let reduced_show = ShowSet::from_zero_based(a.cur_sampl());
    // Same `s->cur` checkpoint as everything above, so the map is consistent with
    // the (formula, show, multiplier) triple rather than describing a different
    // stage's numbering.
    let input_to_reduced_lit = a.orig_to_new_lits(formula.num_vars);
    Some(ArjunProjResult {
        formula: reduced,
        show: reduced_show,
        multiplier_exp,
        input_to_reduced_lit,
    })
}

/// In-process weighted Arjun reduction (the WMC analogue of
/// [`reduce_anytime`]), equivalent to upstream Arjun's `--mode 1` on a `c t wmc`
/// input with no `c p show`. Builds a weighted (FGenMpq) SimplifiedCNF, ingests
/// the per-literal weights, runs the same two stages as the integer path with
/// `all_indep=true`, and reads back the reduced formula, reduced per-literal
/// weights, and the rational multiplier K.
///
/// Soundness: the rational multiplier travels with the SimplifiedCNF, and
/// `all_indep=true` + `clean_sampl` ensures eliminated weighted mass folds into
/// K rather than being projected away. Declaring a `c p show` here would
/// collapse K to 1, which is why this path declares none.
///
/// Returns `None` if the cheap stage fails or the deadline is blown mid-stage;
/// the caller then compiles the raw formula.
///
/// Like [`reduce_anytime`] — and for the same reason, its overrun result is
/// discarded unconditionally — the reduction runs in a hard-killed forked
/// child, so `deadline` bounds the budget even when a stage never returns. (The
/// two projected pre-passes are excluded: they keep their overrun checkpoint;
/// see [`reduce_anytime_projected`].)
///
/// # Errors
///
/// [`VitriError::Env`] naming a `VITRI_*` variable set to a value this path
/// cannot use. A reduction that does not fit the budget is not an error: it
/// comes back as `Ok(None)`.
pub(super) fn reduce_anytime_weighted(
    formula: &CnfFormula,
    weights: &[(i32, num_rational::BigRational)],
    deadline: Instant,
    arjun: ArjunOptions,
    no_sbva: bool,
) -> Result<Option<super::arjun::ArjunWeightedResult>, VitriError> {
    // Same ordering as every other reduce: check what the shim reads first, and
    // in the parent, since the reduction itself runs in a forked child.
    validate_shim_env()?;
    let started = Instant::now();
    let outcome = run_forked_with_deadline(deadline, || {
        reduce_anytime_weighted_inner(formula, weights, deadline, arjun, no_sbva)
    });
    Ok(finish_forked(
        "arjun-anytime-wmc",
        outcome,
        started,
        deadline,
    ))
}

/// The weighted reduction itself — the one implementation, run in the forked
/// child of [`reduce_anytime_weighted`].
fn reduce_anytime_weighted_inner(
    formula: &CnfFormula,
    weights: &[(i32, num_rational::BigRational)],
    deadline: Instant,
    arjun: ArjunOptions,
    // Resolved by the caller — see [`reduce_anytime_weighted`].
    no_sbva: bool,
) -> Option<super::arjun::ArjunWeightedResult> {
    use num_rational::BigRational;

    // No projection on this path either, so the space marker only has to name
    // one.
    let spec = StageSpec::<Reduced> {
        label: "arjun-anytime-wmc",
        report_giveups: false,
        field: ShimField::Rational,
        seed: arjun.seed,
        // No `c p show` ⇒ the clean (all-variables) sampling set and
        // `all_indep = true`. This is the crux: it makes eliminated/defined mass
        // fold into the multiplier K rather than collapse K to 1.
        sampling: Sampling::AllVarsCleaned,
        weights,
        backbone_max_confl: None,
        // Runway gate only — no size gate, no remaining-budget `oracle_mult`
        // sizing (unlike the other three paths): each is a measured trade on
        // the path that adopted it, and neither has been measured on the
        // weighted full-count reduction.
        oracle: Oracle::Gated {
            max_vars: arjun.oracle_max_vars.plain.unwrap_or(u32::MAX),
            scale_mult: false,
        },
        no_sbva,
        no_bve: false,
        deadline,
        // The same three-way classification as the integer path, through the
        // one shared policy. `keep_overrun = false`: this path's overrun discard
        // is unconditional (freeze-lane semantics — `VITRI_ARJUN_KEEP_OVERRUN`
        // is not extended to it), so a blown deadline routes the caller straight
        // to the freeze lane, while a deadline-cut is kept on the same terms as
        // the integer path.
        past_deadline: PastDeadline::Classify {
            keep_overrun: false,
        },
    };
    let StagedArjun { shim: a, .. } = run_stages(formula, &spec, |_| ())?;
    let multiplier: BigRational =
        crate::cnf::parse_weight(multiplier_or_giveup(&a)?.trim()).ok()?;
    let full_formula = a.cur_formula();
    let reduced_weights =
        Weights::try_from_dimacs_lits(full_formula.num_vars, |l| lit_weight_or_giveup(&a, l))?;

    let input_to_reduced_lit = a.orig_to_new_lits(formula.num_vars);
    Some(super::arjun::ArjunWeightedResult {
        formula: full_formula,
        weights: reduced_weights,
        multiplier,
        input_to_reduced_lit,
    })
}

/// Weighted-**projected** analogue of [`reduce_anytime_projected`]. Drives
/// Arjun's projection-set minimization on a *weighted* SimplifiedCNF and returns
/// the most-reduced sound checkpoint `(formula, show, weights, K)`. Like the
/// integer projected pre-pass this is a single-lane pre-pass that keeps the
/// checkpoint past the deadline (the whole anytime value), so — like
/// [`reduce_anytime_projected`] — it is deliberately not run under the
/// hard-kill fork harness (killing it would delete the deliverable).
///
/// Soundness: `cur_formula()`, `cur_sampl()`, `cur_multiplier_decimal()` and
/// `lit_weight_decimal()` all read the same `s->cur` SimplifiedCNF, so
/// `(reduced formula, reduced show, reduced weights, K)` is a consistent
/// quadruple at every checkpoint: `PWMC(orig, show, weights) ==
/// PWMC(reduced, reduced_show, reduced_weights) * K`. The defined-var fold
/// below is required because Arjun's weighted mode moves a *defined* show var
/// out of the show set while keeping its weight, and this bundle's
/// show-vars-only weight convention means every weight-carrying var must be
/// folded back into show — identified as a reduced var whose stored weight is
/// non-default (≠ 1 on either polarity), exactly the vars Arjun writes
/// `c p weight` lines for.
///
/// # Errors
///
/// [`VitriError::Env`] for a `VITRI_*` variable this path reads.
/// Non-convergence inside `deadline` is not an error: it comes back as
/// `Ok(None)`.
pub(super) fn reduce_anytime_weighted_projected<S: Space>(
    formula: &CnfFormula,
    show: &ShowSet<S>,
    weights: &[(i32, num_rational::BigRational)],
    deadline: Instant,
    arjun: ArjunOptions,
    force_no_sbva: bool,
) -> Result<Option<super::arjun::ArjunWeightedProjResult>, VitriError> {
    validate_shim_env()?;
    Ok(reduce_anytime_weighted_projected_inner(
        formula,
        show,
        weights,
        deadline,
        arjun,
        force_no_sbva,
    ))
}

fn reduce_anytime_weighted_projected_inner<S: Space>(
    formula: &CnfFormula,
    show: &ShowSet<S>,
    weights: &[(i32, num_rational::BigRational)],
    deadline: Instant,
    arjun: ArjunOptions,
    no_sbva_call: bool,
) -> Option<super::arjun::ArjunWeightedProjResult> {
    use num_rational::BigRational;

    let spec = StageSpec {
        label: "arjun-anytime-pwmc",
        report_giveups: false,
        field: ShimField::Rational,
        seed: arjun.seed,
        // There is a `c p show` projection ⇒ the declared sampling set and
        // `all_indep = false`. The crux difference from the full-WMC reduce,
        // which uses the clean sampling set and `all_indep = true` to fold all
        // eliminated mass into K. It also filters weight ingestion to the show
        // variables (see [`run_stages`]), keeping a projected variable's mass
        // out of K.
        sampling: Sampling::Projection(show),
        weights,
        backbone_max_confl: None,
        // The same uninterruptible overrun source as the integer projected path
        // and, like it, this single lane keeps its checkpoint regardless of
        // overrun, so a large formula's overrun eats the budget. Own knob,
        // shared projected default.
        oracle: Oracle::Gated {
            max_vars: arjun.oracle_max_vars.weighted_projected.unwrap_or(u32::MAX),
            scale_mult: true,
        },
        no_sbva: no_sbva_call,
        no_bve: false,
        deadline,
        // Keep the checkpoint regardless of overrun — a single-lane anytime
        // pre-pass, exactly as the integer projected path.
        past_deadline: PastDeadline::Keep,
    };
    let StagedArjun { shim: a, .. } = run_stages(formula, &spec, |_| ())?;

    // Read formula + show + K + weights off the one `s->cur` so the quadruple is
    // consistent.
    let multiplier: BigRational =
        crate::cnf::parse_weight(multiplier_or_giveup(&a)?.trim()).ok()?;
    let reduced = a.cur_formula();
    let mut reduced_show = ShowSet::<Reduced>::from_zero_based(a.cur_sampl());
    let reduced_weights =
        Weights::try_from_dimacs_lits(reduced.num_vars, |l| lit_weight_or_giveup(&a, l))?;
    // Defined-var fold: a weight-carrying var not in `c p show` must be folded
    // back into show (sound — a defined var's value is fixed in every model, so
    // it doesn't change the projected model set; it only carries a
    // per-assignment weight).
    for var in reduced_weights.weighted_vars() {
        reduced_show.insert(var);
    }

    let input_to_reduced_lit = a.orig_to_new_lits(formula.num_vars);
    Some(super::arjun::ArjunWeightedProjResult {
        formula: reduced,
        show: reduced_show,
        weights: reduced_weights,
        multiplier,
        input_to_reduced_lit,
    })
}

#[cfg(test)]
mod tests;
