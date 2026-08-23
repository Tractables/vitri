//! `--mode pmc` and `--mode pwmc`: the projection-preserving chain.
//!
//! Same shape as the plain chain, but everything preprocessing touches
//! has to stay expressible over the show set — a variable outside it
//! may be eliminated freely, one inside it may not.

use super::stage::{ArjunOutcome, NO_PROJECTION_GAIN, arjun_stage, no_sbva};
use super::*;

use crate::cnf::{Original, ShowSet, WeightTable, Weights};

/// `pmc` / `pwmc`: Arjun's projection-set minimization, then the shared
/// projected reduction.
///
/// Both stages are exactly ×1 for the projected count, and the projected
/// reduction PRESERVES variable ids, so the only renumbering to compose is
/// Arjun's.
///
/// None of the count-preserving chain's stages runs here, and each is excluded
/// for its own reason: the `2^k` lift charges ×2 for a stripped variable, which
/// is wrong (×1) for a projected-out one and the exponent cannot tell them apart;
/// equivalence substitution can eliminate a SHOW variable, leaving the show set
/// naming a variable `reduced.cnf` no longer has; DVE on a SHOW variable defined
/// by hidden ones merges two distinct show-projections and undercounts; and plain
/// Arjun is show-blind. The last two have show-aware counterparts in this chain
/// (DVE frozen on the show set, `arjun-projection-min`).
pub(super) fn projection_preserving_bundle(
    formula: &CnfFormula,
    meta: &CnfMeta,
    config: &RunConfig,
    mode: Mode,
) -> Result<PreprocessBundle, VitriError> {
    let weighted = mode.is_weighted();
    let orig_nv = formula.num_vars as usize;
    // This chain's show set IS the file's declared show set — the "the mode
    // uses it" half of the seam [`CnfMeta::declared_show_vars`] states. A
    // projected mode without one has nothing to preserve and is refused up front
    // by `resolve_mode`.
    let orig_show = meta
        .declared_show_vars()
        .expect("a projected mode requires a show set");
    let orig_w = original_weights(meta, orig_nv, mode);
    // The literals that carry an EXPLICIT `c p weight` line, not the resolved
    // (defaulted) table above.
    let weight_pairs: Vec<(i32, BigRational)> =
        weight_table(meta, mode).map_or_else(Vec::new, WeightTable::to_literal_pairs);

    // ── Stage 1: Arjun's projection-set minimization ─────────────────────────
    let arjun = projected_arjun_stage(formula, orig_show, &weight_pairs, config, mode)?;
    // The map is the one thing still wanted at the end of the chain, and it is a
    // word per variable; taking a copy of it here is what lets the reduction's
    // payload — a whole formula, its show set, its weight table — MOVE out of the
    // outcome rather than be cloned to keep the outcome alive alongside it.
    let arjun_map: Option<VarMap<Reduced, Reduced>> = arjun.var_map().cloned();
    let (work, mut show_set, mut w, lift) = match arjun {
        ProjArjun::Plain(a) => (
            a.formula,
            a.show,
            Weights::empty(),
            RecordLift::Pow2(a.multiplier_exp),
        ),
        ProjArjun::Weighted(a) => (
            a.formula,
            a.show,
            a.weights,
            RecordLift::Weight(a.multiplier),
        ),
        // Arjun did not run, so it owes no factor of its own — and, having not
        // run, it renumbered nothing: `work` IS `formula`, so the declared set
        // and the declared weights already read over the reduced formula. An
        // unprojected-weight track carries no table at all.
        ProjArjun::Skipped => (
            formula.clone(),
            orig_show.clone().assume_reduced_identity(),
            if weighted {
                orig_w.clone().assume_reduced_identity()
            } else {
                Weights::empty()
            },
            RecordLift::neutral(),
        ),
    };

    // The declared set as a refutation bundle carries it: that bundle is a
    // contradiction over the ORIGINAL variable space, renumbered from the input
    // by nothing at all, so the declared set already reads over its "reduced"
    // formula. Both refutation checks below hand over the same set.
    let refutation_show = orig_show.clone().assume_reduced_identity();
    if let Some(bundle) = refuted(
        &work.clauses,
        formula.num_vars,
        mode,
        Some(refutation_show.clone()),
    ) {
        return Ok(bundle);
    }

    // ── Stage 2: the shared projected reduction ──────────────────────────────
    // Show-frozen strengthening (every elimination hidden, ×1) then projected BVE
    // (clause-level ∃, also ×1). The projected reduction takes the formula and
    // the show set and nothing else: `pmc` and `pwmc` run the identical two
    // stages, and there is no argument through which either could reach a stage
    // that is not ×1 under a projection.
    let ProjectedReduction {
        formula: reduced,
        show_set: reduced_show,
        folds,
    } = strengthen_and_bve(&work, show_set, config.deadline);
    // The projected reduction can REFUTE the instance too: its BCP pass returns the empty
    // clause when propagation hits a conflict, and projected BVE carries that
    // clause through untouched (a clause with no literals is in no occurrence
    // list).
    if let Some(bundle) = refuted(
        &reduced.clauses,
        formula.num_vars,
        mode,
        Some(refutation_show),
    ) {
        return Ok(bundle);
    }
    show_set = reduced_show;
    // A show variable merged away as equivalent to another COUNTED one leaves the
    // show set (already done inside the projected reduction); under weights its
    // survivor now stands for both, so it carries the product of their literal
    // weights.
    if weighted {
        w.fold_eliminated(&folds);
    }

    // The variable map. The projected reduction preserves ids, so the whole
    // renumbering is Arjun's, and it is already input(=original)→reduced.
    let reduced_num_vars = reduced.num_vars;
    if weighted {
        // The projected reduction preserves ids, so the reduced formula's variables are a
        // PREFIX of what the fold worked on — which is what makes a resize the
        // whole of the space change here.
        w.resize_neutral(reduced_num_vars as usize);
    }
    let reduced_to_original_dimacs = match &arjun_map {
        // The projected reduction above preserved ids, so what Arjun calls its input IS
        // the original formula and the inverted map already names originals.
        Some(input_to_reduced) => input_to_reduced
            .invert(reduced_num_vars)
            .assume_original_target(),
        // No Arjun ⇒ no renumbering: the reduced formula is the original's
        // variable space with clauses removed.
        None => VarMap::identity(reduced_num_vars),
    };

    // Three of the neutral defaults are this chain's answer, each for its own
    // reason. `original_to_reduced_dimacs`: a projected-out variable is
    // ∃-absorbed — a reduced model determines neither its value nor that it has
    // one — so there is no total map over the original variables to write.
    // `forced_literals_original_dimacs`: neither projected stage reports forced
    // polarities, since the strengthening pass reports ELIMINATIONS and a
    // variable it eliminated is ∃-absorbed, not fixed.
    // `free_vars_original_dimacs`: no stage of this chain removes a variable as
    // "free" — Arjun folds free SHOW variables into its multiplier (they are
    // named by no list), and BVE removes only projected-out ones, which are ×1.
    let record = PreprocessRecord {
        show_vars_reduced_dimacs: Some(show_set),
        reduced_weights: weighted.then(|| w.to_record_rows()),
        ..PreprocessRecord::new(mode, lift, formula.num_vars, reduced_to_original_dimacs)
    };
    Ok(PreprocessBundle {
        reduced,
        record,
        learnt_clauses_reduced_dimacs: Vec::new(),
    })
}

/// Outcome of the projection-preserving chain's Arjun stage. Its plain
/// reduction minimizes the projection, so it carries the show set the reduced
/// formula is counted over; the count-preserving chain's does not.
pub(super) type ProjArjun = ArjunOutcome<ArjunProjResult, ArjunWeightedProjResult>;

/// Arjun's projection-set minimization, integer or weighted, under the
/// projection keep-gate: keep the reduction ONLY when it actually minimized the
/// PROJECTION. A pure variable elimination that leaves the show set and the
/// multiplier untouched has zero counting benefit and can compile strictly worse
/// than the raw formula, so it is discarded.
pub(super) fn projected_arjun_stage(
    formula: &CnfFormula,
    orig_show: &ShowSet<Original>,
    weight_pairs: &[(i32, BigRational)],
    config: &RunConfig,
    mode: Mode,
) -> Result<ProjArjun, VitriError> {
    if mode.is_weighted() {
        // The projected weighted entry point owns the soundness step that makes
        // weights and projection composable: a weight-carrying variable is folded
        // INTO the show set before Arjun runs, so no weighted mass is
        // ever projected away. It is handed only the literals that carry an
        // EXPLICIT weight, because a weight of 1 written out for every projected
        // variable would drag the whole variable set into the show set for no
        // gain.
        let ar = arjun_stage(
            formula,
            config,
            |budget| {
                run_arjun_weighted_projected_anytime(
                    formula,
                    orig_show,
                    weight_pairs,
                    budget,
                    no_sbva(formula, config),
                )
            },
            |ar| {
                (!arjun_keep_reduction(ArjunKeep::weighted_projection_for(
                    orig_show.len(),
                    formula.num_vars,
                    ar,
                )))
                .then_some(NO_PROJECTION_GAIN)
            },
        )?;
        Ok(ar.map_or(ProjArjun::Skipped, ProjArjun::Weighted))
    } else {
        let ar = arjun_stage(
            formula,
            config,
            |budget| {
                run_arjun_projected_anytime(formula, orig_show, budget, no_sbva(formula, config))
            },
            |ar| {
                (!arjun_keep_reduction(ArjunKeep::projection_for(orig_show.len(), ar)))
                    .then_some(NO_PROJECTION_GAIN)
            },
        )?;
        Ok(ar.map_or(ProjArjun::Skipped, ProjArjun::Plain))
    }
}
