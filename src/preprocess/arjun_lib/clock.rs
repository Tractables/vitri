//! The clock a reduction's deadlines are on: the wall, or the work clock of
//! the vendored CryptoMiniSat (`work_clock.h`), which counts each kind of
//! work the Arjun stack does (propagated literals, visited watches, conflicts,
//! oracle calls, and so on) and charges each its own units, so a reduction
//! under a deterministic preprocessing clock stops each stage at the same
//! point on every run.

use std::time::{Duration, Instant};

use super::shim::ArjunLib;
use crate::preprocess::meter::RUNAWAY_FACTOR;

/// Work units per millisecond of a reduction's budget on the work clock: what
/// an otherwise idle machine counts in a millisecond of the reduction.
const UNITS_PER_MS: f64 = 43_700.0;

/// How many kinds of work the clock counts.
pub(super) const WORK_KINDS: usize = 29;

/// Each kind of work the clock counts, in the order of `WorkKind` in
/// `work_clock.h`: its name in the diagnostic line, and the units one count of
/// it is charged. The charges are fitted together to the reduction's wall
/// time over a set of formulas, so that each formula's work milliseconds
/// track its wall milliseconds. Each charge is at least the least time one
/// count of its kind can take, so no kind of work is free: a reduction whose
/// work is mostly of a kind the fit set saw little of still advances the
/// clock.
pub(super) const WORK: [(&str, f64); WORK_KINDS] = [
    ("cms_prop", 1.37),
    ("cms_watch", 0.163),
    ("cms_bin", 0.611),
    ("cms_deref", 0.775),
    ("cms_confl", 269.0),
    ("cms_light", 0.0155),
    ("orc_prop", 2.17),
    ("orc_watch", 0.174),
    ("orc_bin", 0.0131),
    ("orc_deref", 0.679),
    ("orc_scan", 0.0942),
    ("orc_confl", 76.8),
    ("orc_learn", 0.24),
    ("orc_cache", 0.0131),
    ("orc_undo", 0.0436),
    ("orc_call", 4.01),
    ("orc_build", 0.0436),
    ("ls_visit", 0.434),
    ("ls_flip", 5.49),
    ("cdc_prop", 0.0872),
    ("cdc_confl", 0.872),
    ("cdc_ticks", 0.461),
    ("cdc_learn", 0.0218),
    ("cdc_subchk", 1.49),
    ("cdc_solve", 43.6),
    ("poll", 951.0),
    ("occ_lits", 0.201),
    ("sbva_lits", 0.0436),
    ("load", 420.0),
];

/// When a reduction must be done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArjunDeadline {
    /// At this wall instant.
    Wall(Instant),
    /// After `budget` on the work clock, or at the wall instant `wall`,
    /// whichever comes first. The stages read only the work clock until the
    /// wall instant passes, so a reduction that ends before it makes the same
    /// decisions on any machine.
    Work {
        /// The budget, in milliseconds of work at [`UNITS_PER_MS`].
        budget: Duration,
        /// The wall stop: a runaway reduction ends here with work left.
        wall: Instant,
    },
}

impl From<Instant> for ArjunDeadline {
    fn from(deadline: Instant) -> Self {
        ArjunDeadline::Wall(deadline)
    }
}

/// What the work clock read at the end of a reduction.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct WorkClockReading {
    /// Work milliseconds since the clock started.
    pub(super) now: Duration,
    /// Wall milliseconds since the clock started.
    pub(super) wall: Duration,
    /// The count of each kind of work, in the order of [`WORK`].
    pub(super) counts: [u64; WORK_KINDS],
    /// A deadline passed because the wall did, before the work reached it.
    pub(super) wall_stopped: bool,
}

impl ArjunDeadline {
    /// The work clock deadline for a reduction starting now with `budget`,
    /// stopped in wall time at the deterministic preprocessing phases'
    /// runaway multiple of the budget or at `backstop`, whichever comes first.
    pub(crate) fn work(budget: Duration, backstop: Option<Instant>) -> Self {
        let wall = Instant::now() + budget.saturating_mul(RUNAWAY_FACTOR);
        ArjunDeadline::Work {
            budget,
            wall: crate::budget::earliest(Some(wall), backstop).unwrap_or(wall),
        }
    }

    /// The wall instant the reduction is stopped at however much work it has
    /// left.
    pub(crate) fn wall(self) -> Instant {
        match self {
            ArjunDeadline::Wall(deadline) => deadline,
            ArjunDeadline::Work { wall, .. } => wall,
        }
    }

    /// Put `a`'s deadlines on this clock. Call before setting any of them; the
    /// work clock starts at `started`, the reduction's start.
    pub(super) fn arm(self, a: &mut ArjunLib, started: Instant) {
        if let ArjunDeadline::Work { budget, wall } = self {
            // The shim's wall stop is a multiple of the budget; this one ends
            // the stages at `wall`.
            let runaway = wall.saturating_duration_since(started).as_secs_f64()
                / budget.as_secs_f64().max(1e-3);
            a.start_work_clock(UNITS_PER_MS, runaway, &WORK.map(|(_, charge)| charge));
        }
    }

    /// The reduction's whole budget, from `started`.
    pub(super) fn budget(self, started: Instant) -> Duration {
        match self {
            ArjunDeadline::Wall(deadline) => deadline.saturating_duration_since(started),
            ArjunDeadline::Work { budget, .. } => budget,
        }
    }

    /// What the reduction has spent since `started`, on this clock.
    pub(super) fn elapsed(self, a: &ArjunLib, started: Instant) -> Duration {
        match self {
            ArjunDeadline::Wall(_) => started.elapsed(),
            ArjunDeadline::Work { .. } => a.work_clock().now,
        }
    }

    /// What is left of the budget, on this clock.
    pub(super) fn remaining(self, a: &ArjunLib) -> Duration {
        match self {
            ArjunDeadline::Wall(deadline) => crate::budget::remaining(deadline),
            ArjunDeadline::Work { budget, .. } => budget.saturating_sub(a.work_clock().now),
        }
    }

    /// Whether this is the work clock.
    pub(super) fn is_work(self) -> bool {
        matches!(self, ArjunDeadline::Work { .. })
    }
}
