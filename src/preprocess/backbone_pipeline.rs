//! Backbone-enhanced iterative equivalence preprocessing pipeline.
//!
//! Tarjan stage + unified `Probe` stage + the eq_iter pipeline, composed on the
//! shared pipeline driver; this file owns the `Probe` stage body ([`stage_probe`])
//! and the backbone-level wrapper ([`preprocess_backbone_eq_iter`]) that names
//! the `[Tarjan, Probe]` list and chains the eq_iter wrapper. The pipeline the
//! wrapper expresses:
//! 1. Tarjan SCC on original formula (free) — the shared [`Stage::Tarjan`]
//! 2. SAT-based backbone probing on Tarjan-reduced formula (budgeted) — Probe
//! 3. Inject backbone units + unit propagation (free) — Probe
//! 4. Tarjan SCC again (free) — catch equivs from new binary clauses — Probe
//! 5. SAT-based equivalence probing for leftovers (remaining budget) — Probe
//! 6. CaDiCaL simplify + final Tarjan (existing eq_iter wrapper)

use super::PipelineOutput;
use super::cadical_ffi::note_solver_unavailable;
use super::equivalence;
use super::pipelines::{ClauseCounts, Stage, StageOutcome, diff_stats, unsat_stats};
use super::probe_engine::ProbeEngine;
use super::unit_propagation;
use crate::cnf::{Clause, CnfFormula};
use crate::diagnostics::diag;

/// Statistics from backbone + equivalence detection phase.
#[derive(Clone, Debug, Default)]
pub(crate) struct BackboneStats {
    pub backbone_found: usize,
    pub backbone_probes: usize,
    pub backbone_ms: Option<u64>,
    pub equivalence_ms: Option<u64>,
}

/// `Stage::Probe` body, run as ONE stage on the shared pipeline driver: the
/// stage input `formula` is the Tarjan-reduced formula. Holds ONE CaDiCaL
/// session ([`ProbeEngine`]) across the internal formula rewrite (backbone →
/// unit-prop → post-backbone Tarjan → equiv probing), which is why these four
/// sub-phases stay internal rather than becoming separate pipeline-stage
/// entries.
///
/// **Budgets:** `backbone_budget` and `equiv_budget` are two SEQUENTIAL
/// internal budgets; each is clamped to the budget remaining at ITS OWN phase
/// start (a single entry-time clamp would widen the second). `None` deadline =
/// no clamp.
pub(super) fn stage_probe(
    formula: &CnfFormula,
    backbone_budget: std::time::Duration,
    equiv_budget: Option<std::time::Duration>,
    deadline: Option<std::time::Instant>,
    meter: &mut super::meter::PreprocessMeter,
) -> StageOutcome {
    let input = ClauseCounts::of(&formula.clauses);

    let partial_stats = |bb_count: usize,
                         bb_probes: usize,
                         backbone_ms: Option<u64>,
                         equivalence_ms: Option<u64>| BackboneStats {
        backbone_found: bb_count,
        backbone_probes: bb_probes,
        backbone_ms,
        equivalence_ms,
    };

    // Owned so later phases can rewrite it — the engine copies its own state,
    // not a borrow of `f`.
    let mut f = formula.clone();

    let Some(mut engine) = ProbeEngine::new(&f) else {
        note_solver_unavailable("probe", "the stage is skipped");
        return StageOutcome {
            formula: f,
            stats: diff_stats(input, input, 0),
            unsat: false,
            mapping: None,
            backbone: Some(partial_stats(0, 0, None, None)),
        };
    };

    // Phase 2: backbone probing.
    // Clamp the phase ceiling to the budget remaining now.
    let backbone_budget = meter.clamp(backbone_budget, deadline);
    let bb = engine.run_backbone_with_meter(backbone_budget, meter);

    if bb.unsat {
        return StageOutcome::refuted(
            CnfFormula::contradiction(formula.num_vars),
            unsat_stats(input, 0),
        )
        .with_backbone(partial_stats(
            0,
            bb.probes_completed,
            Some(bb.elapsed_ms),
            None,
        ));
    }

    let bb_count = bb.forced.len();
    let bb_probes = bb.probes_completed;

    if bb_count > 0 || bb.flippable_eliminated > 0 {
        diag!(
            "[backbone] {} forced vars ({}/{} probed, {} fixed, {} flippable-eliminated, {} model-eliminated, SAT solve {}ms)",
            bb_count,
            bb_probes,
            f.num_vars,
            bb.fixed_found,
            bb.flippable_eliminated,
            bb.model_eliminated,
            bb.solve_ms
        );
    }

    // Phase 3: inject backbone units + unit propagation.
    if bb_count > 0 {
        for lit in &bb.forced {
            f.clauses.push(Clause::new(vec![*lit]));
        }
        let (propagated_clauses, propagated_forced) =
            unit_propagation::propagate(&f.clauses, f.num_vars);

        // Re-pinning the forced literals as units can neither create nor remove
        // an empty clause, so the refutation test reads the same either side of
        // it — and this way it reads a formula rather than a loose clause list.
        let mut clauses = propagated_clauses;
        for &lit in &propagated_forced {
            clauses.push(Clause::new(vec![lit]));
        }
        let propagated = CnfFormula {
            num_vars: f.num_vars,
            clauses,
        };

        if propagated.is_refuted() {
            return StageOutcome::refuted(
                CnfFormula::contradiction(formula.num_vars),
                unsat_stats(input, bb_count + propagated_forced.len()),
            )
            .with_backbone(partial_stats(
                bb_count,
                bb_probes,
                Some(bb.elapsed_ms),
                None,
            ));
        }

        f = propagated;
    }

    // Phase 4: Tarjan SCC again.
    let (eq2, mapping2) = equivalence::extract_equivalences_with_mapping(&f);

    if eq2.is_unsat {
        return StageOutcome::refuted(
            CnfFormula::contradiction(formula.num_vars),
            unsat_stats(input, bb_count),
        )
        .with_backbone(partial_stats(
            bb_count,
            bb_probes,
            Some(bb.elapsed_ms),
            None,
        ));
    }

    if eq2.num_equivalences > 0 {
        diag!(
            "[post-backbone-tarjan] {} new equiv classes, {} → {} clauses",
            eq2.num_equivalences,
            f.clauses.len(),
            eq2.formula.clauses.len()
        );
    }
    f = eq2.formula;

    // Feed the phase-4 Tarjan substitutions to the engine as class merges — the
    // eliminated vars are gone from `f`, so the engine must neither probe nor
    // emit them.
    if let Some(m) = mapping2.as_ref() {
        engine.ingest_tarjan_equivs(m);
    }

    // Phase 5: SAT-based equiv probing for leftovers.
    let mut equivalence_ms = None;
    if let Some(equiv_budget) = equiv_budget {
        let equiv_budget = meter.clamp(equiv_budget, deadline);
        // The engine probes its already-refined classes (in phase-2 space) and
        // maps confirmed equivalences through the phase-4 mapping on emit.
        let eq_result = engine.run_equiv_with_meter(equiv_budget, &mapping2, meter);
        equivalence_ms = Some(eq_result.elapsed_ms);

        if eq_result.unsat {
            return StageOutcome::refuted(
                CnfFormula::contradiction(formula.num_vars),
                unsat_stats(input, bb_count),
            )
            .with_backbone(partial_stats(
                bb_count,
                bb_probes,
                Some(bb.elapsed_ms),
                equivalence_ms,
            ));
        }

        if !eq_result.equivalences.is_empty() {
            diag!(
                "[sat-equiv-probing] {} equivalences ({} probes)",
                eq_result.equivalences.len(),
                eq_result.probes_completed
            );
            // Inject equivalence clauses: l1 ↔ l2 as (¬l1 ∨ l2) ∧ (l1 ∨ ¬l2)
            for &(l1, l2) in &eq_result.equivalences {
                f.clauses.push(Clause::new(vec![l1.negated(), l2]));
                f.clauses.push(Clause::new(vec![l1, l2.negated()]));
            }
        }
    }

    let backbone_stats = BackboneStats {
        backbone_found: bb_count,
        backbone_probes: bb_probes,
        backbone_ms: Some(bb.elapsed_ms),
        equivalence_ms,
    };

    // Success: the probe can ADD clauses (backbone units + biconditionals), so
    // `diff_stats`'s saturating_sub keeps eliminated/shortened at 0 when the
    // formula grew. Stats are diffed against the STAGE INPUT (post-Tarjan); the
    // wrapper re-derives the whole-pipeline numbers against the original formula.
    let stats = diff_stats(input, ClauseCounts::of(&f.clauses), bb_count);

    StageOutcome {
        formula: f,
        stats,
        unsat: false,
        mapping: mapping2,
        backbone: Some(backbone_stats),
    }
}

/// Backbone-enhanced iterative equivalence preprocessing.
///
/// Thin wrapper over the shared pipeline driver: `[Stage::Tarjan, Stage::Probe]`
/// (phases 1–5) followed by `preprocess_eq_iter_with_mapping` (phase 6).
///
/// `deadline` is the whole-run wall-clock deadline derived from the caller's
/// budget. Each SAT-bounded phase's ceiling must never outlive it — the
/// backbone/equiv budgets clamp inside `stage_probe`, and phase 6's CaDiCaL
/// passes clamp inside the eq_iter wrapper. `None` = no clamp.
///
/// **Stats law:** the returned stats diff against the ORIGINAL (pre-Tarjan)
/// formula — NOT the pipeline driver's stage-merge — so the backbone level always
/// reports relative to its input; `forced_vars` = backbone found + eq_iter's
/// forced. On UNSAT, the whole-formula counts are all eliminated with
/// `forced_vars` sourced from the Probe stage's merged stats.
#[cfg(test)]
pub(crate) fn preprocess_backbone_eq_iter(
    formula: &CnfFormula,
    backbone_budget: std::time::Duration,
    equiv_budget: Option<std::time::Duration>,
    deadline: Option<std::time::Instant>,
) -> PipelineOutput {
    let mut meter = super::meter::PreprocessMeter::new(crate::config::PreprocessClock::WallClock);
    preprocess_backbone_eq_iter_with_meter(
        formula,
        backbone_budget,
        equiv_budget,
        deadline,
        &mut meter,
    )
}

pub(crate) fn preprocess_backbone_eq_iter_with_meter(
    formula: &CnfFormula,
    backbone_budget: std::time::Duration,
    equiv_budget: Option<std::time::Duration>,
    deadline: Option<std::time::Instant>,
    meter: &mut super::meter::PreprocessMeter,
) -> PipelineOutput {
    let original = ClauseCounts::of(&formula.clauses);

    // Phases 1-5: Tarjan (the shared stage) then the unified Probe stage.
    let p = super::pipelines::run_pipeline_with_meter(
        formula,
        &[
            Stage::Tarjan,
            Stage::Probe {
                backbone: backbone_budget,
                equiv: equiv_budget,
            },
        ],
        deadline,
        meter,
    );
    let bb_stats = p.backbone.unwrap_or_default();

    if p.formula.is_refuted() {
        // UNSAT: whole-diff stats against the ORIGINAL formula; forced_vars
        // comes through the stage stats merge (Tarjan contributes 0; Probe
        // emits the per-exit-point value).
        return PipelineOutput {
            formula: CnfFormula::contradiction(formula.num_vars),
            stats: unsat_stats(original, p.stats.forced_vars),
            mapping: None,
            backbone: Some(bb_stats),
        };
    }

    // Phase 6: CaDiCaL simplify + iterative Tarjan; the deadline threads
    // through to its passes.
    let eq_iter =
        super::pipelines::preprocess_eq_iter_with_mapping_and_meter(&p.formula, deadline, meter);

    let combined = diff_stats(
        original,
        ClauseCounts::of(&eq_iter.formula.clauses),
        bb_stats.backbone_found + eq_iter.stats.forced_vars,
    );
    PipelineOutput {
        stats: combined,
        backbone: Some(bb_stats),
        ..eq_iter
    }
}
