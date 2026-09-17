//! What one portfolio build is handed, and the measurements of it a gate reads.
//!
//! [`Inputs`] is assembled once by [`driver`](super::super::driver) and shared
//! by every entry, so no candidate reads the run under a rule of its own.

use crate::candidates::CandidateRankMetric;
use crate::cnf::CnfFormula;
use crate::decompose::goatd::candidate_param;
use crate::decompose::{ConversionRequest, Reading};
use crate::diagnostics::diag;
use crate::score::StructureProfile;
use crate::score::agg::AggModel;

use super::entry::{CatalogEntry, PORTFOLIO_HEAVY_MAX_VARS, candidate_spec};
use super::run::{RunState, work_ms_since};

/// What one portfolio build was asked for.
pub(crate) struct Inputs<'a> {
    pub(crate) formula: &'a CnfFormula,
    /// Optional profile of the source formula. Only its clause-width
    /// dispersion participates in the structure gate; the formula above owns
    /// the occurrence signal.
    pub(crate) source_profile: Option<StructureProfile>,
    pub(crate) seed: u64,
    pub(crate) peak_mode: bool,
    /// show-set mask (var-indexed) for projection-aware peak selection. `None`
    /// = all-var peak (or plain MC).
    pub(crate) show_mask: Option<&'a crate::cnf::ShowMask>,
    pub(crate) trace: bool,
    pub(crate) flowcutter_cap_ms: Option<i64>,
    /// When this build started, read on the construction clock
    /// ([`crate::decompose::meter::now`]) — the reading the one bound measured
    /// from the build's start rather than from its deadline compares against
    /// ([`Inputs::cap_tripped`]). Without a deterministic budget it is the real
    /// start, and that bound is the wall it has always been.
    ///
    /// The wall the driver REPORTS when the build finishes is measured from a
    /// real reading of the same moment, kept in the driver: a report of elapsed
    /// time has to stay one whatever budget the build ran under.
    pub(crate) t_build: std::time::Instant,
    /// Absolute wall-clock deadline for this whole portfolio build. `None` = no
    /// deadline, so every entry runs to completion.
    pub(crate) deadline: Option<std::time::Instant>,
    /// How many candidates the caller asked to have retained for export
    /// ([`crate::candidates`]). `0`/`1` = do not retain: `fold` then keeps
    /// nothing beyond the running best and the selection tail publishes no
    /// candidate set. Never consulted by any adoption rule — this decides what
    /// is kept, never what wins.
    pub(crate) candidate_capacity: usize,
    /// Projected selection's tie band.
    pub(crate) peak_tolerance: f64,
    /// What the goatd entry is configured with.
    pub(crate) goatd: crate::decompose::goatd::GoatdKnobs,
    /// What this run ranks candidates by — both the deferred selection among
    /// them and the order an exported set is published in.
    pub(crate) rank_metric: CandidateRankMetric,
    /// This build's construction-effort multiplier
    /// ([`crate::budget::vtree_effort_scale`]), computed once from the budget
    /// hint on the selection context.
    pub(crate) effort_scale: f64,
    /// Which dimensions of the conversion the run named. Every candidate that
    /// converts a decomposition inherits it, so one run reads every candidate's
    /// decomposition under the same rule.
    pub(crate) reading: Reading,
    /// Whether each candidate's conversion reports every reading it scored.
    pub(crate) conversion_trace: bool,
    /// The caller's candidate preference, already checked against the catalog
    /// by the driver. Read at the end of the build, never by a gate: the
    /// preference decides what is selected, not what is built.
    pub(crate) prefer: Option<&'a super::super::CandidatePreference>,
    /// The ranker this build selects on, or `None` when it selects on the
    /// structural cost alone: `VITRI_SCORE_AGG=cost`, a caller that turned
    /// [`super::super::PortfolioKnobs::ranker`] off, or projected selection. Set,
    /// every candidate is scored by it as well as by the cost and the driver
    /// takes its argmin once the catalog is in; unset, no aggregate is
    /// computed at all.
    pub(crate) score_agg: Option<&'a AggModel>,
}

impl<'a> Inputs<'a> {
    /// What one catalog entry hands the conversion of its decomposition.
    pub(crate) fn conversion(&self, spec: &'static str) -> ConversionRequest<'static> {
        ConversionRequest::of(
            spec,
            self.reading,
            self.effort_scale,
            self.deadline,
            self.conversion_trace,
        )
    }

    /// Whether the tree `entry` offered at `index` is the candidate this build
    /// was asked to prefer. The spec that tree publishes matches, and so does
    /// the bare family name — which names the first tree of the first entry of
    /// that family, since the catalog order decides.
    pub(crate) fn prefers(&self, entry: &CatalogEntry, index: usize) -> bool {
        self.prefer.is_some_and(|p| {
            (index == 0 && p.name() == entry.name)
                || p.name() == candidate_spec(entry.name, candidate_param(index).or(entry.param))
        })
    }
}

/// The structure signals a subset of entries gate on, computed once from
/// the inputs and from the selection as it stood when first needed.
///
/// Not part of [`Inputs`] because the generation gate reads the incumbent
/// vtree, and not part of [`RunState`] because nothing ever revises it: this is
/// a snapshot taken at a defined point in the catalog.
pub(crate) struct Derived {
    /// Whether the formula is coloring-like (near-uniform variable occurrence
    /// and near-uniform clause width). Always `false` above
    /// `PORTFOLIO_HEAVY_MAX_VARS`.
    pub(crate) coloring_like: bool,
    /// The MCL-floor generation gate hypergraph-bisect keeps in plain mode.
    pub(crate) hypergraph_bisect_gen_gate: bool,
}

impl Derived {
    /// Compute the structure gates read. Called at the first
    /// [`Gate::FromDerived`], so the incumbent already reflects the earlier
    /// entries; the result is reused for every later gate, the adoption
    /// test and the trace.
    pub(crate) fn compute(inp: &Inputs, run: &RunState) -> Derived {
        let formula = inp.formula;
        let num_vars = inp.num_vars();
        // Gated on `PORTFOLIO_HEAVY_MAX_VARS` so the O(formula) scan isn't
        // paid above it.
        let coloring_like = if num_vars <= PORTFOLIO_HEAVY_MAX_VARS {
            let profile = StructureProfile::measure(formula);
            let coloring_like = coloring_like_for_selection(profile, inp.source_profile);
            if inp.trace {
                diag!(
                    "[coloring] occ_cv={:.4} width_cv={:.4} source_width_cv={} \
                     coloring_like={} num_vars={num_vars}",
                    profile.var_occurrence_cv,
                    profile.clause_width_cv,
                    inp.source_profile
                        .map(|p| format!("{:.4}", p.clause_width_cv))
                        .unwrap_or_else(|| "none".to_owned()),
                    coloring_like as u8,
                );
            }
            coloring_like
        } else {
            false
        };
        // The adopted candidate was scored when it was folded in, so its worst
        // node is a field rather than another scan.
        let best_mcl = run.best.candidate.as_ref().map(|c| c.stats.max_clause_load);
        Derived {
            coloring_like,
            hypergraph_bisect_gen_gate: best_mcl.is_none_or(|mcl| mcl > formula.num_vars / 5),
        }
    }
}

/// Resolve the portfolio's structure gate from the formula it is building and
/// an optional profile of that formula's source.
///
/// The reduced/built formula remains authoritative for occurrence dispersion.
/// A source profile supplies only an additional clause-width signal. With no
/// source profile, the measured verdict is returned unchanged.
pub(crate) fn coloring_like_for_selection(
    built: StructureProfile,
    source: Option<StructureProfile>,
) -> bool {
    built.coloring_like
        || source.is_some_and(|source| {
            crate::cnf::stats::coloring_like_predicate(
                built.var_occurrence_cv,
                source.clause_width_cv,
            )
        })
}

impl Inputs<'_> {
    /// How many variables the formula this build was handed has.
    pub(crate) fn num_vars(&self) -> u32 {
        self.formula.num_vars
    }

    /// Whether the projected large-component cap has been spent. A DECISION —
    /// it decides whether the goatd entry is attempted at all — so it is
    /// measured in construction work rather than in elapsed time.
    pub(super) fn cap_tripped(&self) -> bool {
        self.flowcutter_cap_ms
            .is_some_and(|cap| (work_ms_since(self.t_build) as i64) > cap)
    }

    /// Milliseconds left before the construction deadline. `None` = no deadline.
    pub(crate) fn remaining_ms(&self) -> Option<i64> {
        self.deadline
            .map(|d| i64::try_from(crate::budget::remaining(d).as_millis()).unwrap_or(i64::MAX))
    }

    /// True once the construction deadline has passed (always false without one).
    pub(crate) fn out_of_time(&self) -> bool {
        self.remaining_ms().is_some_and(|r| r <= 0)
    }

    /// Fair share, in ms, for the next entry when `n_remaining` entries
    /// (including it) are still to be attempted: `remaining / n_remaining`,
    /// floored at 1 ms, so a build already past its deadline still gets a cap
    /// rather than a zero one. Computed at each entry's start, so time a
    /// cheap builder leaves unspent rolls forward to the rest. `None` when
    /// there is no deadline.
    pub(crate) fn fair_share_ms(&self, n_remaining: usize) -> Option<i64> {
        self.remaining_ms()
            .map(|r| (r / n_remaining.max(1) as i64).max(1))
    }
}
