//! Portfolio vtree construction: several backends built for the same formula,
//! one of them selected on a cost score.

mod catalog;
mod driver;

#[cfg(test)]
mod tests;

pub(crate) use driver::vtree_from_portfolio;

use crate::score::Ranker;

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
    /// Caller-owned wall history shared by builds in one retry cascade. A fresh
    /// default isolates an independent request; cloning the handle deliberately
    /// shares the most recent measurement across components and retry rungs.
    pub build_history: PortfolioBuildHistory,

    /// Seed for the goatd candidate (the FlowCutter candidates seed
    /// themselves). `0` is the production setting; a different seed is a cheap
    /// vtree-diversity axis for retry experiments.
    pub seed: u64,

    /// How much of the candidate trace to print. The trace goes through the
    /// diagnostics channel, so it prints only after the consumer has called
    /// [`crate::diagnostics::set_verbose`]; the knob alone prints nothing.
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

    /// Bias selection toward a named candidate. `None` (the default) selects on
    /// score alone.
    ///
    /// The portfolio still builds and scores its whole catalog, still splits the
    /// formula into components, and still falls back the same way; the
    /// preference decides only what happens at the end. A caller retrying a
    /// piece it compiled badly wants exactly that — a DIFFERENT tree from the
    /// same construction, not a different construction.
    ///
    /// The accepted names are [`PortfolioKnobs::candidate_names`]; one the
    /// catalog does not have is refused before anything is built, rather than
    /// spending a construction budget and then selecting on score as if nothing
    /// had been asked for.
    pub prefer: Option<CandidatePreference>,

    /// Built-in catalog entries left out of this build, by base name, read
    /// once and parsed from `VITRI_PORTFOLIO_SKIP`. A name the catalog does
    /// not have is refused, and so is a list that leaves nothing to build.
    /// The default is [`DEFAULT_SKIP`]; empty skips nothing.
    pub skip: Vec<&'static str>,

    /// Which whole-tree aggregate ranker selects this build's candidates.
    /// [`Ranker::Off`] selects on the structural cost with no model read at
    /// all, for a caller that wants the ranker on some of its builds and not
    /// others in one process. `VITRI_SCORE_AGG` fills this in
    /// [`SelectionCtx::with_env_defaults`](crate::decompose::SelectionCtx::with_env_defaults),
    /// which leaves [`Ranker::Off`] off.
    pub ranker: Ranker,

    /// How far above the cost pick's cost a candidate may sit and still be
    /// ranked, in the cost's own units; the default is
    /// [`DEFAULT_MARGIN`](crate::score::DEFAULT_MARGIN). `None` ranks every
    /// candidate; a build with no ranker has no field to narrow and ignores
    /// this. `VITRI_SCORE_AGG_MARGIN` fills it in
    /// [`SelectionCtx::with_env_defaults`](crate::decompose::SelectionCtx::with_env_defaults).
    pub margin: Option<f64>,

    /// Opponent weights used by the pairwise aggregate ranker.
    ///
    /// This affects selection among already constructed candidates. It does
    /// not change construction budgets, the cost margin, or candidate
    /// preferences. It has no effect with a linear ranker, structural-cost
    /// selection, or projected peak selection. No environment variable sets it.
    pub pairwise_weighting: PairwiseWeighting,
}

/// How opponents contribute to a candidate's pairwise ranking score.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PairwiseWeighting {
    /// Every other candidate contributes equally, including candidates from
    /// the same construction family. This is the default.
    #[default]
    Candidate,
    /// Every represented opponent family contributes equally in total.
    ///
    /// A family is a catalog entry's base name, such as `goatd-incidence` or
    /// `flowcutter-primal`. Remove the candidate being scored, then divide
    /// each family's weight equally among its remaining candidates. All
    /// constructed opponents contribute, including identical trees and trees
    /// outside the selection cost margin. A lone candidate scores zero.
    Family,
}

/// How strongly a caller's candidate preference binds. See
/// [`PortfolioKnobs::prefer`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CandidatePreference {
    /// Select this candidate when it was built; otherwise select on score as
    /// usual. For a caller retrying a piece with a different tree that would
    /// rather have the ordinary answer than none.
    Preferred(String),
    /// Select this candidate or fail with
    /// [`VitriError::Construction`](crate::error::VitriError::Construction).
    /// For a caller that needs THIS tree and would rather know than silently be
    /// given another.
    Required(String),
}

impl CandidatePreference {
    /// The candidate this preference names.
    pub fn name(&self) -> &str {
        match self {
            CandidatePreference::Preferred(name) | CandidatePreference::Required(name) => name,
        }
    }

    /// Whether a candidate that did not build is an error rather than a
    /// fallback to score.
    pub fn is_required(&self) -> bool {
        matches!(self, CandidatePreference::Required(_))
    }
}

/// The catalog entries a default build leaves out: goatd on the primal graph
/// and the two recursive bisections. Under the ranker each wins a component
/// now and then and costs a build on every one, and on the model-counting
/// competition benchmarks leaving them out cost no solve.
/// `VITRI_PORTFOLIO_SKIP` replaces the list, an empty value with nothing.
pub const DEFAULT_SKIP: [&str; 3] = ["goatd-primal", "hypergraph-bisect", "guided-bisect"];

impl Default for PortfolioKnobs {
    /// The production configuration: the fixed seed, no trace, no cap, the
    /// tuned tie band, [`DEFAULT_SKIP`] and the ranker on.
    fn default() -> Self {
        PortfolioKnobs {
            build_history: PortfolioBuildHistory::default(),
            seed: 0,
            trace: TraceLevel::Off,
            flowcutter_cap_ms: None,
            peak_tolerance: DEFAULT_PEAK_TOLERANCE,
            prefer: None,
            skip: DEFAULT_SKIP.to_vec(),
            ranker: Ranker::default(),
            margin: Some(crate::score::DEFAULT_MARGIN),
            pairwise_weighting: PairwiseWeighting::default(),
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
    /// Every candidate name [`prefer`](Self::prefer) accepts, in the order the
    /// portfolio builds them.
    ///
    /// Each is also a vtree spec that builds that candidate alone, so a name
    /// read out of a selection record can be handed straight back here. A
    /// candidate built at a parameter is named with it; the bare family name is
    /// accepted too, and names the first entry of that family. An entry that
    /// offers several trees contributes one name per tree, so a runner-up can
    /// be asked for by the name it was published under.
    pub fn candidate_names() -> Vec<String> {
        driver::CATALOG
            .iter()
            .flat_map(|c| c.published_specs())
            .collect()
    }

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
            build_history,
            seed,
            trace,
            flowcutter_cap_ms,
            peak_tolerance,
            prefer,
            skip,
            ranker,
            margin,
            pairwise_weighting,
        } = self;
        // The ranker and its margin are read together: the margin is refused
        // under a ranker that is off, so the choice has to be resolved first.
        // A caller that turned the ranker off for this build keeps it off —
        // no variable turns one back on.
        let ranker = match ranker {
            Ranker::Off => Ranker::Off,
            _ => crate::score::agg::ranker_from_env()?,
        };
        let margin = crate::score::agg::margin_over(margin, ranker != Ranker::Off)?;
        Ok(PortfolioKnobs {
            build_history,
            seed: parse(
                "VITRI_PORTFOLIO_SEED",
                seed,
                "a non-negative integer seed for the portfolio's goatd-incidence candidate",
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
            // A preference is a per-call decision by the caller that is
            // retrying, not a machine-wide setting, so no variable names it.
            prefer,
            skip: match env_raw(
                "VITRI_PORTFOLIO_SKIP",
                "built-in catalog entry names separated by `;`, left out of the portfolio in \
                 place of the default list; empty leaves none out",
            )? {
                Some(raw) => parse_skip_names(&raw)?,
                None => skip,
            },
            ranker,
            margin,
            pairwise_weighting,
        })
    }
}

/// Parse `VITRI_PORTFOLIO_SKIP`'s value into the built-in entries it names,
/// in writing order. Each name is matched against the catalog and what is
/// kept is the catalog's own `&'static str` for it
/// ([`catalog::CatalogEntry::name`]), so nothing read from the
/// environment has to outlive this call.
///
/// Names are separated by `;`, whitespace around each is not part of it, and
/// an empty piece contributes nothing. A name that is not a built-in entry's
/// base name is refused, and so is a list naming every built-in entry, which
/// would leave the portfolio nothing to build.
fn parse_skip_names(raw: &str) -> Result<Vec<&'static str>, crate::error::VitriError> {
    let known: Vec<&'static str> = driver::CATALOG.iter().map(|c| c.name).collect();
    let mut names = Vec::new();
    for piece in raw.split(';') {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        let Some(&name) = known.iter().find(|k| **k == piece) else {
            return Err(crate::error::VitriError::env(
                "VITRI_PORTFOLIO_SKIP",
                format!("{piece:?} is not a built-in catalog entry; the entries are {known:?}"),
            ));
        };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    if known.iter().all(|k| names.contains(k)) {
        return Err(crate::error::VitriError::env(
            "VITRI_PORTFOLIO_SKIP",
            "names every built-in entry, which leaves the portfolio nothing to build",
        ));
    }
    Ok(names)
}

/// Real-wall history for portfolio builds that belong to one caller-owned
/// cascade. Keeping this in the request removes process-order dependence: two
/// independent callers no longer cap one another because they happen to share
/// a process.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PortfolioBuildHistory {
    last_build_ms: std::rc::Rc<std::cell::Cell<u64>>,
    last_winning_spec: std::rc::Rc<std::cell::RefCell<Option<String>>>,
    last_scores: std::rc::Rc<std::cell::Cell<Option<crate::score::VtreeScores>>>,
}

impl PortfolioBuildHistory {
    /// Real milliseconds spent by the most recent build in this history, or
    /// `None` before one completes.
    pub fn last_build_ms(&self) -> Option<u64> {
        match self.last_build_ms.get() {
            0 => None,
            elapsed_ms => Some(elapsed_ms),
        }
    }

    /// The candidate selected by the most recent successful portfolio build in
    /// this history, or `None` before one succeeds.
    pub fn last_winning_spec(&self) -> Option<String> {
        self.last_winning_spec.borrow().clone()
    }

    /// Scores of the most recent successful portfolio winner in this history.
    pub fn last_scores(&self) -> Option<crate::score::VtreeScores> {
        self.last_scores.get()
    }

    fn record(&self, elapsed_ms: u64) {
        self.last_build_ms.set(elapsed_ms);
    }

    fn record_winner(&self, winning_spec: &str, scores: crate::score::VtreeScores) {
        *self.last_winning_spec.borrow_mut() = Some(winning_spec.to_owned());
        self.last_scores.set(Some(scores));
    }
}

/// A millisecond cap read from the environment, where the accepted spelling of
/// "no cap" is `0` and anything that is not a positive duration is no cap.
fn positive_ms(ms: i64) -> Option<i64> {
    (ms > 0).then_some(ms)
}
