//! The construction meter: a count of the work vtree construction does, and a
//! clock derived from it.
//!
//! Construction is a budgeted search. The portfolio divides its budget between
//! candidates, goatd divides its share between elimination configurations, the
//! FlowCutter restart loop stops when its window is gone, and the TD→vtree
//! sweep stops considering conversions once its bound is reached. Every one of
//! those decisions changes which tree comes out, and every one of them is
//! normally made against a wall clock — so the tree a run produces depends on
//! how fast, and how loaded, the machine was. Two runs of the same input on the
//! same binary can select different vtrees.
//!
//! This module replaces the quantity those decisions read. It counts *work*:
//! graph elements touched, in one unit shared by every construction here, and
//! serves a clock whose reading is `epoch + spent / rate`. A budget expressed
//! in units is then a budget in work, and the search that spends it makes the
//! same decisions in the same order on every machine.
//!
//! # What a unit is
//!
//! One unit is roughly one graph-element touch — a neighbour entry scanned, a
//! pin visited, a bitset word cleared. The absolute size does not matter; what
//! matters is that every construction charges on the *same* scale, so a budget
//! divided between them divides work rather than one backend's private counter.
//!
//! Charges are deliberately pessimistic. A caller that spends a unit budget it
//! sized from a wall (see
//! [`ConstructionBudget::units_for_wall_ms`](crate::config::ConstructionBudget::units_for_wall_ms))
//! should finish inside that wall rather than past it, so where a cost is only
//! known to within a factor the constant sits at the expensive end of the
//! range. The measured price of that choice is a few percent more construction
//! wall than the same build takes under a wall-clock budget.
//!
//! # The rate is a calibration constant
//!
//! [`UNITS_PER_MS`] converts units to the milliseconds every existing budget in
//! this crate is written in. It was fitted by regressing charged units against
//! measured milliseconds over a set of construction runs; it is not a law, and
//! a machine much faster or slower than the one it was fitted on will do more
//! or less real work per unit. That does not affect reproducibility — the same
//! unit budget buys the same *decisions* everywhere — only the wall those
//! decisions take.
//!
//! # How it is armed
//!
//! Nothing here reads the environment. The meter is armed for the duration of
//! one construction by [`arm`], from the single place that resolves a
//! construction budget, and disarmed when the returned guard drops. Off the
//! seam [`charge`] is a predictable branch and [`now`] is `Instant::now()`, so
//! a run that asks for no deterministic budget behaves exactly as it did
//! before this module existed.
//!
//! The state is thread-local, so a caller running two constructions on two
//! threads meters them independently.

use std::cell::Cell;
use std::time::{Duration, Instant};

/// Work units per millisecond of construction: the calibration constant that
/// converts a unit budget into the milliseconds this crate's budgets are
/// written in, and back.
///
/// Fitted as the median of `units / measured_ms` over per-candidate
/// construction runs. See the module docs on what that does and does not
/// guarantee.
pub(crate) const UNITS_PER_MS: u64 = 775_000;

thread_local! {
    /// Where the construction clock was started: the real instant the meter was
    /// armed, paired with the meter reading at that instant. `None` = not
    /// armed, which is every moment outside one metered construction.
    ///
    /// Written only by [`arm`] and by [`Armed::drop`]; read by everything else
    /// here. Being the one flag makes "armed" a single fact rather than
    /// something two cells could disagree about.
    static EPOCH: Cell<Option<(Instant, u64)>> = const { Cell::new(None) };

    /// Units charged on this thread, ever. Monotone and never reset, so a mark
    /// taken at arming turns into elapsed work by plain subtraction.
    static SPENT: Cell<u64> = const { Cell::new(0) };
}

/// The meter is armed for as long as this value lives. Dropping it restores
/// whatever was armed before, so a nested construction cannot leave the meter
/// running for the one that contains it.
#[must_use = "the meter is armed only while the guard is alive"]
pub(crate) struct Armed {
    previous: Option<(Instant, u64)>,
}

impl Drop for Armed {
    fn drop(&mut self) {
        EPOCH.with(|c| c.set(self.previous));
    }
}

/// Arm the meter, with the construction clock starting at `now` — the same
/// instant the budget being armed is measured from.
///
/// Pairing the epoch with the current meter reading is what makes a whole
/// construction spend ONE budget: a later component of a split formula enters
/// with the meter already advanced by the earlier ones, exactly as it would
/// enter with the clock already advanced.
pub(crate) fn arm(now: Instant) -> Armed {
    let previous = EPOCH.with(Cell::get);
    EPOCH.with(|c| c.set(Some((now, spent()))));
    Armed { previous }
}

/// Whether the meter is armed, and therefore whether anything charged to it can
/// be read back.
///
/// Hoisted out of hot loops that would otherwise compute a charge nobody
/// records: a charge whose *amount* costs a scan is guarded on this, and one
/// that is a single arithmetic expression is not.
#[inline]
pub(crate) fn metering() -> bool {
    EPOCH.with(Cell::get).is_some()
}

/// Charge `units` of construction work. Inert, and unread, when the meter is
/// not armed.
#[inline]
pub(crate) fn charge(units: u64) {
    if metering() {
        SPENT.with(|m| m.set(m.get().saturating_add(units)));
    }
}

/// Units charged on this thread so far.
#[inline]
pub(crate) fn spent() -> u64 {
    SPENT.with(Cell::get)
}

/// **THE CONSTRUCTION CLOCK.** `Instant::now()` when the meter is not armed;
/// under it, the instant the work says it is — the epoch plus the work charged
/// since, converted at [`UNITS_PER_MS`].
///
/// Every construction DECISION reads this instead of the real clock: the
/// portfolio's skip gate and fair shares, goatd's schedule and refinement
/// deadlines, the TD→vtree sweep's bound, the separator search's own cap.
///
/// It is an `Instant` and not a duration or a counter because that is the shape
/// construction's budgets already have: a deadline armed once travels as a bare
/// `Instant` through the portfolio driver into goatd's schedule, its
/// elimination core and the conversion sweep, and is compared against the clock
/// in a dozen places that never see the site that set it. Converting the clock
/// those comparisons read converts all of them at once and leaves every
/// deadline expression in the subsystem untouched.
///
/// Monotone, because the meter is: it never runs backwards and never precedes
/// the epoch. A run that charged enough units to overflow the addition
/// saturates at the epoch rather than panicking.
pub(crate) fn now() -> Instant {
    match EPOCH.with(Cell::get) {
        None => Instant::now(),
        Some((epoch, mark)) => {
            let ms = spent().saturating_sub(mark) / UNITS_PER_MS;
            epoch
                .checked_add(Duration::from_millis(ms))
                .unwrap_or(epoch)
        }
    }
}

/// A unit count as the milliseconds it converts to, for the one place a budget
/// in units has to be handed to code written in milliseconds.
pub(crate) fn wall_ms_for_units(units: u64) -> u64 {
    units / UNITS_PER_MS
}

#[cfg(test)]
mod tests;
