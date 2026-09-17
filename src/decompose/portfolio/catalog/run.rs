//! What the catalog walk carries from one entry to the next: the running best,
//! the candidates retained for export, and the fold that decides both.
//!
//! One fold, so every candidate meets the incumbent under the same rule
//! whichever entry built it.

use std::sync::Arc;

use crate::decompose::goatd::candidate_param;
use crate::decompose::{BagMetadata, FcBudget, TdConversion, WallCapMode};
use crate::diagnostics::diag;
use crate::score::VtreeScores;
use crate::score::agg::{AggScore, agg_score};
use crate::vtree::Vtree;

use super::entry::{CatalogEntry, HG_BISECT, HG_BISECT_PARAM, TraceRow, candidate_spec};
use super::inputs::Inputs;

/// One built-and-scored candidate, retained past its scoring, for two
/// independent reasons:
/// 1. Projected (`peak_mode`) selection — the whole catalog must be collected
///    before the blended band selection ([`select_peak_band`]) can pick a winner.
/// 2. A caller that asked for an exported candidate set
///    ([`Inputs::candidate_capacity`]). This adds no selection semantics:
///    plain-MC selection never reads `cands`.
#[derive(Clone)]
pub(crate) struct ScoredCandidate {
    /// This candidate's score under the run's ranking metric
    /// ([`Inputs::rank_metric`]). The blended projected selection minimizes it.
    pub(crate) sel_metric: f64,
    /// Every metric this candidate scored, carried whole rather than as the two
    /// selection needs, so the retained candidate set can report the same five numbers
    /// the selector saw without recomputing any of them.
    pub(crate) stats: VtreeScores,
    /// The ranker's score for this candidate — or, for the boosted kind, its
    /// inputs until the component's candidates are all known; `None` when the
    /// build selects on the cost alone.
    pub(crate) agg: Option<AggScore>,
    pub(crate) name: &'static str,
    /// The parameter this candidate was built at, carried beside the name so
    /// the retained set can publish a spec rather than a bare family.
    pub(crate) param: Option<&'static str>,
    pub(crate) vtree: Arc<Vtree>,
    /// TD bag metadata for this candidate's vtree (`None` for non-TD families).
    /// Carried per candidate so only the selected one's metadata escapes.
    pub(crate) meta: Option<Arc<BagMetadata>>,
}

/// The candidate a selection has adopted.
///
/// The whole candidate rather than a handful of fields off it, because none of
/// them means anything without the rest: the bag metadata describes THIS tree
/// and no other, and the name and parameter are what would rebuild it. Adoption
/// replaces one value, which is what makes "kept in lockstep" a property of the
/// code rather than a warning in a comment.
#[derive(Default)]
pub(crate) struct Incumbent {
    /// `None` until something is adopted.
    pub(crate) candidate: Option<ScoredCandidate>,
}

impl Incumbent {
    /// Take over from whatever was adopted before.
    pub(crate) fn adopt(&mut self, candidate: ScoredCandidate) {
        self.candidate = Some(candidate);
    }

    /// The adopted tree's combined cost, or the maximum before anything is
    /// adopted, so the first candidate scored beats it.
    pub(crate) fn cost(&self) -> f64 {
        self.candidate.as_ref().map_or(f64::MAX, |c| c.stats.cost)
    }

    /// Its clause-load standard deviation, under the same rule.
    pub(crate) fn stddev(&self) -> f64 {
        self.candidate
            .as_ref()
            .map_or(f64::MAX, |c| c.stats.clause_load_stddev)
    }

    /// The family that built it, `"none"` before anything is adopted.
    pub(crate) fn name(&self) -> &'static str {
        self.candidate.as_ref().map_or("none", |c| c.name)
    }
}

/// What the build has produced so far: the running selection accumulators,
/// the retained side tables, and the effort/budget dials the driver re-aims
/// per entry.
pub(crate) struct RunState {
    /// FlowCutter step budget for the TD entries.
    pub(crate) reduced_steps: i64,
    /// FlowCutter restart breadth, alongside `reduced_steps`.
    pub(crate) iters: i32,
    /// This entry's fair share of the remaining budget, in ms; `None` = no
    /// limit, which is the `deadline == None` case. Recomputed by the driver
    /// loop at each entry's start, so a
    /// builder that finishes early rolls its unspent time forward to the rest.
    ///
    /// This is the SCHEDULE, not the bound: how much of the budget this entry is
    /// planned to use, and what the anytime goatd builder takes as its budget.
    /// What an entry may not outlive is `cand_wall_ms`.
    pub(crate) cand_cap_ms: Option<i64>,
    /// Hard wall bound, in ms, for the entry being built: it may not outlive the
    /// construction budget it was admitted under. `None` = no deadline.
    ///
    /// Set by the driver loop at each entry's start from the whole time left,
    /// not from the fair share, so an entry that behaves is bounded only by a
    /// wall it never reaches. The deadline is otherwise consulted only between
    /// entries, which cannot stop the one that has already begun — and that is
    /// the entry which overruns the ceiling.
    ///
    /// The one exception is the attempt the driver allows when the deadline is
    /// already spent and nothing has been built: there the share and the wall
    /// are both a fixed short number, because what is left is zero or less.
    pub(crate) cand_wall_ms: Option<i64>,
    /// Latched once some entry has overrun its own fair share, and set outright
    /// for the one attempt a spent deadline allows. Until it latches every entry
    /// is bounded only by the whole remaining budget; after it latches the
    /// remaining FlowCutter builds are additionally tightened to the fair share,
    /// and take the tight search with it (see `fc_time_cap_ms` and
    /// `fc_cap_mode`).
    pub(crate) behind_schedule: bool,
    pub(crate) flowcutter_incidence_td_cache: Option<crate::decompose::TreeDecomposition>,
    /// The candidate plain-MC greedy selection has adopted so far.
    pub(crate) best: Incumbent,
    /// Machine-parseable per-candidate trace rows. Populated only when
    /// tracing; fully inert otherwise.
    pub(crate) trace_rows: Vec<TraceRow>,
    // Projected (peak_mode) collects every generated candidate and picks via
    // blended selection, rather than greedy argmin.
    pub(crate) cands: Vec<ScoredCandidate>,
    /// Whether the hypergraph-bisect family was scored by the chain itself, so the
    /// score-everything trace simulation does not re-score the one imbalance
    /// point production already covered. Only ever set while tracing.
    pub(crate) hypergraph_bisect_040_built: bool,
    /// The preferred candidate, kept as it is scored so the selection tail can
    /// adopt it whatever the scores said. `None` on every build that asked for
    /// no preference, and on one whose preferred candidate did not build —
    /// which are the two cases the tail has to tell apart.
    pub(crate) preferred: Option<ScoredCandidate>,
}

impl RunState {
    pub(crate) fn new(reduced_steps: i64, iters: i32) -> RunState {
        RunState {
            reduced_steps,
            iters,
            cand_cap_ms: None,
            cand_wall_ms: None,
            behind_schedule: false,
            flowcutter_incidence_td_cache: None,
            best: Incumbent::default(),
            trace_rows: Vec::new(),
            cands: Vec::new(),
            hypergraph_bisect_040_built: false,
            preferred: None,
        }
    }

    /// Wall cap (ms) to hand a FlowCutter build; `None` = no cap, which is the
    /// deterministic step-budgeted search.
    ///
    /// Three sources, and the tightest wins:
    /// - `cand_wall_ms`, the time actually left in the construction budget when
    ///   this entry started — or the fixed short wall of the one attempt a spent
    ///   deadline allows, where the time left is zero or less. Under a deadline
    ///   this is always armed, the first entry included, which is what makes the
    ///   budget a ceiling rather than a suggestion.
    /// - `cand_cap_ms`, this entry's fair share, once `behind_schedule` has
    ///   latched. That is the scheduling tightening the latch has always
    ///   applied; it no longer decides whether a cap exists at all.
    /// - the caller's projected large-component cap.
    pub(crate) fn fc_time_cap_ms(&self, inp: &Inputs) -> Option<i64> {
        let share = if self.behind_schedule {
            self.cand_cap_ms
        } else {
            None
        };
        [self.cand_wall_ms, share, inp.flowcutter_cap_ms]
            .into_iter()
            .flatten()
            .min()
    }

    /// Whether the cap `fc_time_cap_ms` hands FlowCutter is expected to bite.
    ///
    /// Tightness changes what the search considers, not only when it stops (see
    /// [`WallCapMode`]), so it is keyed on the two conditions that mean the
    /// build is already in the regime where finishing beats searching:
    /// - `behind_schedule` — some entry has already overrun its fair share, or
    ///   this is the one attempt a spent deadline allows;
    /// - `flowcutter_cap_ms` — the projected large-component cap, whose whole
    ///   purpose is to cut a grinding `flowcutter-primal` short.
    ///
    /// Any other wall is an outer bound the build is expected to finish inside,
    /// and gets a search identical to the unbounded one.
    pub(crate) fn fc_cap_mode(&self, inp: &Inputs) -> WallCapMode {
        if self.behind_schedule || inp.flowcutter_cap_ms.is_some() {
            WallCapMode::Tight
        } else {
            WallCapMode::BoundOnly
        }
    }

    /// The budget both FlowCutter entries search under: this run's step and
    /// iteration dials, timed once `fc_time_cap_ms` says the build owes time
    /// back. Without a cap the search is the deterministic step-budgeted one.
    pub(super) fn fc_budget(&self, inp: &Inputs) -> FcBudget {
        match self.fc_time_cap_ms(inp) {
            None => FcBudget::Steps {
                steps: self.reduced_steps,
                iters: self.iters,
            },
            Some(timeout_ms) => FcBudget::Timed {
                timeout_ms,
                patience_ms: 0,
                iters: self.iters,
                steps: self.reduced_steps,
                cap_mode: self.fc_cap_mode(inp),
            },
        }
    }

    /// Wall budget for a goatd build: its fair share, or `None` when there is no
    /// deadline. Unlike the FlowCutter cap this is armed unconditionally — the
    /// goatd schedule and its post-process refinement are anytime by
    /// construction (the lex-min picker keeps the best TD found so far, and both
    /// deadline checks sit between phases), so a budget that never trips leaves
    /// the output unchanged.
    pub(super) fn goatd_budget_ms(&self) -> Option<u64> {
        self.cand_cap_ms.map(|cap| cap as u64)
    }

    /// Scores a freshly built candidate and folds it into selection — the one
    /// fold for the whole catalog. `index` is the tree's place among what the
    /// entry offered: the first carries the entry's own parameter, the rest are
    /// named by [`candidate_param`]. Any of them can be the preferred
    /// candidate, since any of them can be published as the winner.
    pub(crate) fn fold(
        &mut self,
        inp: &Inputs,
        entry: &CatalogEntry,
        index: usize,
        built: TdConversion,
    ) {
        let TdConversion { vtree, td } = built;
        let param = candidate_param(index).or(entry.param);
        // Only TD-based families' metadata describes the vtree just built;
        // bisection families recombine several conversions, so theirs would
        // describe a different tree.
        let meta = if entry.td_based { td.meta } else { None };
        let formula = inp.formula;
        let (stats, agg) = if let Some(model) = inp.score_agg {
            let (stats, score) = agg_score(&vtree, formula, model, inp.show_mask)
                .expect(crate::score::BUILT_FROM_THIS_FORMULA);
            (stats, Some(score))
        } else {
            (
                VtreeScores::compute(&vtree, formula, inp.show_mask)
                    .expect(crate::score::BUILT_FROM_THIS_FORMULA),
                None,
            )
        };
        let sel_metric = inp.rank_metric.value(&stats);
        if inp.trace && entry.td_based {
            diag!(
                "[portfolio] cand {:18} stddev={:8.2} peak_ctx={:5} peak_context_width_show={:>5} cost={:.2}",
                candidate_spec(entry.name, candidate_param(index)),
                stats.clause_load_stddev,
                stats.peak_context_width_all,
                stats
                    .peak_context_width_show
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                stats.cost,
            );
        }
        let candidate = ScoredCandidate {
            sel_metric,
            stats,
            agg,
            name: entry.name,
            param,
            vtree,
            meta,
        };
        // Kept whatever the mode, and independently of the retained set: plain
        // selection retains no candidate at all, so without this the preference
        // would have nothing left to adopt by the time the catalog is done.
        if self.preferred.is_none() && inp.prefers(entry, index) {
            self.preferred = Some(candidate.clone());
        }
        if !inp.peak_mode {
            // Record every candidate for the trace (built=true) before the greedy
            // adoption, so the row exists even for candidates the chain built but
            // did not adopt.
            if inp.trace {
                self.trace_rows.push(TraceRow::from_scores(
                    entry.name,
                    param.unwrap_or("-").to_string(),
                    &stats,
                    true,
                ));
                // Matched on the pair, not the name alone, so a bare
                // family name cannot stand in for this one point.
                if entry.name == HG_BISECT && entry.param == Some(HG_BISECT_PARAM) {
                    self.hypergraph_bisect_040_built = true;
                }
            }
            if stats.cost < self.best.cost() {
                self.best.adopt(candidate.clone());
            }
        }
        // Retained when the selection waits for the whole catalog: peak_mode,
        // the ranker (which compares the cost pick against its own once every
        // candidate is in), or an exported candidate set. A build selecting on
        // the cost alone with `candidate_capacity <= 1` keeps nothing: no
        // retained vtree, nothing alive past this function.
        if inp.peak_mode || inp.candidate_capacity > 1 || inp.score_agg.is_some() {
            self.cands.push(candidate);
        }
    }
}

/// Milliseconds of construction work done since `start`, measured on the
/// construction clock ([`crate::decompose::meter::now`]).
///
/// The one spelling of that read. Under a deterministic construction budget the
/// clock advances with the work charged rather than with the wall, so a bound
/// expressed through this function is a bound on what the build DOES; without
/// one it is `start.elapsed()` and the bound is the wall it always was. Every
/// portfolio bound that decides how hard the search tries — the projected
/// large-component cap, the behind-schedule latch — is measured with it, so all
/// of them change currency together and none can be left reading the other
/// clock.
pub(crate) fn work_ms_since(start: std::time::Instant) -> u64 {
    crate::decompose::meter::now()
        .saturating_duration_since(start)
        .as_millis() as u64
}

/// This build has less room than the last portfolio build in its caller-owned
/// history actually took.
///
/// `was` is a measurement, not a forecast, and `None` before the first build in
/// the history finishes. A build with more room than the measurement is not
/// gated, so nothing changes on a run whose builds fit the room left. Without a
/// deadline `remaining_ms` is `None` and the gate cannot fire at all.
pub(crate) fn outspent(remaining_ms: Option<i64>, was: Option<u64>) -> bool {
    remaining_ms
        .zip(was)
        .is_some_and(|(left, was)| left > 0 && (left as u64) <= was)
}
