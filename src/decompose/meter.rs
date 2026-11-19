//! One work clock across goatd and the CNF/vtree adapters around it, plus the
//! run's wall deadline as a backstop under a deterministic construction
//! budget.
//!
//! The work clock is goatd's, re-exported unchanged. The backstop is this
//! crate's: charged work is only an estimate of wall time, and on some formulas
//! construction costs far more wall than it is charged, so a work budget alone
//! can carry a build past the run's deadline. Every stopping check this crate
//! makes during construction goes through [`crate::budget::expired`] or
//! [`crate::budget::remaining`], and both read [`past_backstop`].

use std::cell::Cell;
use std::time::Instant;

pub(crate) use ::goatd::meter::*;

thread_local! {
    /// The wall instant a metered construction on this thread stops at.
    static BACKSTOP: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// The work clock and the backstop, armed for as long as this value lives.
/// Dropping it restores the backstop armed before, then disarms the clock.
#[must_use = "the meter is armed only while the guard is alive"]
pub(crate) struct Armed {
    previous: Option<Instant>,
    _work: Guard,
}

impl Drop for Armed {
    fn drop(&mut self) {
        BACKSTOP.with(|b| b.set(self.previous));
    }
}

/// Arm the work clock at `epoch` and stop construction at `backstop`, the
/// run's wall deadline, whatever work is left. `None` leaves the build bounded
/// by work alone.
pub(crate) fn arm_with_backstop(epoch: Instant, backstop: Option<Instant>) -> Armed {
    let previous = BACKSTOP.with(|b| b.replace(backstop));
    Armed {
        previous,
        _work: arm(epoch),
    }
}

/// Whether the backstop of the construction running on this thread has
/// passed. Reads the wall clock only while one is armed.
#[inline]
pub(crate) fn past_backstop() -> bool {
    BACKSTOP
        .with(Cell::get)
        .is_some_and(|d| Instant::now() >= d)
}
