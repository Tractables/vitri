//! The portfolio catalog: what can be built, and under what
//! conditions. [`driver`](super::driver) runs what is described here.

use crate::candidates::CandidateRankMetric;
use crate::cnf::CnfFormula;
use crate::cnf::stats::{clause_width_cv, coloring_like_predicate, var_occurrence_cv};
use crate::decompose::flowcutter::built_from_td_best;
use crate::decompose::{BagMetadata, FcBudget, GraphKind, TdConversion};
use crate::diagnostics::diag;
use crate::score::{VtreeScores, vtree_max_clause_load};
use crate::vtree::Vtree;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Portfolio: the catalog entries + goatd
// ---------------------------------------------------------------------------

/// Above this var count, skip the bisection-family entries (hypergraph-bisect,
/// hybrid-flowcutter-incidence) and release the held flowcutter-incidence TD
/// early.
pub(super) const PORTFOLIO_HEAVY_MAX_VARS: u32 = 500_000;

/// One built-and-scored candidate, retained past its scoring, for two
/// independent reasons:
/// 1. Projected (`peak_mode`) selection — the whole catalog must be collected
///    before the blended band selection ([`select_peak_band`]) can pick a winner.
/// 2. A caller that asked for an exported candidate set
///    ([`Inputs::candidate_capacity`]). This adds no selection semantics:
///    plain-MC selection never reads `cands`.
pub(super) struct ScoredCandidate {
    /// This candidate's score under the run's ranking metric
    /// ([`Inputs::rank_metric`]). The blended projected selection minimizes it.
    pub(super) sel_metric: f64,
    /// Every metric this candidate scored, carried whole rather than as the two
    /// selection needs, so the retained candidate set can report the same five numbers
    /// the selector saw without recomputing any of them.
    pub(super) stats: VtreeScores,
    pub(super) name: &'static str,
    /// The parameter this candidate was built at, carried beside the name so
    /// the retained set can publish a spec rather than a bare family.
    pub(super) param: Option<&'static str>,
    pub(super) vtree: Arc<Vtree>,
    /// TD bag metadata for this candidate's vtree (`None` for non-TD families).
    /// Carried per candidate so only the selected one's metadata escapes.
    pub(super) meta: Option<Arc<BagMetadata>>,
}

/// The candidate a selection has adopted, and the two scores the adoption rules
/// weigh a challenger against.
///
/// One value rather than six fields, because none of them means anything
/// without the rest: the bag metadata describes THIS tree and no other, and the
/// name and parameter are what would rebuild it. Adoption replaces all six at
/// once, which is what makes "kept in lockstep" a property of the code rather
/// than a warning in a comment.
pub(super) struct Incumbent {
    /// Clause-load standard deviation of `vtree`.
    pub(super) stddev: f64,
    /// Cost score of `vtree`.
    pub(super) cost: u64,
    /// The adopted tree; `None` until something is adopted, which is also what
    /// the scores being at their maxima means.
    pub(super) vtree: Option<Arc<Vtree>>,
    /// TD bag metadata of `vtree`. `None` whenever the incumbent is not
    /// TD-derived.
    pub(super) meta: Option<Arc<BagMetadata>>,
    /// The family that built `vtree`.
    pub(super) name: &'static str,
    /// The parameter behind `name`, so the published winner can be spelled as
    /// the spec that rebuilds it.
    pub(super) param: Option<&'static str>,
}

impl Default for Incumbent {
    /// Nothing adopted: both scores at the maximum, so the first candidate
    /// scored beats it.
    fn default() -> Self {
        Incumbent {
            stddev: f64::MAX,
            cost: u64::MAX,
            vtree: None,
            meta: None,
            name: "none",
            param: None,
        }
    }
}

impl Incumbent {
    /// Take over from whatever was adopted before.
    pub(super) fn adopt(
        &mut self,
        stats: &VtreeScores,
        vtree: Arc<Vtree>,
        meta: Option<Arc<BagMetadata>>,
        name: &'static str,
        param: Option<&'static str>,
    ) {
        *self = Incumbent {
            stddev: stats.clause_load_stddev,
            cost: stats.cost,
            vtree: Some(vtree),
            meta,
            name,
            param,
        };
    }
}

/// One machine-parseable per-candidate trace row (plain-MC tracing only),
/// emitted after selection so `built`/`adopted` reflect the true chain pick.
pub(super) struct TraceRow {
    pub(super) family: &'static str,
    pub(super) param: String,
    pub(super) stddev: f64,
    pub(super) mcl: u32,
    pub(super) peak_context_width_all: u32,
    pub(super) cost: u64,
    pub(super) built: bool,
}

impl TraceRow {
    /// The row a scored vtree reports as. `built` separates the candidates the
    /// chain realized from the ones a score-everything trace generated for
    /// comparison only.
    pub(super) fn from_scores(
        family: &'static str,
        param: String,
        scores: &VtreeScores,
        built: bool,
    ) -> Self {
        Self {
            family,
            param,
            stddev: scores.clause_load_stddev,
            mcl: scores.max_clause_load,
            peak_context_width_all: scores.peak_context_width_all,
            cost: scores.cost,
            built,
        }
    }
}

/// How a built candidate folds into the plain-MC greedy selection. Peak-mode
/// selection ignores this — every candidate is collected and decided by the
/// blended band selection instead.
pub(super) enum AdoptRule {
    MinStddev,
    ColoringGated,
    JointStddevCost,
}

/// One entry of the portfolio catalog as data. The one driver loop runs
/// `gate → build → fold` over the ordered catalog, so extending the portfolio
/// is one more `CatalogEntry`, never a new inline block.
pub(super) struct CatalogEntry {
    /// What a run publishes as the candidate that won, and the `--vtree` base
    /// that builds this construction alone.
    pub(super) name: &'static str,
    /// The parameter that base needs to reproduce THIS build, `None` when the
    /// bare base already does — so `name` and `param` together spell the spec
    /// this run publishes, and the plain-MC trace prints the same parameter in
    /// its own column.
    /// `every_catalog_candidate_names_a_spec_that_rebuilds_it` holds the pair
    /// to the grammar.
    pub(super) param: Option<&'static str>,
    /// Gates both the bag metadata the fold keeps and the verbose per-entry
    /// trace line: only entries built through `td_to_vtree_best` come back
    /// with metadata describing the tree they returned.
    pub(super) td_based: bool,
    pub(super) gate: Gate,
    pub(super) build: fn(&Inputs, &mut RunState) -> Option<TdConversion>,
    pub(super) adopt: AdoptRule,
}

/// The `--vtree` spec that rebuilds one candidate: its name, plus the
/// parameter the name needs to mean THIS build. The one place a published
/// candidate identity is assembled — `winning_spec` and `built_by` are read
/// back as specs, so a name that dropped the parameter it was built at would
/// send its reader to a different tree.
pub(super) fn candidate_spec(name: &str, param: Option<&str>) -> String {
    match param {
        None => name.to_string(),
        Some(p) => format!("{name}:{p}"),
    }
}

/// What an entry's gate is allowed to consult, and therefore when the
/// driver must have the derived signals computed.
pub(super) enum Gate {
    Always,
    FromInputs(fn(&Inputs) -> bool),
    FromDerived(fn(&Inputs, &Derived) -> bool),
}

/// What one portfolio build was asked for.
pub(super) struct Inputs<'a> {
    pub(super) formula: &'a CnfFormula,
    pub(super) seed: u64,
    pub(super) peak_mode: bool,
    /// show-set mask (var-indexed) for projection-aware peak selection. `None`
    /// = all-var peak (or plain MC).
    pub(super) show_mask: Option<&'a crate::cnf::ShowMask>,
    pub(super) trace: bool,
    pub(super) flowcutter_cap_ms: Option<i64>,
    pub(super) t_build: std::time::Instant,
    /// Absolute wall-clock deadline for this whole portfolio build. `None` = no
    /// deadline, so every entry runs to completion.
    pub(super) deadline: Option<std::time::Instant>,
    /// How many candidates the caller asked to have retained for export
    /// ([`crate::candidates`]). `0`/`1` = do not retain: `fold` then keeps
    /// nothing beyond the running best and the selection tail publishes no
    /// candidate set. Never consulted by any adoption rule — this decides what
    /// is kept, never what wins.
    pub(super) candidate_capacity: usize,
    /// Projected selection's tie band.
    pub(super) peak_tolerance: f64,
    /// What the goatd entry is configured with.
    pub(super) goatd: crate::decompose::goatd::GoatdKnobs,
    /// What this run ranks candidates by — both the deferred selection among
    /// them and the order an exported set is published in.
    pub(super) rank_metric: CandidateRankMetric,
    /// This build's construction-effort multiplier
    /// ([`crate::budget::vtree_effort_scale`]), computed once from the budget
    /// hint on the selection context.
    pub(super) effort_scale: f64,
}

/// What the build has produced so far: the running selection accumulators,
/// the retained side tables, and the effort/budget dials the driver re-aims
/// per entry.
pub(super) struct RunState {
    /// FlowCutter step budget for the TD entries.
    pub(super) reduced_steps: i64,
    /// FlowCutter restart breadth, alongside `reduced_steps`.
    pub(super) iters: i32,
    /// This entry's fair share of the remaining budget, in ms; `None` = no
    /// limit, which is the `deadline == None` case. Recomputed by the driver
    /// loop at each entry's start, so a
    /// builder that finishes early rolls its unspent time forward to the rest.
    pub(super) cand_cap_ms: Option<i64>,
    /// Latched once some entry has overrun its own fair share. Until it
    /// latches the build is on schedule and every entry runs its
    /// deterministic path; after it latches the remaining FlowCutter builds
    /// accept a budget cap (see `fc_time_cap_ms`).
    pub(super) behind_schedule: bool,
    pub(super) flowcutter_incidence_td_cache: Option<crate::decompose::TreeDecomposition>,
    /// The candidate plain-MC greedy selection has adopted so far.
    pub(super) best: Incumbent,
    /// Machine-parseable per-candidate trace rows. Populated only when
    /// tracing; fully inert otherwise.
    pub(super) trace_rows: Vec<TraceRow>,
    // Projected (peak_mode) collects every generated candidate and picks via
    // blended selection, rather than greedy argmin.
    pub(super) cands: Vec<ScoredCandidate>,
    /// Whether the hypergraph-bisect family was scored by the chain itself, so the
    /// score-everything trace simulation does not re-score the one imbalance
    /// point production already covered. Only ever set while tracing.
    pub(super) hypergraph_bisect_040_built: bool,
}

/// The structure signals a subset of entries gate on, computed once from
/// the inputs and from the selection as it stood when first needed.
///
/// Not part of [`Inputs`] because `best_mcl` reads the incumbent vtree, and not
/// part of [`RunState`] because nothing ever revises it: a snapshot taken at a
/// defined point in the catalog, held as an `Option` so "never computed" stays
/// distinguishable from any value it could take.
pub(super) struct Derived {
    /// Whether the formula is coloring-like (near-uniform variable occurrence
    /// and near-uniform clause width). Always `false` above
    /// `PORTFOLIO_HEAVY_MAX_VARS`.
    pub(super) coloring_like: bool,
    /// `max_clause_load` of the incumbent vtree, `None` while nothing has been
    /// adopted yet.
    pub(super) best_mcl: Option<u32>,
    /// The MCL-floor generation gate hypergraph-bisect keeps in plain mode.
    pub(super) hypergraph_bisect_gen_gate: bool,
}

impl Derived {
    /// Compute the structure gates read. Called at the first
    /// [`Gate::FromDerived`], so the incumbent already reflects the earlier
    /// entries; the result is reused for every later gate, the adoption
    /// test and the trace.
    pub(super) fn compute(inp: &Inputs, run: &RunState) -> Derived {
        let formula = inp.formula;
        let num_vars = inp.num_vars();
        // Gated on `PORTFOLIO_HEAVY_MAX_VARS` so the O(formula) scan isn't
        // paid above it.
        let coloring_like = if num_vars <= PORTFOLIO_HEAVY_MAX_VARS {
            let occ_cv = var_occurrence_cv(formula);
            let width_cv = clause_width_cv(formula);
            let like = coloring_like_predicate(occ_cv, width_cv);
            if inp.trace {
                diag!(
                    "[coloring] occ_cv={occ_cv:.4} width_cv={width_cv:.4} \
                     coloring_like={} num_vars={num_vars}",
                    like as u8,
                );
            }
            like
        } else {
            false
        };
        let best_mcl = run
            .best
            .vtree
            .as_ref()
            .map(|v| vtree_max_clause_load(v, formula));
        Derived {
            coloring_like,
            best_mcl,
            hypergraph_bisect_gen_gate: best_mcl.is_none_or(|mcl| mcl > formula.num_vars / 5),
        }
    }
}

impl Inputs<'_> {
    /// How many variables the formula this build was handed has.
    pub(super) fn num_vars(&self) -> u32 {
        self.formula.num_vars
    }

    fn cap_tripped(&self) -> bool {
        self.flowcutter_cap_ms
            .is_some_and(|cap| (self.t_build.elapsed().as_millis() as i64) > cap)
    }

    /// Milliseconds left before the construction deadline. `None` = no deadline.
    pub(super) fn remaining_ms(&self) -> Option<i64> {
        self.deadline
            .map(|d| crate::budget::remaining(d).as_millis() as i64)
    }

    /// True once the construction deadline has passed (always false without one).
    pub(super) fn out_of_time(&self) -> bool {
        self.remaining_ms().is_some_and(|r| r <= 0)
    }

    /// Fair share, in ms, for the next entry when `n_remaining` entries
    /// (including it) are still to be attempted: `remaining / n_remaining`,
    /// floored at 1 ms, so a build already past its deadline still gets a cap
    /// rather than a zero one. Computed at each entry's start, so time a
    /// cheap builder leaves unspent rolls forward to the rest. `None` when
    /// there is no deadline.
    pub(super) fn fair_share_ms(&self, n_remaining: usize) -> Option<i64> {
        self.remaining_ms()
            .map(|r| (r / n_remaining.max(1) as i64).max(1))
    }
}

impl RunState {
    pub(super) fn new(reduced_steps: i64, iters: i32) -> RunState {
        RunState {
            reduced_steps,
            iters,
            cand_cap_ms: None,
            behind_schedule: false,
            flowcutter_incidence_td_cache: None,
            best: Incumbent::default(),
            trace_rows: Vec::new(),
            cands: Vec::new(),
            hypergraph_bisect_040_built: false,
        }
    }

    /// Wall cap (ms) to hand a FlowCutter build; `0` = no cap, which is the
    /// deterministic step-budgeted search.
    ///
    /// Deliberately not the fair share on the healthy path: a capped build
    /// takes the vendored C++'s wall-clock entry, which is not a superset of the step-budgeted one — it drops the
    /// `effective_steps` clamp, tightens the min-degree / min-shortcut node
    /// gates (50 000 to 2 000 and 10 000 to 1 000 active nodes), and makes the
    /// produced TD depend on machine load, whether or not the cap is ever
    /// reached. The cap arms only once `behind_schedule` has latched.
    fn fc_time_cap_ms(&self, inp: &Inputs) -> Option<i64> {
        let net = if self.behind_schedule {
            self.cand_cap_ms
        } else {
            None
        };
        match (net, inp.flowcutter_cap_ms) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (cap, None) | (None, cap) => cap,
        }
    }

    /// The budget both FlowCutter entries search under: this run's step and
    /// iteration dials, timed once `fc_time_cap_ms` says the build owes time
    /// back. Without a cap the search is the deterministic step-budgeted one.
    fn fc_budget(&self, inp: &Inputs) -> FcBudget {
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
            },
        }
    }

    /// Wall budget for a goatd build: its fair share, or `None` when there is no
    /// deadline. Unlike the FlowCutter cap this is armed unconditionally — the
    /// goatd schedule and its post-process refinement are anytime by
    /// construction (the lex-min picker keeps the best TD found so far, and both
    /// deadline checks sit between phases), so a budget that never trips leaves
    /// the output unchanged.
    fn goatd_budget_ms(&self) -> Option<u64> {
        self.cand_cap_ms.map(|cap| cap as u64)
    }

    /// Scores a freshly built candidate and folds it into selection — the one
    /// fold for the whole catalog.
    ///
    /// `derived` is whatever the driver has materialized so far — `None` on a
    /// build no `Gate::FromDerived` entry ever reached. Only
    /// `AdoptRule::ColoringGated` reads it, and that rule's own gate is what
    /// materializes `Derived`: an absent `Derived` therefore means the
    /// coloring-like adoption could not have fired anyway.
    pub(super) fn fold(
        &mut self,
        inp: &Inputs,
        derived: Option<&Derived>,
        entry: &CatalogEntry,
        built: TdConversion,
    ) {
        let TdConversion { vtree, td } = built;
        // Only TD-based families' metadata describes the vtree just built;
        // bisection families recombine several conversions, so theirs would
        // describe a different tree.
        let meta = if entry.td_based { td.meta } else { None };
        let formula = inp.formula;
        let stats = VtreeScores::compute(&vtree, formula, inp.show_mask)
            .expect(crate::score::BUILT_FROM_THIS_FORMULA);
        let sel_metric = inp.rank_metric.value(&stats);
        if inp.trace && entry.td_based {
            diag!(
                "[portfolio] cand {:18} stddev={:8.2} peak_ctx={:5} peak_context_width_show={:>5} cost={}",
                entry.name,
                stats.clause_load_stddev,
                stats.peak_context_width_all,
                stats
                    .peak_context_width_show
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                stats.cost,
            );
        }
        // Retained when peak_mode (deferred selection) or an exported candidate
        // set was asked for. At the default (`candidate_capacity <= 1`, every
        // compile-driver call) this costs nothing: no clone, no retained vtree,
        // nothing kept alive past this function.
        if inp.peak_mode || inp.candidate_capacity > 1 {
            self.cands.push(ScoredCandidate {
                sel_metric,
                stats,
                name: entry.name,
                param: entry.param,
                vtree: Arc::clone(&vtree),
                meta: meta.clone(),
            });
        }
        if !inp.peak_mode {
            // Record every candidate for the trace (built=true) before the greedy
            // adoption, so the row exists even for candidates the chain built but
            // did not adopt.
            if inp.trace {
                self.trace_rows.push(TraceRow::from_scores(
                    entry.name,
                    entry.param.unwrap_or("-").to_string(),
                    &stats,
                    true,
                ));
                if entry.name == "hypergraph-bisect" {
                    self.hypergraph_bisect_040_built = true;
                }
            }
            let adopt = match entry.adopt {
                AdoptRule::MinStddev => stats.clause_load_stddev < self.best.stddev,
                AdoptRule::ColoringGated => derived.is_some_and(|d| {
                    d.best_mcl.is_none_or(|mcl| stats.max_clause_load < mcl)
                        && stats.clause_load_stddev < self.best.stddev * 0.9
                        && d.coloring_like
                }),
                AdoptRule::JointStddevCost => {
                    stats.clause_load_stddev < self.best.stddev && stats.cost <= self.best.cost
                }
            };
            if adopt {
                self.best
                    .adopt(&stats, vtree, meta, entry.name, entry.param);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The catalog itself: gate + build free functions, coerced to fn pointers in
// the `CatalogEntry` table in `driver`. Adoption is carried by
// `CatalogEntry::adopt` and run in `fold`.
// ---------------------------------------------------------------------------

/// Catalog entry 1, flowcutter-incidence — FlowCutter incidence TD.
pub(super) fn build_fc_inc(inp: &Inputs, run: &mut RunState) -> Option<TdConversion> {
    let formula = inp.formula;
    run.flowcutter_incidence_td_cache = crate::decompose::flowcutter::flowcutter_td(
        formula,
        GraphKind::Incidence,
        run.fc_budget(inp),
    )
    .ok();
    let vtree = run
        .flowcutter_incidence_td_cache
        .as_ref()
        .map(|td| built_from_td_best(formula, td, inp.effort_scale));
    if inp.num_vars() > PORTFOLIO_HEAVY_MAX_VARS {
        run.flowcutter_incidence_td_cache = None;
    }
    vtree
}

/// Catalog entry 2, flowcutter-primal — FlowCutter primal TD.
pub(super) fn build_fc_pri(inp: &Inputs, run: &mut RunState) -> Option<TdConversion> {
    let formula = inp.formula;
    crate::decompose::flowcutter::flowcutter_td(formula, GraphKind::Primal, run.fc_budget(inp))
        .ok()
        .map(|td| built_from_td_best(formula, &td, inp.effort_scale))
}

/// Catalog entry 3, goatd gate.
pub(super) fn gate_goatd(inp: &Inputs) -> bool {
    if !inp.cap_tripped() {
        true
    } else {
        if inp.trace {
            diag!(
                "[portfolio] cap tripped ({}ms) \u{2192} skip goatd",
                inp.t_build.elapsed().as_millis()
            );
        }
        false
    }
}

/// Catalog entry 3, goatd — goatd incidence-refine.
pub(super) fn build_goatd(inp: &Inputs, run: &mut RunState) -> Option<TdConversion> {
    crate::decompose::goatd::vtree_from_goatd_incidence_refined_best(
        inp.formula,
        inp.seed,
        run.goatd_budget_ms(),
        inp.goatd,
        inp.effort_scale,
    )
    .ok()
}

/// Catalog entry 4, hypergraph-bisect gate.
pub(super) fn gate_hypergraph_bisect(inp: &Inputs, derived: &Derived) -> bool {
    inp.num_vars() <= PORTFOLIO_HEAVY_MAX_VARS
        // Dropping the plain-mode prefilter wouldn't change what's adoptable —
        // it only adds build cost.
        && derived.coloring_like
        && (inp.peak_mode || derived.hypergraph_bisect_gen_gate)
}

/// Catalog entry 4, hypergraph-bisect@0.40.
pub(super) fn build_hypergraph_bisect(inp: &Inputs, _run: &mut RunState) -> Option<TdConversion> {
    let dials = crate::decompose::BisectDials {
        imbalance: crate::decompose::multilevel_hg_bisect::IMBALANCE_PORTFOLIO_RELAXED,
        base_seed: 0,
        effort_scale: inp.effort_scale,
    };
    crate::decompose::multilevel_hg_bisect::vtree_from_hg_bisect(inp.formula, dials)
        .ok()
        .map(TdConversion::bare)
}

/// Catalog entry 5, hybrid-flowcutter-incidence gate.
pub(super) fn gate_hybrid(inp: &Inputs, derived: &Derived) -> bool {
    derived.coloring_like && inp.num_vars() <= PORTFOLIO_HEAVY_MAX_VARS
}

/// Catalog entry 5, hybrid-flowcutter-incidence — reuses the flowcutter-incidence TD.
pub(super) fn build_hybrid(inp: &Inputs, run: &mut RunState) -> Option<TdConversion> {
    let td = run.flowcutter_incidence_td_cache.as_ref()?;
    crate::decompose::hybrid::hybrid_from_incidence_td(inp.formula, td, inp.effort_scale).ok()
}
