//! Preprocessing pipelines.
//!
//! A pipeline is a `&[Stage]` list run by the single [`run_pipeline`] driver,
//! which owns what each composition would otherwise repeat: the UNSAT
//! early-exit, the stats merge, the deadline threading. The simplification
//! algorithms live in the [`Stage`] bodies, never hand-stitched into the
//! wrappers.
//!
//! Two wrappers name lists. [`preprocess_eq_iter_with_mapping`] — the one
//! `preprocess_*` function here — runs `[Tarjan, CadicalSimplify]`, then a
//! conditional second CaDiCaL pass, and returns the simplified formula, stats,
//! and an optional [`equivalence::EquivMapping`] for equivalence-aware vtree
//! construction.
//! `backbone_pipeline::preprocess_backbone_eq_iter` runs `[Tarjan, Probe]` and
//! chains that wrapper afterwards; its `Probe` stage body lives in
//! `backbone_pipeline.rs` (see that module's doc).

use super::cadical;
use super::equivalence;
use crate::cnf::{Clause, CnfFormula};
use crate::diagnostics::diag;

/// What one stage, or a whole pipeline, did to the clause set — the counts the
/// run's preprocessing summary is written from.
#[derive(Clone, Debug, Default)]
pub(crate) struct PreprocessStats {
    pub original_clauses: usize,
    pub eliminated_clauses: usize,
    pub shortened_literals: usize,
    pub forced_vars: usize,
}

/// The size of a clause set, in the two units the stats below are written
/// from. They are always measured together, over the same clauses, so they
/// travel as one value.
///
/// `pub(super)` so stage bodies in sibling modules (e.g. `backbone_pipeline`'s
/// `Probe` body) share this ONE definition instead of duplicating it.
#[derive(Clone, Copy, Debug)]
pub(super) struct ClauseCounts {
    pub clauses: usize,
    pub literals: usize,
}

impl ClauseCounts {
    pub(super) fn of(clauses: &[Clause]) -> Self {
        ClauseCounts {
            clauses: clauses.len(),
            literals: clauses.iter().map(|c| c.literals.len()).sum(),
        }
    }
}

/// Build stats by comparing original vs. final clause/literal counts.
pub(super) fn diff_stats(
    before: ClauseCounts,
    after: ClauseCounts,
    forced_vars: usize,
) -> PreprocessStats {
    PreprocessStats {
        original_clauses: before.clauses,
        eliminated_clauses: before.clauses.saturating_sub(after.clauses),
        shortened_literals: before.literals.saturating_sub(after.literals),
        forced_vars,
    }
}

/// Stats for the UNSAT case: all clauses/literals counted as eliminated.
pub(super) fn unsat_stats(before: ClauseCounts, forced_vars: usize) -> PreprocessStats {
    PreprocessStats {
        original_clauses: before.clauses,
        eliminated_clauses: before.clauses,
        shortened_literals: before.literals,
        forced_vars,
    }
}

// ===========================================================================
// The `Stage` enum + the single `run_pipeline` driver.
// ===========================================================================

/// One preprocessing stage: consumes a formula, returns a further-reduced one
/// as a [`StageOutcome`]. The unit of stats, UNSAT-exit, and deadline handling.
///
/// New behavior = a new variant in this ONE file, exhaustively matched by
/// [`run_stage`] — no forked pipeline copies.
pub(super) enum Stage {
    /// SCC literal-equivalence extraction. Free (no SAT solve), UNSAT when a
    /// variable is equivalent to its own negation. Stats-neutral — see
    /// [`stage_tarjan`] for why.
    Tarjan,
    /// Freeze-only CaDiCaL simplification of every variable, UNSAT on empty
    /// clause. Deadline-only bounding — see [`stage_cadical_simplify`].
    CadicalSimplify,
    /// Unified SAT probing (`ProbeEngine`, ONE CaDiCaL session): backbone probing,
    /// backbone-unit injection + unit propagation, a post-backbone Tarjan pass,
    /// then SAT equivalence probing — the former `backbone_pipeline` phases 2–5,
    /// internal to this ONE stage because splitting the phases into separate list
    /// entries would force solver-session state through the pipeline driver.
    /// `backbone`/`equiv` are the two internal phase budgets — see
    /// [`super::backbone_pipeline::stage_probe`] for the clamping rule. The stage
    /// input is the Tarjan-reduced formula (former phase 1 is a preceding
    /// `Stage::Tarjan`).
    /// Body: [`super::backbone_pipeline::stage_probe`].
    Probe {
        backbone: std::time::Duration,
        equiv: Option<std::time::Duration>,
    },
}

/// The result of running one [`Stage`]. Uniform across stages so the pipeline
/// driver can merge/early-exit without knowing which stage produced it.
pub(super) struct StageOutcome {
    /// The further-reduced formula. Every stage here is count-neutral — the
    /// equivalence reduction rides `mapping`, the free-variable lift rides
    /// `simplify.rs` — so a stage hands back a formula and nothing else about
    /// the count.
    pub formula: CnfFormula,
    /// Per-stage stats; the pipeline driver folds these into one
    /// [`PreprocessStats`].
    pub stats: PreprocessStats,
    /// When set, the formula is UNSAT and the pipeline driver stops executing
    /// stages.
    pub unsat: bool,
    /// Vtree-usable equivalence mapping; the pipeline driver keeps
    /// last-writer-wins.
    /// `None` for stages that emit no mapping (e.g. `CadicalSimplify`).
    pub mapping: Option<equivalence::EquivMapping>,
    /// Backbone/equiv probing stats; last-writer-wins. `None` for stages that do
    /// no probing.
    pub backbone: Option<super::BackboneStats>,
}

impl StageOutcome {
    /// The outcome of a stage that derived a contradiction: `formula` is the
    /// refutation it reports. No stage runs after one, so there is no mapping
    /// to carry and nothing left to carry it to.
    pub(super) fn refuted(formula: CnfFormula, stats: PreprocessStats) -> Self {
        StageOutcome {
            formula,
            stats,
            unsat: true,
            mapping: None,
            backbone: None,
        }
    }

    /// Attach the probing counts a stage had already collected when it returned.
    pub(super) fn with_backbone(mut self, backbone: super::BackboneStats) -> Self {
        self.backbone = Some(backbone);
        self
    }
}

/// What a preprocessing pipeline returns: the reduced formula, merged stats,
/// and the carried mapping/backbone.
///
/// The shape of every pipeline in this module and of the backbone wrapper —
/// which re-derives its stats against its own input rather than handing this
/// one on, but reports them in the same four slots.
pub(crate) struct PipelineOutput {
    /// The reduced formula after the last stage that ran.
    pub formula: CnfFormula,
    /// Every stage's stats folded into one, by `merge_stats`:
    /// `original_clauses` from the FIRST stage's input, the rest summed.
    pub stats: PreprocessStats,
    /// The variable renaming to compose back to the input numbering, carried
    /// from the last stage that produced one. `None` if no stage did.
    pub mapping: Option<equivalence::EquivMapping>,
    /// Backbone / equivalence probing counters from the last stage that
    /// probed. `None` if none did.
    pub backbone: Option<super::BackboneStats>,
}

/// Fold one stage's stats into the running total — the ONE cross-stage merge
/// site (the per-stage `diff_stats`/`unsat_stats` build a single stage's
/// numbers; this composes them): `original_clauses` is pinned to the FIRST
/// stage's input, the rest sum.
fn merge_stats(acc: Option<PreprocessStats>, next: PreprocessStats) -> PreprocessStats {
    match acc {
        None => next,
        Some(acc) => PreprocessStats {
            original_clauses: acc.original_clauses,
            eliminated_clauses: acc.eliminated_clauses + next.eliminated_clauses,
            shortened_literals: acc.shortened_literals + next.shortened_literals,
            forced_vars: acc.forced_vars + next.forced_vars,
        },
    }
}

/// Dispatch one [`Stage`] against the current formula. Exhaustive over `Stage`;
/// each new variant lands its body here (or in a helper it calls). `deadline`
/// is threaded to every stage for a final shape — `Tarjan` ignores it.
fn run_stage(
    stage: &Stage,
    formula: &CnfFormula,
    deadline: Option<std::time::Instant>,
    meter: &mut super::meter::PreprocessMeter,
) -> StageOutcome {
    match stage {
        Stage::Tarjan => stage_tarjan(formula),
        Stage::CadicalSimplify => stage_cadical_simplify(formula, deadline, meter),
        Stage::Probe { backbone, equiv } => {
            super::backbone_pipeline::stage_probe(formula, *backbone, *equiv, deadline, meter)
        }
    }
}

/// `Stage::Tarjan` body — the exact former equivalence-extraction step
/// (`equivalence::extract_equivalences_with_mapping`): substitute each SCC
/// equivalence class by a representative, emit the vtree-usable `mapping`, and
/// UNSAT-exit when a variable is equivalent to its own negation.
///
/// **Neutral stats:** the equivalence reduction never appears in the
/// clause/literal counters — it is carried by `mapping` and applied at
/// vtree/TDD-expansion time — so this stage contributes zero
/// eliminated/shortened/forced and pins `original_clauses` to its OUTPUT clause
/// count, the formula CaDiCaL actually consumes. That makes the merged
/// `original_clauses` CaDiCaL's own baseline. Free (no SAT solve).
fn stage_tarjan(formula: &CnfFormula) -> StageOutcome {
    let (eq_result, mapping) = equivalence::extract_equivalences_with_mapping(formula);

    if eq_result.is_unsat {
        return StageOutcome::refuted(
            eq_result.formula,
            unsat_stats(ClauseCounts::of(&formula.clauses), 0),
        );
    }

    if eq_result.num_equivalences > 0 {
        diag!(
            "[tarjan] {} classes, {} → {} clauses",
            eq_result.num_equivalences,
            formula.clauses.len(),
            eq_result.formula.clauses.len()
        );
    }

    let stats = PreprocessStats {
        original_clauses: eq_result.formula.clauses.len(),
        ..Default::default()
    };
    StageOutcome {
        formula: eq_result.formula,
        stats,
        unsat: false,
        mapping,
        backbone: None,
    }
}

/// `Stage::CadicalSimplify` body: freeze-only CaDiCaL simplification of every
/// variable
/// (`cadical::preprocess_cadical_with_meter(formula, 3, deadline, meter)`),
/// UNSAT-exit on empty
/// clause, diff stats otherwise. Deadline-only bounding: `deadline` is threaded
/// straight into `preprocess_cadical_with_meter`, which clamps to the remaining
/// budget (`None` = unbounded). No own budget.
fn stage_cadical_simplify(
    formula: &CnfFormula,
    deadline: Option<std::time::Instant>,
    meter: &mut super::meter::PreprocessMeter,
) -> StageOutcome {
    let before = ClauseCounts::of(&formula.clauses);

    let (result, forced_count) =
        cadical::preprocess_cadical_with_meter(formula, 3, deadline, meter);

    // Empty clause signals UNSAT (via `traverse_clauses`).
    if result.is_refuted() {
        return StageOutcome::refuted(
            CnfFormula::contradiction(formula.num_vars),
            unsat_stats(before, forced_count),
        );
    }

    let stats = diff_stats(before, ClauseCounts::of(&result.clauses), forced_count);
    StageOutcome {
        formula: result,
        stats,
        unsat: false,
        mapping: None,
        backbone: None,
    }
}

/// The ONE pipeline driver: run `stages` in order against `formula`, owning the
/// cross-stage obligations exactly once.
///
/// - **Per-stage dispatch** via [`run_stage`].
/// - **UNSAT early-exit:** the first `outcome.unsat` stops execution; the unsat
///   formula + stats-merged-so-far are returned (later stages do not run).
/// - **Stats merge:** a single fold through [`merge_stats`].
/// - **Mapping / backbone:** last-writer-wins across stages.
/// - **Deadline threading:** `deadline` is passed to every stage; free stages
///   ignore it.
#[cfg(test)]
pub(super) fn run_pipeline(
    formula: &CnfFormula,
    stages: &[Stage],
    deadline: Option<std::time::Instant>,
) -> PipelineOutput {
    let mut meter = super::meter::PreprocessMeter::new(crate::config::PreprocessClock::WallClock);
    run_pipeline_with_meter(formula, stages, deadline, &mut meter)
}

pub(super) fn run_pipeline_with_meter(
    formula: &CnfFormula,
    stages: &[Stage],
    deadline: Option<std::time::Instant>,
    meter: &mut super::meter::PreprocessMeter,
) -> PipelineOutput {
    // `None` while still pointing at the caller's input (avoids cloning it);
    // becomes `Some(reduced)` after the first stage consumes it.
    let mut current: Option<CnfFormula> = None;
    let mut merged: Option<PreprocessStats> = None;
    let mut mapping: Option<equivalence::EquivMapping> = None;
    let mut backbone: Option<super::BackboneStats> = None;

    for stage in stages {
        let input: &CnfFormula = current.as_ref().unwrap_or(formula);
        let outcome = run_stage(stage, input, deadline, meter);

        merged = Some(merge_stats(merged, outcome.stats));
        if outcome.mapping.is_some() {
            mapping = outcome.mapping;
        }
        if outcome.backbone.is_some() {
            backbone = outcome.backbone;
        }

        let unsat = outcome.unsat;
        current = Some(outcome.formula);
        if unsat {
            break;
        }
    }

    PipelineOutput {
        // Empty `stages` is the only path that never set `current` — return the
        // input unchanged (an honest degenerate case; every caller passes at
        // least one stage).
        formula: current.unwrap_or_else(|| formula.clone()),
        stats: merged.unwrap_or_default(),
        mapping,
        backbone,
    }
}

/// Iterative equivalence: equiv → CaDiCaL → equiv again, returning the combined
/// mapping. CaDiCaL's preprocessing can create new binary implications that
/// reveal new equivalences.
///
/// A composition of driver runs with one pipeline-level branch (the "did pass 2
/// find equivalences" decision stays in the wrapper — it is control flow, not a
/// stage):
/// - Pass 1: `run_pipeline([Tarjan, CadicalSimplify])`; UNSAT short-circuits.
/// - Re-extract equivalences on the pass-1 result (a direct extraction, so the
///   pass-2 log line is preserved); none found ⇒ pass-1 result is final.
/// - Pass 2: `run_pipeline([CadicalSimplify])` on the further-simplified formula.
/// - Final mapping: `run_pipeline([Tarjan])` on the pass-2 result — a fresh
///   vtree-usable mapping consistent with the final variable space (the pass-1
///   mapping may be inconsistent after pass 2 merged representatives).
///
/// Combined stats: `merge_stats(pass1, pass2)` pins `original_clauses` to pass 1
/// and sums the rest (the pass-1 result is itself the `[Tarjan, CadicalSimplify]`
/// merge; the final `[Tarjan]` run's stats are discarded). `deadline` bounds each
/// CaDiCaL pass. No stage here probes, so the backbone slot stays empty.
#[cfg(test)]
pub(super) fn preprocess_eq_iter_with_mapping(
    formula: &CnfFormula,
    deadline: Option<std::time::Instant>,
) -> PipelineOutput {
    let mut meter = super::meter::PreprocessMeter::new(crate::config::PreprocessClock::WallClock);
    preprocess_eq_iter_with_mapping_and_meter(formula, deadline, &mut meter)
}

pub(super) fn preprocess_eq_iter_with_mapping_and_meter(
    formula: &CnfFormula,
    deadline: Option<std::time::Instant>,
    meter: &mut super::meter::PreprocessMeter,
) -> PipelineOutput {
    let p1 = run_pipeline_with_meter(
        formula,
        &[Stage::Tarjan, Stage::CadicalSimplify],
        deadline,
        meter,
    );

    if p1.formula.is_refuted() {
        return p1;
    }

    // Direct extraction, not a `Stage::Tarjan` run, so the eq_iter-specific log
    // line below is preserved.
    let (eq2, _m2) = equivalence::extract_equivalences_with_mapping(&p1.formula);
    if eq2.num_equivalences == 0 {
        return p1;
    }

    diag!(
        "[iter-equiv-pass-2] {} new classes, {} → {} clauses",
        eq2.num_equivalences,
        p1.formula.clauses.len(),
        eq2.formula.clauses.len()
    );

    let p2 = run_pipeline_with_meter(&eq2.formula, &[Stage::CadicalSimplify], deadline, meter);
    let combined = merge_stats(Some(p1.stats), p2.stats);

    let pf = run_pipeline_with_meter(&p2.formula, &[Stage::Tarjan], deadline, meter);
    PipelineOutput {
        stats: combined,
        ..pf
    }
}
