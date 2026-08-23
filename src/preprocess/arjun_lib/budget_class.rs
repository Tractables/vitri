//! Whether a reduction that came back late is kept or discarded: the outcome
//! classes, the boundary between them, and the one acceptance policy every
//! reduce path runs. Pure of the shim and of the stage driver, so the
//! boundaries are unit-testable on their own.

use super::{Spent, giveup};
use crate::diagnostics::diag;
use crate::error::VitriError;
use std::time::{Duration, Instant};

/// How far past `deadline` a reduction may return and still count as a
/// **deadline cut** (layer 1 stopped it cooperatively) rather than an
/// uncontrolled **overrun** (layer 1 never got a poll site and the stage simply
/// ran long).
///
/// Sized from the mechanism, not from taste: Arjun's deadline is polled at the
/// existing abort/loop sites (between `elim_to_file` steps, the independent-
/// support and extend loops, the CMS-oracle and CadiBack budget-exhausted
/// paths), so a cut lands at the next poll — a propagation/simplify step past
/// the deadline, not an unbounded interval — plus the read-back/serialize cost.
/// 5 s leaves room for a slower box and a large read-back while staying far
/// below the multiples (1.3–5×) that characterize the overrun class.
///
/// The independent outer bound: in the default forked configuration the
/// backstop `SIGKILL`s the child at `deadline + fork_budget::KILL_GRACE`, so a
/// cut later than that never reaches this classification (the caller sees the
/// hard-kill give-up line instead). The effective window is the smaller of the
/// two; this constant defines the class, the fork grace bounds the wall, and
/// raising the latter would spend extra budget on the overrun class, which is
/// discarded either way.
pub(in crate::preprocess) const DEADLINE_CUT_GRACE: Duration = Duration::from_millis(5_000);

/// What a reduce path's budget behaviour was, relative to its budget. Three
/// distinct classes: [`Self::DeadlineCut`] — the in-process deadline stopping a
/// stage cooperatively — is kept separate from [`Self::Overrun`] because the
/// two get opposite accept/discard treatment below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::preprocess) enum BudgetClass {
    /// Returned before the deadline — nothing special, accepted as always.
    InBudget,
    /// Returned within [`DEADLINE_CUT_GRACE`] of the deadline with the
    /// in-process deadline armed: Arjun stopped itself at the budget and handed
    /// back its sound partial checkpoint, on time. Not an overrun.
    ///
    /// Kept (handed to the caller), unlike [`Self::Overrun`]: a cut hands back a
    /// less-reduced formula on time, whereas an overrun hands back a
    /// more-reduced one bought with budget the caller no longer has. No knob —
    /// keeping is unconditional.
    DeadlineCut,
    /// Past the deadline by more than the grace, or with no in-process deadline
    /// armed: the stage ran uncontrolled and returned whenever it finished,
    /// which under a long budget can be most of it.
    ///
    /// Discarded by default, routing the caller straight to its own fallback
    /// instead of a compile over a reduction bought with budget it no longer
    /// has. `VITRI_ARJUN_KEEP_OVERRUN` opts out.
    Overrun,
}

/// Classify a reduce path's outcome. Pure (no clock, no env) so the boundaries
/// are unit-testable; `finished` is when the stages returned.
pub(in crate::preprocess) fn classify_budget(
    finished: Instant,
    deadline: Instant,
    deadline_armed: bool,
) -> BudgetClass {
    if finished <= deadline {
        BudgetClass::InBudget
    } else if deadline_armed && finished <= deadline + DEADLINE_CUT_GRACE {
        BudgetClass::DeadlineCut
    } else {
        BudgetClass::Overrun
    }
}

/// The one past-deadline acceptance policy. Returns true to hand the
/// checkpoint back to the caller, false to discard it (caller falls back to
/// the raw formula). Logs the give-up/cut line, so the classification and the
/// message describing it cannot drift.
pub(in crate::preprocess) fn keep_after_deadline(
    label: &str,
    finished: Instant,
    started: Instant,
    deadline: Instant,
    deadline_armed: bool,
    keep_overrun: bool,
) -> bool {
    let elapsed = finished.saturating_duration_since(started);
    let budget = deadline.saturating_duration_since(started);
    match classify_budget(finished, deadline, deadline_armed) {
        BudgetClass::InBudget => true,
        BudgetClass::DeadlineCut => {
            diag!(
                "[{label}] deadline-cut: sound checkpoint at {:.1}s (budget {:.1}s) — accepted",
                elapsed.as_secs_f64(),
                budget.as_secs_f64(),
            );
            true
        }
        BudgetClass::Overrun => {
            if keep_overrun {
                return true;
            }
            giveup(
                label,
                format_args!("overrun-discard"),
                Spent::ElapsedOfBudget(elapsed, budget),
            );
            false
        }
    }
}

/// Whether a full-count reduction that finished past its budget is still
/// returned to the caller instead of discarded. Default off (discard): a
/// more-reduced formula is not necessarily a more compilable one, and the
/// accept gate cannot see compile-friendliness. `VITRI_ARJUN_KEEP_OVERRUN=1`
/// (also `on`/`true`) opts in.
///
/// Scope: this knob is about the [`BudgetClass::Overrun`] class only —
/// [`BudgetClass::DeadlineCut`] is kept unconditionally regardless.
///
/// # Errors
///
/// [`VitriError::Env`] when the variable is set to neither an on nor an off
/// spelling.
pub(in crate::preprocess) fn keep_overrun_enabled() -> Result<bool, VitriError> {
    crate::env::env_flag("VITRI_ARJUN_KEEP_OVERRUN")
}
