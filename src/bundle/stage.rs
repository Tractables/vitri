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

/// Why the Arjun stage is skipped before it starts, or `None` when it is not
/// skipped. Reports the reason it is.
///
/// The first two reasons hold in every mode. The third holds under `mc` alone,
/// and it reads `formula`, the formula the stage is handed: what the simplify
/// chain left, not the input.
pub(super) fn arjun_skipped(
    formula: &CnfFormula,
    config: &RunConfig,
    mode: Mode,
) -> Option<SkipReason> {
    if !config.stages.arjun {
        diag!("c note: skipping arjun (stage disabled)");
        return Some(SkipReason::NotRequested);
    }
    if formula.num_vars() == 0 {
        diag!("c note: skipping arjun (nothing left to reduce)");
        return Some(SkipReason::NothingToDo);
    }
    if mode == Mode::Mc && is_monotone(formula) {
        diag!("c note: skipping arjun (monotone formula: no variable is forced or defined)");
        return Some(SkipReason::Monotone);
    }
    None
}

/// Whether `formula` is monotone up to renaming, as [`SkipReason::Monotone`]
/// states it: every variable occurs, always in the same polarity, and every
/// clause names two variables or more.
///
/// Rename each variable so that it occurs positively. The assignment making
/// every variable true then satisfies the formula, and so does each assignment
/// that differs from it in one variable, because every clause keeps a true
/// literal on another variable. So both values of every variable extend one
/// assignment of the others: no variable is forced, and none is defined by the
/// others. Those are the variables Arjun's independent-support minimization and
/// elimination remove. It also drops, for a factor of two, a variable the
/// formula does not depend on: requiring every variable to occur rules out one
/// that no clause mentions, though not one whose every clause is subsumed by a
/// clause without it (`docs/preprocessing.md` lists what the skip forgoes).
///
/// The argument is about the plain model count. Weights can make a variable
/// count as forced that the formula leaves free, and a show set lets a variable
/// go that nothing defines, so the other modes do not ask.
fn is_monotone(formula: &CnfFormula) -> bool {
    let num_vars = formula.num_vars() as usize;
    let freq = crate::cnf::occ::literal_frequency(formula.clauses(), num_vars);
    let one_polarity_each = (0..num_vars).all(|v| {
        let positive = freq[crate::cnf::occ::literal_index(v, true)] > 0;
        let negative = freq[crate::cnf::occ::literal_index(v, false)] > 0;
        positive != negative
    });
    if !one_polarity_each {
        return false;
    }
    // A clause whose literals all name one variable is a unit however many times
    // it repeats the literal, and the empty clause names none.
    formula.clauses().iter().all(|clause| {
        let mut vars = clause.literals.iter().map(|l| l.var);
        vars.next().is_some_and(|first| vars.any(|v| v != first))
    })
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
    pub(super) fn kept(&self) -> Option<&dyn ArjunReduction> {
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
/// are named; it is handed the instant the stage must be back by, turned from
/// this stage's budget here so the four modes cannot disagree about when the
/// clock started. `discard_reason` is the mode's keep-gate, returning the phrase
/// to report when the reduction has to be thrown away; the gates themselves are
/// all [`arjun_keep_reduction`]'s. `mode` is the chain's mode, which only the
/// skip gate ([`arjun_skipped`]) reads.
pub(super) fn arjun_stage<R: ArjunReduction>(
    formula: &CnfFormula,
    config: &RunConfig,
    mode: Mode,
    report: &mut StageReport,
    telemetry: &mut PreprocessTelemetry,
    run: impl FnOnce(std::time::Instant, bool) -> Result<Option<R>, VitriError>,
    discard_reason: impl FnOnce(&R) -> Option<DiscardReason>,
) -> Result<Option<R>, VitriError> {
    if let Some(why) = arjun_skipped(formula, config, mode) {
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
    let result = run(started + arjun_budget(config), no_sbva);
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
    if !ar.var_map().is_injective(ar.reduced_formula().num_vars()) {
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
/// every mode applies the same policy to the same formula. Evaluated once in
/// [`arjun_stage`], after its skip gate, so a formula that skips Arjun never
/// pays for it, and the default
/// [`ArjunSbva::On`](crate::preprocess::ArjunSbva::On) pays nothing at all.
pub(super) fn no_sbva(formula: &CnfFormula, config: &RunConfig) -> bool {
    crate::preprocess::arjun::arjun_sbva_skip(formula, config.arjun.sbva)
}

/// The Arjun stage's budget: either [`ArjunBudget::derived_duration`] of this
/// run's wall-clock hint, or the exact duration its caller already carved out.
/// The result is clamped to whatever is actually left after the earlier
/// stages. Both inputs are read off the anchored `config`, which is the run's
/// own account of what is left to spend.
///
/// [`ArjunBudget::derived_duration`]: crate::config::ArjunBudget::derived_duration
pub(super) fn arjun_budget(config: &RunConfig) -> std::time::Duration {
    let budget = match config.arjun_budget {
        crate::config::ArjunBudget::Derived => {
            crate::config::ArjunBudget::derived_duration(config.budget_ms)
        }
        crate::config::ArjunBudget::Exact(duration) => duration,
    };
    crate::budget::clamp(budget, config.deadline)
}
