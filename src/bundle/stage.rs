//! The Arjun stage, as both counting chains run it.
//!
//! Every mode reaches Arjun through the same three steps — may it run, what may
//! it spend, and is what came back worth keeping — and only the call itself and
//! the mode's own keep-gate differ. Those two arrive as closures; everything
//! around them lives here, so a chain describes its mode rather than its
//! plumbing.

use super::*;

/// What the crate's own simplify chain did, which every chain settles the same
/// way: it runs unless the configuration turned it off, and it always produces
/// something — a chain with every stage off returns the formula it was given
/// rather than nothing.
pub(super) fn simplify_outcome(config: &RunConfig) -> StageOutcome {
    if config.stages.simplify {
        StageOutcome::Ran
    } else {
        StageOutcome::Skipped(SkipReason::NotRequested)
    }
}

/// Why the Arjun stage is skipped before it starts, for a reason that holds in
/// every mode, or `None` when it is not skipped. Reports the reason it is.
pub(super) fn arjun_skipped(formula: &CnfFormula, config: &RunConfig) -> Option<SkipReason> {
    if !config.stages.arjun {
        diag!("c note: skipping arjun (stage disabled)");
        return Some(SkipReason::NotRequested);
    }
    if formula.num_vars == 0 {
        diag!("c note: skipping arjun (nothing left to reduce)");
        return Some(SkipReason::NothingToDo);
    }
    None
}

/// The two things the shared Arjun stage needs from a reduction, whichever of
/// the four result shapes it came back as: the formula it produced, and the
/// variable map back to the formula it was handed.
///
/// Declared here rather than on the results themselves so that the `arjun` pass
/// does not grow a trait only the export chains use.
pub(super) trait ArjunReduction {
    /// The reduced formula.
    fn reduced_formula(&self) -> &CnfFormula;
    /// Input variable → reduced literal, for the formula Arjun was handed.
    fn var_map(&self) -> &VarMap<Reduced, Reduced>;
}

/// The four result shapes spell those two things the same way — a `formula`
/// and an `input_to_reduced_lit` — so the impl is written once and the shapes
/// are listed. A result that named them differently would not be admitted here
/// by accident.
macro_rules! impl_arjun_reduction {
    ($($result:ty),+ $(,)?) => {$(
        impl ArjunReduction for $result {
            fn reduced_formula(&self) -> &CnfFormula {
                &self.formula
            }
            fn var_map(&self) -> &VarMap<Reduced, Reduced> {
                &self.input_to_reduced_lit
            }
        }
    )+};
}

impl_arjun_reduction!(
    ArjunResult,
    ArjunProjResult,
    ArjunWeightedProjResult,
    ArjunWeightedResult,
);

/// What a chain's Arjun stage came back with: the reduction being exported, or
/// its absence. `P` and `W` are the chain's own unweighted and weighted result
/// types — the ones the two chains do not share, since a projected reduction
/// carries a show set an unprojected one has no place for. Every path to
/// `Skipped` reports its reason through [`crate::diagnostics`] first.
pub(super) enum ArjunOutcome<P, W> {
    Plain(P),
    Weighted(W),
    Skipped,
}

impl<P: ArjunReduction, W: ArjunReduction> ArjunOutcome<P, W> {
    /// The reduction Arjun kept, whichever shape it came back as, or `None`
    /// when the stage was skipped or its result discarded — the one place the
    /// three variants collapse to the two states a caller acts on.
    fn kept(&self) -> Option<&dyn ArjunReduction> {
        match self {
            ArjunOutcome::Plain(a) => Some(a),
            ArjunOutcome::Weighted(a) => Some(a),
            ArjunOutcome::Skipped => None,
        }
    }

    /// The formula Arjun produced, or `None` when the chain keeps the one it
    /// already had.
    pub(super) fn reduced_formula(&self) -> Option<&CnfFormula> {
        self.kept().map(ArjunReduction::reduced_formula)
    }

    /// The input→reduced map Arjun renumbered by, or `None` when nothing was
    /// renumbered.
    pub(super) fn var_map(&self) -> Option<&VarMap<Reduced, Reduced>> {
        self.kept().map(ArjunReduction::var_map)
    }
}

/// The Arjun stage skeleton all four modes share: refuse the stage when it may
/// not run, spend its allotted budget on it, then apply the keep-or-discard
/// gates — this mode's own, then the universal variable-map check — reporting
/// every refusal through [`crate::diagnostics`]. `None` means the caller keeps the
/// formula it already had.
///
/// The two things that genuinely differ come in as closures. `run` is the entry
/// point for the mode (plain, weighted, projected, weighted projected), which is
/// also the only place the mode's own arguments — a show set, a weight table —
/// are named. `discard_reason` is the mode's keep-gate, returning the phrase to
/// report when the reduction has to be thrown away; the gates themselves are all
/// [`arjun_keep_reduction`]'s.
pub(super) fn arjun_stage<R: ArjunReduction>(
    formula: &CnfFormula,
    config: &RunConfig,
    report: &mut StageReport,
    telemetry: &mut PreprocessTelemetry,
    run: impl FnOnce(std::time::Duration, bool) -> Result<Option<R>, VitriError>,
    discard_reason: impl FnOnce(&R) -> Option<DiscardReason>,
) -> Result<Option<R>, VitriError> {
    if let Some(why) = arjun_skipped(formula, config) {
        report.arjun = Some(StageOutcome::Skipped(why));
        return Ok(None);
    }
    // Decided once, here: the policy reads the clause set about to be reduced,
    // and asking it twice would scan the formula twice and leave two places for
    // the answer to be reported from.
    let no_sbva = no_sbva(formula, config);
    report.sbva = Some(if no_sbva {
        StageOutcome::Skipped(SkipReason::NotRequested)
    } else {
        StageOutcome::Ran
    });
    let started = std::time::Instant::now();
    let result = run(arjun_budget(config), no_sbva);
    telemetry.arjun_ms = Some(started.elapsed().as_millis() as u64);
    let Some(ar) = result? else {
        diag!("c note: skipping arjun (no result inside its budget)");
        report.arjun = Some(StageOutcome::GaveUp);
        return Ok(None);
    };
    if let Some(why) = discard_reason(&ar)
        && !(why == DiscardReason::NotSmaller
            && config.arjun_clause_growth == crate::config::ArjunClauseGrowth::KeepSound)
    {
        diag!("c note: discarding the arjun reduction ({})", why.phrase());
        report.arjun = Some(StageOutcome::Discarded(why));
        return Ok(None);
    }
    // A map that aliased two input variables onto one reduced variable would
    // still satisfy the count identity while making every model lifted back
    // through it wrong, so the reduction goes rather than the map being repaired.
    if !ar.var_map().is_injective(ar.reduced_formula().num_vars) {
        let why = DiscardReason::NonInjectiveMap;
        diag!("c note: discarding the arjun reduction ({})", why.phrase());
        report.arjun = Some(StageOutcome::Discarded(why));
        return Ok(None);
    }
    report.arjun = Some(StageOutcome::Ran);
    Ok(Some(ar))
}

/// Whether THIS Arjun call runs with bounded variable addition turned off:
/// [`ArjunOptions::sbva`](crate::preprocess::ArjunOptions::sbva), judged on the
/// clause set about to be reduced.
///
/// One helper rather than the decision spelled at each of the four stages, so
/// every mode applies the same policy to the same formula. Evaluated inside each
/// stage's run closure, so a stage the keep-gate skips pays nothing, and the
/// default [`ArjunSbva::On`](crate::preprocess::ArjunSbva::On) pays nothing at
/// all.
pub(super) fn no_sbva(formula: &CnfFormula, config: &RunConfig) -> bool {
    crate::preprocess::arjun::arjun_sbva_skip(formula, config.arjun.sbva)
}

/// The Arjun stage's budget: either [`crate::budget::arjun_budget_ms`] against
/// this run's wall-clock hint, or the exact duration its caller already carved
/// out. The result is clamped to whatever is actually left after the earlier
/// stages. Both inputs are read off the anchored `config`, which is the run's
/// own account of what is left to spend.
pub(super) fn arjun_budget(config: &RunConfig) -> std::time::Duration {
    let budget = match config.arjun_budget {
        crate::config::ArjunBudget::Derived => {
            std::time::Duration::from_millis(crate::budget::arjun_budget_ms(config.budget_ms))
        }
        crate::config::ArjunBudget::Exact(duration) => duration,
    };
    crate::budget::clamp(budget, config.deadline)
}
