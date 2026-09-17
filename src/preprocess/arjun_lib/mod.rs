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

use crate::cnf::{CnfFormula, Reduced, ShowSet, Space, Weights};
use crate::diagnostics::diag;
use crate::error::VitriError;
use std::time::{Duration, Instant};

use super::arjun::{ArjunEffort, ArjunOptions, ArjunProjResult, ArjunResult};
use super::fork_budget::{ForkOutcome, run_forked_with_deadline};

mod budget_class;
mod knobs;
mod shim;
mod stages;

pub(in crate::preprocess) use budget_class::keep_overrun_enabled;
pub(in crate::preprocess) use knobs::{
    PROJECTED_ORACLE_MAX_VARS_DEFAULT, projected_oracle_max_vars,
};
pub(crate) use knobs::{export_learned_clauses_enabled, resolve_arjun_effort};
use shim::{ArjunLib, validate_shim_env};
use stages::{Oracle, PastDeadline, Sampling, ShimField, StageSpec, StagedArjun, run_stages};

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

/// A checkpoint reading this reduce cannot use, reported as a give-up and
/// turned into the `None` every reduce path answers a failed read with. The
/// readings themselves stay pure so a test can see the reason without a shim.
fn or_giveup<T>(read: Result<T, String>, spent: Spent) -> Option<T> {
    match read {
        Ok(value) => Some(value),
        Err(why) => {
            giveup("arjun-anytime", format_args!("{why}"), spent);
            None
        }
    }
}

/// The checkpoint's multiplier as the exponent N in `2^N`, or why it cannot be
/// one. Every unweighted reduce lifts its count by `2^N`, so a multiplier that
/// is not an exact power of two belongs to a weighted or unexpected reduction
/// and must not be coerced into one.
fn multiplier_exp_of(decimal: &str) -> Result<u32, String> {
    multiplier_decimal_to_exp(decimal)
        .ok_or_else(|| format!("multiplier {decimal:?} is not a power of two"))
}

/// The checkpoint's multiplier as an exact rational, or why it cannot be read.
/// The weighted reduces lift by this factor, so an unreadable one abandons the
/// reduction rather than being replaced by a default.
fn multiplier_weight_of(decimal: &str) -> Result<num_rational::BigRational, String> {
    crate::cnf::parse_weight(decimal.trim())
        .map_err(|e| format!("multiplier {decimal:?} does not parse: {e}"))
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
pub(crate) fn reduce_anytime(
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
    //     than the lite contract allows).
    // Those three shim-exposed heavy knobs are all the lite contract needs. The
    // remaining `elim_to_file` stages (extend-indep, autarky, renumber; BCE off
    // by default) stay on in both arms — cheap, count-preserving, and the shim
    // exposes no per-stage disable for them. Both arms keep the reduced CNF +
    // strictly-`2^N` multiplier contract; lite is count-preserving, only larger.
    let (oracle, no_sbva, no_bve) = match arjun.effort {
        ArjunEffort::Full => (
            Oracle::Gated {
                max_vars: arjun.oracle_max_vars.plain.unwrap_or(u32::MAX),
                scale_mult: true,
            },
            no_sbva_call,
            false,
        ),
        ArjunEffort::Lite => (Oracle::Off, true, true),
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
    let multiplier_exp = or_giveup(
        multiplier_exp_of(&multiplier_or_giveup(&a)?),
        Spent::Elapsed(started.elapsed()),
    )?;
    let full_formula = a.cur_formula();
    // The independent support is rewritten in lock-step with the formula by
    // `elim_to_file`; read it from this same final checkpoint and carry it as
    // reduced-space data rather than trying to reconstruct it later.
    let independent_support = ShowSet::from_vars(a.cur_sampl());
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
        independent_support,
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
pub(crate) fn reduce_anytime_projected<S: Space>(
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
    let multiplier_exp = or_giveup(
        multiplier_exp_of(&multiplier_or_giveup(&a)?),
        Spent::Unmeasured,
    )?;
    let reduced = a.cur_formula();
    let reduced_show = ShowSet::from_vars(a.cur_sampl());
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
pub(crate) fn reduce_anytime_weighted(
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
    let multiplier: BigRational = or_giveup(
        multiplier_weight_of(&multiplier_or_giveup(&a)?),
        Spent::Unmeasured,
    )?;
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
pub(crate) fn reduce_anytime_weighted_projected<S: Space>(
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
    let multiplier: BigRational = or_giveup(
        multiplier_weight_of(&multiplier_or_giveup(&a)?),
        Spent::Unmeasured,
    )?;
    let reduced = a.cur_formula();
    let mut reduced_show = ShowSet::<Reduced>::from_vars(a.cur_sampl());
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
