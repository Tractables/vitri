//! The shared stage skeleton. All four reduce paths — full count, projected,
//! weighted, weighted projected — drive Arjun through [`run_stages`]; the axes
//! they differ on are the fields of [`StageSpec`], so a change to the sequence
//! lands on all four at once.
//!
//! Reading the checkpoint back is not shared: the four return different result
//! types over different multiplier arithmetic, so each entry point does its own
//! read-back off the handle this hands it.

use crate::cnf::{CnfFormula, Literal, ShowSet, Space};
use crate::diagnostics::diag;
use std::time::Instant;

use super::budget_class::keep_after_deadline;
use super::knobs::oracle_mult_for_budget;
use super::shim::ArjunLib;
use super::{Spent, giveup};

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
pub(super) enum ShimField {
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
pub(super) enum Sampling<'a, S: Space> {
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
pub(super) enum Oracle {
    /// Off: the passes do not run and `oracle_mult` is inert.
    Off,
    /// On when the formula is small enough AND enough budget remains.
    Gated {
        /// Variable count above which the oracle is skipped; `u32::MAX` = no
        /// size gate. Each path resolves its own from
        /// [`OracleCaps`](crate::preprocess::arjun::OracleCaps).
        max_vars: u32,
        /// Whether `oracle_mult` is sized from the budget still remaining when the
        /// oracle starts, bounding its worst case near that budget
        /// ([`oracle_mult_for_budget`]).
        scale_mult: bool,
    },
}

/// What a reduction does with a checkpoint that arrives past its deadline.
#[derive(Clone, Copy)]
pub(super) enum PastDeadline {
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
pub(super) struct StageSpec<'a, S: Space> {
    /// Prefix for this path's diagnostic lines.
    pub(super) label: &'static str,
    /// Whether the per-stage give-up lines are reported at all — a per-path
    /// choice, since some existing callers are silent about a stage that
    /// simply did not fit and reporting there would change what they see.
    pub(super) report_giveups: bool,
    /// Integer or rational arithmetic — picks the shim constructor.
    pub(super) field: ShimField,
    /// Seed for Arjun's own randomization — see
    /// [`ArjunOptions::seed`](crate::preprocess::arjun::ArjunOptions::seed).
    pub(super) seed: u32,
    /// The sampling set, which also fixes `all_indep`.
    pub(super) sampling: Sampling<'a, S>,
    /// Per-literal weights to ingest, as `(signed DIMACS literal, weight)`;
    /// empty on an integer path. Only sampling-set variables' weights are
    /// ingested — see [`run_stages`] for why a projected variable's weight must
    /// not reach the shim.
    pub(super) weights: &'a [(i32, num_rational::BigRational)],
    /// The heavy stage's oracle gate.
    pub(super) oracle: Oracle,
    /// Disable SBVA in the heavy stage (count-preserving).
    pub(super) no_sbva: bool,
    /// Disable BVE in the heavy stage (count-preserving).
    pub(super) no_bve: bool,
    /// The budget this reduction runs against, absolute.
    pub(super) deadline: Instant,
    /// What to do with a checkpoint that arrives past `deadline`.
    pub(super) past_deadline: PastDeadline,
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
pub(super) struct StagedArjun<T> {
    /// The handle to read the checkpoint off. Every getter reads the one
    /// `s->cur`, so formula, sampling set, weights and multiplier are always a
    /// consistent tuple.
    pub(super) shim: ArjunLib,
    /// When the stages started.
    pub(super) started: Instant,
    /// The `after_minimize` closure's result.
    pub(super) harvest: T,
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
pub(super) fn run_stages<T, S: Space>(
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
