//! Portfolio vtree construction: several backends built for the same formula,
//! one of them selected on a cost score.

mod catalog;
mod driver;

#[cfg(test)]
mod tests;

pub(crate) use driver::vtree_from_portfolio;

/// Projected selection's tie band, as a fraction of the narrowest peak
/// frontier. Tuned, not derived.
const DEFAULT_PEAK_TOLERANCE: f64 = 0.10;

/// What a portfolio build is configured with, beyond the formula and the
/// budget.
///
/// One value rather than four loose fields on the selection context: every one
/// of them is read here and nowhere else, so this is where they are named, and
/// a caller varying them is varying one thing.
#[derive(Clone, Debug, PartialEq)]
pub struct PortfolioKnobs {
    /// Seed for the goatd candidate (the FlowCutter candidates seed
    /// themselves). `0` is the production setting; a different seed is a cheap
    /// vtree-diversity axis for retry experiments.
    pub seed: u64,

    /// How much of the candidate trace to print.
    pub trace: TraceLevel,

    /// Wall-clock cap in milliseconds on the FlowCutter primal candidate under
    /// projected selection, applied only to components above two thousand
    /// variables. `None` (the default) leaves the candidate fully deterministic
    /// and step-budgeted. A cap makes it anytime, which lets a dense projected
    /// component hand its remaining budget to later work instead of spending
    /// all of it here — at the price of a candidate whose output depends on
    /// machine speed.
    pub flowcutter_cap_ms: Option<i64>,

    /// Relative tolerance band for projected selection. Candidates whose peak
    /// context width is within this fraction of the narrowest are
    /// treated as a tie and decided on clause-load balance instead, because a
    /// marginally narrower frontier is not worth a much worse balanced tree.
    /// `0.0` makes the peak width an exact argmin.
    pub peak_tolerance: f64,
}

impl Default for PortfolioKnobs {
    /// The production configuration: the fixed seed, no trace, no cap, and the
    /// tuned tie band.
    fn default() -> Self {
        PortfolioKnobs {
            seed: 0,
            trace: TraceLevel::Off,
            flowcutter_cap_ms: None,
            peak_tolerance: DEFAULT_PEAK_TOLERANCE,
        }
    }
}

/// How much of a portfolio build to narrate.
///
/// [`TraceLevel::All`] costs candidates: it BUILDS and scores the
/// multilevel-hypergraph family at every imbalance point the generation gate
/// would have skipped, purely so the trace shows them. Those extra candidates
/// are discarded — selection never sees them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TraceLevel {
    /// Print nothing.
    #[default]
    Off,
    /// One line per scored candidate.
    Scored,
    /// Also the candidates the generation gate skips.
    All,
}

impl PortfolioKnobs {
    /// Fill the knobs from the `VITRI_*` process environment: a variable that is
    /// set overrides the knob it names, an unset one leaves the caller's value.
    ///
    /// # Errors
    ///
    /// [`VitriError`](crate::error::VitriError) naming the offending variable
    /// and the form it expects.
    pub(super) fn with_env_defaults(self) -> Result<Self, crate::error::VitriError> {
        use crate::env::{env_raw, parse};
        let PortfolioKnobs {
            seed,
            trace,
            flowcutter_cap_ms,
            peak_tolerance,
        } = self;
        Ok(PortfolioKnobs {
            seed: parse(
                "VITRI_PORTFOLIO_SEED",
                seed,
                "a non-negative integer seed for the portfolio's goatd candidate",
            )?,
            // The one place a trace level is spelled as text: any value at all
            // turns the trace on, and `all` is the one word that means more.
            trace: match env_raw(
                "VITRI_PORTFOLIO_TRACE",
                "any value to trace every scored candidate, or `all` to also \
                 build and score the candidates the generation gate skips",
            )? {
                Some(raw) if crate::env::is_form(&raw, "all") => TraceLevel::All,
                Some(_) => TraceLevel::Scored,
                None => trace,
            },
            flowcutter_cap_ms: positive_ms(parse(
                "VITRI_PMC_FLOWCUTTER_CAP_MS",
                flowcutter_cap_ms.unwrap_or(0),
                "a wall-clock cap in milliseconds for the projected FlowCutter \
                 candidates (0 = no cap)",
            )?),
            peak_tolerance,
        })
    }
}

/// A millisecond cap read from the environment, where the accepted spelling of
/// "no cap" is `0` and anything that is not a positive duration is no cap.
fn positive_ms(ms: i64) -> Option<i64> {
    (ms > 0).then_some(ms)
}
