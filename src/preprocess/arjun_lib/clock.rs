//! The clock a reduction's deadlines are on: the wall, or the work clock of
//! the vendored CryptoMiniSat (`work_clock.h`), which counts the literals its
//! search and its oracle propagate and the watches they visit, so a reduction
//! under a deterministic preprocessing clock stops each stage at the same
//! point on every run.

use std::time::{Duration, Instant};

use super::shim::ArjunLib;
use crate::preprocess::meter::RUNAWAY_FACTOR;

/// Work units per millisecond of a reduction's budget on the work clock: what
/// an otherwise idle machine propagates in a millisecond of the reduction.
const UNITS_PER_MS: f64 = 50_000.0;

/// Work units charged for each poll of the backbone search's terminator,
/// which CaDiCaL makes every ten turns of its search loop.
const UNITS_PER_POLL: f64 = 200.0;

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
    pub(super) visits: u64,
    pub(super) polls: u64,
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
            a.start_work_clock(UNITS_PER_MS, runaway, UNITS_PER_POLL);
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
