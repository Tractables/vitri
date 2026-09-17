//! What vtree construction may spend, and the instant that resolves to.

use super::*;

/// How much of the run's remaining wall vtree construction may spend, or — for
/// [`Self::Deterministic`] — how much WORK it may do instead.
///
/// Construction is one phase of a run. A caller that hands this crate a
/// whole-run deadline is asking it to leave room for the phases either side of
/// construction; a caller that has already carved a construction window out of
/// its own wall is not — it is naming the window. Those are different requests,
/// and this is where they are told apart.
///
/// Whichever policy is chosen, the bound is SOFT and by more than one step: the
/// portfolio consults it between candidates, and FlowCutter checks it between
/// restart iterations and before each of its two greedy pre-passes. Whatever is
/// in flight when the bound passes runs to completion. The bound decides what is
/// *started*, not what is interrupted.
///
/// `#[non_exhaustive]`: a run bounded by something other than the clock is a
/// policy this enum should be able to gain without breaking a caller that
/// matches on it.
///
/// ```
/// use vitri::RunConfig;
/// use vitri::config::ConstructionBudget;
///
/// // The work a ninety-second construction is calibrated to do, asked for as a
/// // wall and stored as the work it converts to.
/// let config = RunConfig {
///     construction_budget: ConstructionBudget::for_wall_ms(90_000),
///     ..RunConfig::default()
/// };
/// config.validate()?;
///
/// assert_eq!(
///     config.construction_budget,
///     ConstructionBudget::Deterministic {
///         units: 90_000 * ConstructionBudget::UNITS_PER_MS,
///     },
/// );
/// # Ok::<(), vitri::VitriError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ConstructionBudget {
    /// Construction gets a SHARE of what is left when it starts: a third of the
    /// remaining wall, clamped to between 90 s and 900 s, and never past the run
    /// deadline.
    ///
    /// The default, and what a whole-run caller wants. Preprocessing has already
    /// spent part of the budget by the time construction starts, and the phases
    /// after construction still need some of what is left.
    #[default]
    Share,

    /// Construction may spend the whole remaining wall, up to the run deadline
    /// and no further.
    ///
    /// For a caller that has ALREADY decided how much of its wall construction
    /// gets and is passing that instant as [`RunConfig::deadline`]. Such a
    /// caller wants the deadline honoured as given; under [`Self::Share`] it
    /// would be divided a second time.
    WholeRemaining,

    /// Construction stops at this instant, or at the run deadline, whichever is
    /// sooner. It can only ever be tighter than [`Self::WholeRemaining`].
    ///
    /// For a caller whose construction window is neither the run deadline nor a
    /// fixed share of it.
    Until(Instant),

    /// Construction spends a fixed amount of WORK rather than a fixed amount of
    /// time.
    ///
    /// It counts the graph work it does — one unit is about one graph-element
    /// touch: a neighbour entry scanned, a hyperedge pin visited, a
    /// decomposition restart run — and makes every stopping decision against
    /// that count instead of against a clock. Every construction backend charges
    /// on that one scale, so a budget divided between them divides work rather
    /// than one backend's private counter. Two runs over the same formula at the
    /// same `units` therefore consider the same candidates in the same order and
    /// select the same vtree, on any machine, under any load, and whatever
    /// another thread is building beside them — the count belongs to the
    /// construction that spends it. None of the three policies above can promise
    /// that: which candidates a loaded machine gets through is what decides the
    /// tree.
    ///
    /// It bounds CONSTRUCTION and nothing else. The preprocessing ahead of it is
    /// budgeted on the clock as before, so a reproducible run needs those stages
    /// turned off as well.
    ///
    /// The count replaces the wall for construction entirely. This is the one
    /// policy that does not consult [`RunConfig::deadline`] — a deadline
    /// anchored before preprocessing leaves construction a different amount of
    /// time on every run, which is the dependence this variant exists to remove,
    /// and it applies to a run that declared no deadline at all. Size `units`
    /// for the wall you are willing to give construction with
    /// [`Self::units_for_wall_ms`], and expect a few percent more than that:
    /// charges are deliberately pessimistic, so a build finishes inside its
    /// budget rather than past it.
    Deterministic {
        /// Work units construction may spend, in the unit
        /// [`ConstructionBudget::UNITS_PER_MS`] converts. Must be positive.
        units: u64,
    },
}

impl ConstructionBudget {
    /// Work units one millisecond of construction is calibrated at.
    ///
    /// A calibration constant, not a law: it was fitted by regressing charged
    /// work against measured milliseconds over a set of construction runs, so a
    /// machine faster or slower than that one does more or less real work per
    /// unit. Reproducibility does not depend on it — the same unit budget buys
    /// the same decisions everywhere — only the wall those decisions take does.
    pub const UNITS_PER_MS: u64 = crate::decompose::meter::UNITS_PER_MS;

    /// The work `ms` milliseconds of construction is calibrated to do.
    ///
    /// The conversion is [`Self::UNITS_PER_MS`], exposed so a caller keeps the
    /// choice of stating work or stating the wall it converts from rather than
    /// having it made for them.
    pub fn units_for_wall_ms(ms: u64) -> u64 {
        ms.saturating_mul(Self::UNITS_PER_MS)
    }

    /// [`Self::Deterministic`] sized for `ms` milliseconds of construction —
    /// [`Self::units_for_wall_ms`] and the variant in one call, which is how a
    /// caller converting an existing wall-clock budget usually wants it.
    pub fn for_wall_ms(ms: u64) -> Self {
        ConstructionBudget::Deterministic {
            units: Self::units_for_wall_ms(ms),
        }
    }
}

impl RunConfig {
    /// The instant vtree construction will stop at, resolved against `now`: the
    /// run deadline of [`Self::resolved_deadline`] narrowed by
    /// [`Self::construction_budget`]. `None` when the run has no cutoff at all
    /// and none of its own — construction cannot be bounded by a share of
    /// nothing.
    ///
    /// This is the value construction enforces, not a second derivation of it,
    /// so a caller sizing its own downstream phases reads it here rather than
    /// recomputing the policy and hoping the two agree.
    ///
    /// Pass the instant construction starts at. Under
    /// [`ConstructionBudget::Share`] the answer depends on it: the share is of
    /// what is still left, so a run that spent most of its wall preprocessing
    /// gets a smaller construction window than the same run measured at its
    /// start.
    ///
    /// [`ConstructionBudget::Deterministic`] is the one policy that divides
    /// nothing: it names its own window in work, so it answers whether or not
    /// the run has a deadline, and never narrows to one. The instant it returns
    /// is on the construction meter's clock rather than the wall — which is what
    /// makes it a bound on work — so it is only meaningful while that meter is
    /// armed, and [`crate::component::build_vtree`] arms it at exactly this
    /// `now`.
    pub fn construction_deadline(&self, now: Instant) -> Option<Instant> {
        match self.construction_budget {
            ConstructionBudget::Deterministic { units } => {
                crate::budget::deterministic_deadline(units, now)
            }
            ConstructionBudget::Share => Some(crate::budget::vtree_share_deadline(
                self.resolved_deadline(now)?,
                now,
            )),
            ConstructionBudget::WholeRemaining => self.resolved_deadline(now),
            ConstructionBudget::Until(t) => Some(t.min(self.resolved_deadline(now)?)),
        }
    }
}
