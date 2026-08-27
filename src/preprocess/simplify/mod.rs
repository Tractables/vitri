//! Unified simplification: preprocessing + equivalence reduction + optional DVE.
//!
//! Reduces a CNF formula through progressive stages:
//!
//! ```text
//!   original ──→ preprocessed ──→ stripped ──→ equiv_reduced ──→ dve_reduced
//!                (CaDiCaL)        (backbone)   (SCC equivs)      (SAT-based, mc only)
//! ```
//!
//! `reduced_formula()` returns the most reduced stage available; that is what both
//! vtree construction and the output bundle use.
//! - **Expansion**: `equiv_reduced.mapping` for equivalences, `stripped` for backbone/dead vars.
//!   DVE-eliminated variables are NOT re-introduced — model count is adjusted by `2^dve_free`.

use crate::cnf::VarId;
use crate::cnf::{Clause, CnfFormula, Literal};
use crate::diagnostics::diag;

use super::equivalence::EquivMapping;
use super::gates;
use super::renumber::Renumber;

mod config;
mod record;
mod strip;
pub(crate) use config::*;
pub(crate) use record::*;
// Every item in `strip` is `pub(super)`, reached from this module and its own
// tests and nowhere else, so this import carries no re-export.
use strip::*;

/// Every layer a preprocessing run produces before DVE.
///
/// The two runs below return what they produced instead of filling in a
/// half-built [`SimplifiedFormula`], so the struct is assembled once, from
/// values — no field of it exists in a state a later stage still has to
/// repair.
#[derive(Default)]
struct Layers {
    preprocessed: Option<CnfFormula>,
    stripped: Option<Stripped>,
    equiv_reduced: Option<EquivReduction>,
    telemetry: SimplifyTelemetry,
}

fn preprocess_and_reduce(
    formula: &CnfFormula,
    config: &SimplifyConfig,
    meter: &mut super::meter::PreprocessMeter,
) -> Layers {
    use std::time::Duration;

    let (pipeline, strip_backbone, telemetry) = match config.prefix {
        SimplifyPrefix::Disabled => return Layers::default(),
        SimplifyPrefix::EqIter => (
            super::pipelines::preprocess_eq_iter_with_mapping_and_meter(
                formula,
                config.deadline,
                meter,
            ),
            false,
            SimplifyTelemetry::default(),
        ),
        SimplifyPrefix::Backbone {
            budget_ms,
            equivalence_budget_ms,
        } => {
            let pipeline = super::backbone_pipeline::preprocess_backbone_eq_iter_with_meter(
                formula,
                Duration::from_millis(budget_ms),
                equivalence_budget_ms.map(Duration::from_millis),
                config.deadline,
                meter,
            );
            let measured = pipeline.backbone.clone().unwrap_or_default();
            (
                pipeline,
                true,
                SimplifyTelemetry {
                    backbone_ms: measured.backbone_ms,
                    equivalence_ms: measured.equivalence_ms,
                    backbone_found: measured.backbone_found,
                    backbone_probes: measured.backbone_probes,
                    ..SimplifyTelemetry::default()
                },
            )
        }
    };

    diag!(
        "[simplify] {} clauses removed, {} literals shortened, {} forced vars",
        pipeline.stats.eliminated_clauses,
        pipeline.stats.shortened_literals,
        pipeline.stats.forced_vars,
    );

    let preprocessed = pipeline.formula;
    let mut layers = Layers {
        telemetry,
        ..Layers::default()
    };

    let mut mapping = pipeline.mapping;
    if strip_backbone && let Some((stripped, bb_reduction)) = strip_backbone_vars(&preprocessed) {
        // Removing forced vars keeps them out of the primal graph used for tree
        // decomposition. This belongs only to the prefix that actually proved
        // a backbone.
        diag!(
            "[backbone-stripping] {} → {} vars ({} forced removed)",
            preprocessed.num_vars,
            stripped.num_vars,
            bb_reduction.backbone.len(),
        );
        mapping = mapping.and_then(|m| m.remap_for_stripped(&bb_reduction));
        layers.stripped = Some(Stripped {
            formula: stripped,
            removed: bb_reduction,
        });
    }

    // Everything after the selected prefix converges here: one equivalence
    // reduction, then the shared gate/DVE tail below.
    let equivalence_input = layers
        .stripped
        .as_ref()
        .map_or(&preprocessed, |stripped| &stripped.formula);
    layers.equiv_reduced = apply_equiv_reduction(
        equivalence_input,
        mapping,
        config.stages.reduce_equivalences,
    );

    layers.preprocessed = Some(preprocessed);
    layers
}

pub(crate) fn simplify(formula: &CnfFormula, config: &SimplifyConfig) -> SimplifiedFormula {
    let started = std::time::Instant::now();
    let mut meter = super::meter::PreprocessMeter::new(config.clock);
    let layers = preprocess_and_reduce(formula, config, &mut meter);

    let mut result = SimplifiedFormula {
        original: formula.clone(),
        equiv_reduced: layers.equiv_reduced,
        dve_reduced: None,
        preprocessed: layers.preprocessed,
        stripped: layers.stripped,
        telemetry: layers.telemetry,
        decision_trace: None,
    };

    // DVE reduces `reduced_formula()`, so unlike the layers above it is stated
    // over the assembled result rather than over the input formula.
    if let Some(dve_budget) = config.stages.dve
        && !result.reduced_formula().is_refuted()
        && result.reduced_formula().num_vars > 0
    {
        let dve = run_dve(config, dve_budget, &result, &mut meter);
        result.telemetry.dve_ms = Some(dve.elapsed_ms);
        result.dve_reduced = dve.reduction;
    }

    result.telemetry.total_ms = started.elapsed().as_millis() as u64;
    result.decision_trace = meter.into_trace();
    result
}

/// One attempted DVE phase: its kept reduction, if any, and its elapsed time
/// whether or not the keep gate retained it.
struct DveAttempt {
    reduction: Option<DveReduction>,
    elapsed_ms: u64,
}

/// The DVE layer over the current `reduced_formula()`, or `None` when the pass
/// eliminated too little to be worth the renumber and recompile.
///
/// When gates are enabled, syntactic gate detection runs first (on the unmodified
/// CNF) and the set of gate output variables is fed into DVE as "known defined".
/// DVE then bypasses the expensive SAT definability probe for those vars and
/// eliminates them via resolution directly. The clause structure is preserved
/// (no gate pre-resolution), which helps DVE find further
/// defined vars that depend on the original structural cues.
///
/// The "meaningful elimination" guard matches the mc-branch heuristic: below
/// it, the renumber/recompile overhead isn't justified.
fn run_dve(
    config: &SimplifyConfig,
    budget: DveBudget,
    result: &SimplifiedFormula,
    meter: &mut super::meter::PreprocessMeter,
) -> DveAttempt {
    let dve_input = result.reduced_formula().clone();

    let known_defined: rustc_hash::FxHashSet<VarId> = if config.stages.gates {
        let mapping = gates::detect_gates(&dve_input);
        if !mapping.is_empty() {
            let mut by_type = [0usize; 5];
            for g in &mapping.gates {
                let idx = match g.gate_type {
                    gates::GateType::And => 0,
                    gates::GateType::Or => 1,
                    gates::GateType::Xor => 2,
                    gates::GateType::Xnor => 3,
                    gates::GateType::Ite => 4,
                };
                by_type[idx] += 1;
            }
            diag!(
                "[gate-detection] {} defined outputs ({} AND, {} OR, {} XOR, {} XNOR, {} ITE) — fed as DVE short-circuits",
                mapping.num_eliminated(),
                by_type[0],
                by_type[1],
                by_type[2],
                by_type[3],
                by_type[4],
            );
            mapping.eliminated.clone()
        } else {
            rustc_hash::FxHashSet::default()
        }
    } else {
        rustc_hash::FxHashSet::default()
    };

    // FREEZE-AND-KEEP (weighted DVE): `config.frozen_vars` holds ORIGINAL
    // VarId indices that must not be eliminated (unequal-weight vars whose
    // contribution is gate-value-dependent and thus non-scalar).
    let frozen_local = result.frozen_in_dve_space(&config.frozen_vars, dve_input.num_vars);

    let mut dve = super::dve::preprocess_dve_with_meter(
        &dve_input,
        budget.rounds,
        budget.budget_ms,
        false,
        &known_defined,
        &frozen_local,
        super::dve::FrozenEquiv::Ignore,
        meter,
    );
    super::dve::post_dve_strengthen_with_meter(&mut dve, &frozen_local, meter);
    let elapsed_ms = dve.elapsed_ms;

    let total_elim = dve.total_eliminated();
    let meaningful =
        total_elim >= 3 || total_elim as f64 / dve_input.num_vars.max(1) as f64 >= 0.05;

    if !meaningful {
        return DveAttempt {
            reduction: None,
            elapsed_ms,
        };
    }

    // `None` means the pass renumbered nothing, hence eliminated nothing —
    // which contradicts `meaningful` and so cannot occur here — but the
    // identity is what that case MEANS, making it the correct fallback.
    let renumbering = dve
        .renumbering
        .unwrap_or_else(|| Renumber::keeping(dve_input.num_vars as usize, |_| true));
    debug_assert_eq!(
        renumbering.num_old_vars(),
        dve_input.num_vars as usize,
        "the DVE renumbering must be stated over the space DVE was given",
    );
    DveAttempt {
        reduction: Some(DveReduction {
            formula: dve.formula,
            renumbering,
            fates: dve.fates,
        }),
        elapsed_ms,
    }
}

#[cfg(test)]
mod tests;
