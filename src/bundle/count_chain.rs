//! `--mode mc` and `--mode wmc`: the count-preserving chain.
//!
//! Preprocess the formula, and record enough that the count of what is
//! left, scaled by what the record says, is the count of what went
//! in.

use num_traits::One;

use super::stage::{ArjunOutcome, arjun_stage, simplify_outcome};
use super::*;

/// `mc` / `wmc`: this crate's own simplify chain, then Arjun on what it
/// produced.
///
/// The weighted variant differs in exactly three places, all of them
/// parameterized rather than forked: DVE is frozen on the unequal-weight
/// variables, the DVE stage is kept or reverted by
/// [`weighted_lift::dve_verdict`], and each eliminated variable's factor is an
/// exact rational rather than a power of two.
/// The ordinary count bundle plus the exact simplify checkpoint it finished.
/// A full frontend session retains the checkpoint only when its retry policy
/// can use it; standalone preprocessing discards it without changing the
/// execution path.
pub(super) fn count_preserving_bundle_with_stage1(
    formula: &CnfFormula,
    meta: &CnfMeta,
    config: &RunConfig,
    mode: Mode,
) -> Result<(PreprocessBundle, CountStage1), VitriError> {
    let stage1 = count_stage1(formula, meta, config, mode);
    let bundle = finish_count_preserving_attempt(&stage1, config)?;
    Ok((bundle, stage1))
}

/// The count-preserving chain after simplify and before Arjun.
///
/// This is the one owned checkpoint an embedding frontend may keep while it
/// makes more than one Arjun attempt. It deliberately is not `Clone`: retries
/// borrow the same simplified formula and only copy the small per-attempt
/// report and telemetry values before Arjun updates them.
pub(super) struct CountStage1 {
    simplified: SimplifiedFormula,
    stage1_weights: Weights<Reduced>,
    stage1_lift: BigRational,
    stages: StageReport,
    telemetry: PreprocessTelemetry,
    mode: Mode,
}

/// Run the count-preserving chain's simplify stage exactly once.
pub(super) fn count_stage1(
    formula: &CnfFormula,
    meta: &CnfMeta,
    config: &RunConfig,
    mode: Mode,
) -> CountStage1 {
    let started = std::time::Instant::now();
    let weighted = mode.is_weighted();
    let orig_nv = formula.num_vars as usize;
    let orig_w = original_weights(meta, orig_nv, mode);
    let stages = StageReport {
        simplify: Some(simplify_outcome(config)),
        ..StageReport::default()
    };

    // ── Stage 1: the crate's own simplify chain ──────────────────────────────
    let purpose = if weighted {
        SimplifyPurpose::WeightedCount
    } else {
        SimplifyPurpose::Count
    };
    let mut simplified = simplify(formula, &preprocess_config(config, purpose, &orig_w));
    let mut telemetry = PreprocessTelemetry::from_simplified(&simplified, config.stages.simplify);

    // Weighted DVE is only sound under restrictions: an elimination no scalar
    // can express (an unequal-weight DEFINED variable, or an equivalence chain
    // ending at an eliminated variable) costs the whole stage, which falls back
    // to the exact pre-DVE formula. `freeze = true` is safe here because the
    // unequal-weight vars are frozen out of DVE below, so any residual
    // elimination is one a scalar can express.
    if weighted {
        let folded = weighted_lift::folded_weights(&simplified, &orig_w);
        if let DveVerdict::Revert(why) =
            weighted_lift::dve_verdict(&simplified, &folded, /*freeze=*/ true)
        {
            diag!("c note: reverting dve ({why})");
            // Dropping the DVE stage restores the exact pre-DVE formula: every
            // later reader — the lift, the weights, the variable map — composes
            // the stages that are present.
            simplified.dve_reduced = None;
        }
    }

    // Vtree construction needs at least one variable. When stripping removed
    // every one, the crate's own promotion puts a single backbone variable back
    // into the live set (as a unit clause, so it still contributes ×1 and the
    // lift is unchanged), so there is always something to build a vtree over.
    if !crate::cnf::contains_empty_clause(&simplified.reduced_formula().clauses)
        && simplified.reduced_formula().num_vars == 0
    {
        simplified.promote_all_backbone_to_live();
    }

    // The weight vector the count over stage 1's output must be taken under, and
    // the scalar stage 1 owes. Recomputed AFTER the possible DVE revert, because
    // both read the stage set that actually survived.
    let folded = weighted_lift::folded_weights(&simplified, &orig_w);
    // Stage 1's own renumbering, read as the correspondence it is: reduced
    // index `i` carries the folded weight of the original variable it stands for.
    let stage1_weights = simplified.composed_var_map().carry_weights(&folded);
    let stage1_lift = if weighted {
        weighted_lift::weighted_lift(&simplified, &orig_w, &folded)
    } else {
        BigRational::one()
    };

    telemetry.total_ms = started.elapsed().as_millis() as u64;
    CountStage1 {
        simplified,
        stage1_weights,
        stage1_lift,
        stages,
        telemetry,
        mode,
    }
}

/// Finish one Arjun attempt over an already-owned simplify checkpoint.
///
/// Kept separate from [`count_preserving_bundle_with_stage1`] so a frontend session
/// can retry Arjun without replaying simplify or cloning its formula.
pub(super) fn finish_count_preserving_attempt(
    stage1: &CountStage1,
    config: &RunConfig,
) -> Result<PreprocessBundle, VitriError> {
    let weighted = stage1.mode.is_weighted();
    let started = std::time::Instant::now();
    let mut bundle = finish_count_preserving_attempt_using(
        stage1,
        config,
        |formula, weights, report, telemetry| {
            if weighted {
                weighted_arjun_stage(formula, weights, config, report, telemetry)
            } else {
                plain_arjun_stage(formula, config, report, telemetry)
            }
        },
    )?;
    bundle.telemetry.total_ms = stage1
        .telemetry
        .total_ms
        .saturating_add(started.elapsed().as_millis() as u64);
    Ok(bundle)
}

/// The single finish path, with the Arjun invocation supplied by its caller.
/// The seam keeps the production composition singular and lets private tests
/// exercise reuse with deterministic Arjun outcomes.
fn finish_count_preserving_attempt_using(
    stage1: &CountStage1,
    config: &RunConfig,
    run_arjun: impl FnOnce(
        &CnfFormula,
        &Weights<Reduced>,
        &mut StageReport,
        &mut PreprocessTelemetry,
    ) -> Result<CountArjun, VitriError>,
) -> Result<PreprocessBundle, VitriError> {
    let simplified = &stage1.simplified;
    let mut stages = stage1.stages.clone();
    let mut telemetry = stage1.telemetry;

    // Preprocessing derived the empty clause: the instance is UNSAT.
    if let Some(mut bundle) = refuted(
        &simplified.reduced_formula().clauses,
        simplified.original.num_vars,
        stage1.mode,
        None,
        stages.clone(),
        telemetry,
    ) {
        bundle.decision_trace = simplified.decision_trace.clone();
        return Ok(bundle);
    }

    // ── Stage 2: Arjun, on what stage 1 produced ──────────────────────────────
    // Retained BEFORE the stage runs, because the stage is what a caller
    // re-reducing a derived formula wants to start from — and only when asked,
    // since it is a second whole formula held in memory.
    let arjun_input = config
        .retain_arjun_input
        .then(|| simplified.reduced_formula().clone());
    let arjun = run_arjun(
        simplified.reduced_formula(),
        &stage1.stage1_weights,
        &mut stages,
        &mut telemetry,
    )?;
    // Arjun refuted the instance.
    if let Some(f) = arjun.reduced_formula()
        && let Some(mut bundle) = refuted(
            &f.clauses,
            simplified.original.num_vars,
            stage1.mode,
            None,
            stages.clone(),
            telemetry,
        )
    {
        bundle.decision_trace = simplified.decision_trace.clone();
        return Ok(bundle);
    }

    // Attributed rather than fused: the simplify chain's share belongs to the
    // whole run and is applied once, while a caller re-reducing a formula
    // derived from `arjun_input` reconciles against the Arjun share alone.
    // Weighted, neither half is a power of two and both stay zero — the lift is
    // `PreprocessRecord::weight_lift` there.
    let count_lift = if stage1.mode.is_weighted() {
        CountLift::default()
    } else {
        CountLift {
            simplify_pow2: simplified.count_lift_pow2(0),
            arjun_pow2: arjun_multiplier_exp(&arjun),
        }
    };
    let record = count_preserving_record(
        simplified,
        &arjun,
        simplified.original.num_vars,
        stage1.mode,
        &stage1.stage1_lift,
        &stage1.stage1_weights,
        count_lift,
    );
    // The harvest is in the space of the formula Arjun produced, which is the
    // one being exported — so it travels only with a reduction that was KEPT.
    // A discarded or absent one leaves the export in stage 1's numbering, where
    // those clause literals would name different variables.
    let (reduced, learnt_clauses_reduced_dimacs, independent_support_reduced) = match arjun {
        CountArjun::Plain(ar) => (ar.formula, ar.learnt_clauses, Some(ar.independent_support)),
        CountArjun::Weighted(ar) => (ar.formula, Vec::new(), None),
        CountArjun::Skipped => (simplified.reduced_formula().clone(), Vec::new(), None),
    };
    // The harvest leaves no file behind, so this line is how a run that asked
    // for it can tell "Arjun derived none" from "the request went nowhere".
    if config.arjun.export_learned_clauses {
        diag!(
            "c note: exporting {} learnt clauses from arjun",
            learnt_clauses_reduced_dimacs.len(),
        );
    }
    Ok(PreprocessBundle {
        reduced,
        record,
        learnt_clauses_reduced_dimacs,
        stages,
        count_lift,
        telemetry,
        decision_trace: simplified.decision_trace.clone(),
        arjun_input,
        independent_support_reduced,
    })
}

/// The exponent the Arjun stage earned, or zero when it produced nothing to
/// keep. Read off the reduction rather than recomputed, and in one place, so it
/// cannot disagree with the exponent the record composes.
fn arjun_multiplier_exp(arjun: &CountArjun) -> u32 {
    match arjun {
        CountArjun::Plain(ar) => ar.multiplier_exp,
        // A weighted reduction's lift is a rational, not an exponent.
        CountArjun::Weighted(_) | CountArjun::Skipped => 0,
    }
}

/// Outcome of the count-preserving chain's Arjun stage. Its plain reduction is
/// show-blind, which is the whole of what distinguishes it from the projected
/// chain's.
pub(super) type CountArjun = ArjunOutcome<ArjunResult, ArjunWeightedResult>;

/// The clause-blowup gate both count-preserving stages apply: a reduction that
/// GREW the clause count compiles worse despite having fewer variables, so it is
/// discarded rather than exported.
pub(super) fn grew_clause_count(
    baseline_clauses: usize,
    reduced: &CnfFormula,
) -> Option<DiscardReason> {
    (!arjun_keep_reduction(ArjunKeep::ClauseCount {
        raw_clauses: baseline_clauses,
        reduced_clauses: reduced.clauses.len(),
    }))
    .then_some(DiscardReason::NotSmaller)
}

/// Plain (`mc`) Arjun over `formula` (already reduced by stage 1).
pub(super) fn plain_arjun_stage(
    formula: &CnfFormula,
    config: &RunConfig,
    report: &mut StageReport,
    telemetry: &mut PreprocessTelemetry,
) -> Result<CountArjun, VitriError> {
    let ar = arjun_stage(
        formula,
        config,
        report,
        telemetry,
        |budget, no_sbva| run_arjun_anytime(formula, budget, config.arjun, no_sbva),
        |ar| {
            grew_clause_count(
                config
                    .arjun_clause_growth
                    .clause_count_baseline(formula.clauses.len()),
                &ar.formula,
            )
        },
    )?;
    Ok(ar.map_or(CountArjun::Skipped, CountArjun::Plain))
}

/// Weighted (`wmc`) Arjun over stage 1's formula and stage 1's FOLDED weights.
///
/// Two gates, applied in order: the weighted usability gate (`ArjunKeep::Weighted`
/// — a full solve drops weighted mass its multiplier does not carry) and then
/// the shared clause-blowup gate.
pub(super) fn weighted_arjun_stage(
    formula: &CnfFormula,
    weights: &Weights<Reduced>,
    config: &RunConfig,
    report: &mut StageReport,
    telemetry: &mut PreprocessTelemetry,
) -> Result<CountArjun, VitriError> {
    let ar = arjun_stage(
        formula,
        config,
        report,
        telemetry,
        |budget, no_sbva| {
            run_arjun_weighted_anytime(
                formula,
                &weights.to_dimacs_pairs(),
                budget,
                config.arjun,
                no_sbva,
            )
        },
        |ar| {
            if !arjun_keep_reduction(ArjunKeep::weighted_for(formula.num_vars, ar)) {
                return Some(DiscardReason::WeightedUnusable);
            }
            grew_clause_count(
                config
                    .arjun_clause_growth
                    .clause_count_baseline(formula.clauses.len()),
                &ar.formula,
            )
        },
    )?;
    Ok(ar.map_or(CountArjun::Skipped, CountArjun::Weighted))
}

/// Assemble the count-preserving chain's record from its two completed stages.
/// Every number here is read off `simplified` / `arjun` and composed — nothing is
/// recomputed, and nothing is inferred from the shape of the formulas.
pub(super) fn count_preserving_record(
    simplified: &SimplifiedFormula,
    arjun: &CountArjun,
    original_num_vars: u32,
    mode: Mode,
    stage1_lift: &BigRational,
    stage1_weights: &Weights<Reduced>,
    count_lift: CountLift,
) -> PreprocessRecord {
    let weighted = mode.is_weighted();
    // Stage 1's map: index in `reduced_formula()` → ORIGINAL variable id, via the
    // ONE composed map on `SimplifiedFormula`. Never sign-flipped: stage 1 only
    // ever substitutes ELIMINATED variables, so a survivor stands for itself.
    let stage1_to_original =
        |j: usize| VarId(simplified.reduced_var_to_original(j) as u32).to_dimacs();

    // Compose stage 2 on top. Arjun's map is INPUT(=stage 1 output) var →
    // signed reduced literal; invert it and push each entry through stage 1's
    // map to land in the original space.
    let reduced_to_original_dimacs = match arjun.var_map() {
        Some(input_to_reduced) => input_to_reduced.invert_composed(
            arjun.reduced_formula().unwrap().num_vars,
            stage1_to_original,
        ),
        None => simplified.composed_var_map(),
    };

    let (mut forced_literals_original_dimacs, mut free_vars_original_dimacs) =
        simplified.stripped_forced_and_free();
    // DVE's own free variables. They are already inside `free_var_exp()`, so
    // naming them here is what makes `free_vars_original_dimacs.len()` add up to the
    // crate's share of the exponent instead of under-reporting it. They are named
    // by DVE-INPUT var, which `pre_dve_var_to_original` maps to the original
    // space; disjoint from the stripped `dead` set by construction (stripping
    // runs before DVE, so a stripped variable is not a DVE input).
    if let Some(dve) = simplified.dve_reduced.as_ref() {
        for j in dve.free_vars() {
            free_vars_original_dimacs
                .push(VarId(simplified.pre_dve_var_to_original(j) as u32).to_dimacs() as u32);
        }
    }

    let mut arjun_rational = BigRational::one();
    // The weights the reduced count is taken under: stage 1's folded weights,
    // pushed through Arjun's own renumbering when it ran (Arjun reports the
    // reduced weight table itself, which already carries whatever it folded).
    let mut final_weights: Option<Weights<Reduced>> = weighted.then(|| stage1_weights.clone());
    match arjun {
        CountArjun::Plain(ar) => {
            // Arjun's backbone is in the space of the formula it was HANDED (=
            // stage 1's output), so each literal maps back through stage 1's map
            // alone. Kept only for variables Arjun actually removed — a variable
            // still present in `reduced.cnf` is not a "forced literal" of this
            // bundle even if it happens to be forced.
            for l in &ar.backbone {
                let j = l.var.0 as usize;
                if ar.input_to_reduced_lit.get(l.var).is_some() {
                    continue;
                }
                let o = stage1_to_original(j);
                forced_literals_original_dimacs.push(if l.positive { o } else { -o });
            }
        }
        CountArjun::Weighted(ar) => {
            arjun_rational = ar.multiplier.clone();
            final_weights = Some(ar.weights.clone());
        }
        CountArjun::Skipped => {}
    }

    // Unweighted, the whole lift is this crate's own free-variable exponent with
    // Arjun's multiplier folded in — the same `count_lift(extra_pow2)`
    // composition a consumer applies. Weighted, each of those variables
    // contributes a rational instead, and stage 1 owes one of its own.
    let lift = if weighted {
        RecordLift::Weight(stage1_lift * arjun_rational)
    } else {
        RecordLift::Pow2(count_lift.total_pow2())
    };

    PreprocessRecord {
        forced_literals_original_dimacs,
        free_vars_original_dimacs,
        reduced_weights: final_weights.as_ref().map(Weights::to_record_rows),
        // `original_to_reduced_dimacs` stays absent: gate detection, DVE and
        // Arjun each remove a variable whose value a model of `reduced.cnf`
        // does not determine, so no total map over the original variables exists
        // to write. A count does not need one.
        ..PreprocessRecord::new(mode, lift, original_num_vars, reduced_to_original_dimacs)
    }
}

#[cfg(test)]
mod tests;
