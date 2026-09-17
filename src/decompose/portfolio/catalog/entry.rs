//! What the catalog offers: one row per construction, and the gate and build
//! functions its rows point at.
//!
//! A row names a construction, says when it is worth attempting, and builds it.
//! The table itself is in [`driver`](super::super::driver), which walks it.

use crate::decompose::goatd::candidate_param;
use crate::decompose::{GraphKind, TdConversion, convert_td};
use crate::diagnostics::diag;
use crate::score::VtreeScores;

use super::inputs::{Derived, Inputs};
use super::run::{RunState, work_ms_since};

/// Above this var count, skip the bisection-family entries (hypergraph-bisect,
/// guided-bisect) and release the held flowcutter-incidence TD early.
pub(crate) const PORTFOLIO_HEAVY_MAX_VARS: u32 = 500_000;

/// One machine-parseable per-candidate trace row (plain-MC tracing only),
/// emitted after selection so `built`/`adopted` reflect the true chain pick.
pub(crate) struct TraceRow {
    pub(crate) family: &'static str,
    pub(crate) param: String,
    pub(crate) stddev: f64,
    pub(crate) mcl: u32,
    pub(crate) peak_context_width_all: u32,
    pub(crate) cost: f64,
    pub(crate) built: bool,
}

impl TraceRow {
    /// The row a scored vtree reports as. `built` separates the candidates the
    /// chain realized from the ones a score-everything trace generated for
    /// comparison only.
    pub(crate) fn from_scores(
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

/// The catalog's bisection entry, and the parameter it is built at: the ONE
/// spelling of each. The fold's "was this one already built" test, the catalog
/// table and the trace simulation are three readers of the same entry, and a
/// second spelling would let them disagree about which entry that is.
pub(crate) const HG_BISECT: &str = "hypergraph-bisect";

/// The parameter of [`HG_BISECT`], spelling
/// [`IMBALANCE_PORTFOLIO_RELAXED`](crate::decompose::multilevel_hg_bisect::IMBALANCE_PORTFOLIO_RELAXED).
pub(crate) const HG_BISECT_PARAM: &str = "imbalance=0.40";

/// One entry of the portfolio catalog as data. The one driver loop runs
/// `gate → build → fold` over the ordered catalog, so extending the portfolio
/// is one more `CatalogEntry`, never a new inline block.
pub(crate) struct CatalogEntry {
    /// What a run publishes as the candidate that won, and the `--vtree` base
    /// that builds this construction alone.
    pub(crate) name: &'static str,
    /// The parameter that base needs to reproduce THIS build, `None` when the
    /// bare base already does — so `name` and `param` together spell the spec
    /// this run publishes, and the plain-MC trace prints the same parameter in
    /// its own column. An entry whose build returns several trees names the
    /// first with this; the rest carry [`candidate_param`] of their index.
    /// `every_catalog_candidate_names_a_spec_that_rebuilds_it` holds the pair
    /// to the grammar.
    pub(crate) param: Option<&'static str>,
    /// Gates both the bag metadata the fold keeps and the verbose per-entry
    /// trace line: only entries built through the one conversion come back with
    /// metadata describing the tree they returned.
    pub(crate) td_based: bool,
    pub(crate) gate: Gate,
    /// The most trees this entry's build can offer at once. A build may offer
    /// fewer — the goatd entries offer as many as [`GoatdKnobs::candidates`]
    /// asks for — but never more, so this is how many names the entry
    /// contributes to [`PortfolioKnobs::candidate_names`]. An entry above 1
    /// carries no `param` of its own, since a tree past the first is named by
    /// [`candidate_param`] of its index instead.
    ///
    /// [`GoatdKnobs::candidates`]: crate::decompose::GoatdKnobs::candidates
    /// [`PortfolioKnobs::candidate_names`]: super::super::PortfolioKnobs::candidate_names
    pub(crate) offers: u32,
    /// The trees this entry offers, best first by its own reckoning; empty when
    /// it produced none. Most entries offer one.
    pub(crate) build: Build,
}

impl CatalogEntry {
    /// The trees this entry offers for this build, best first by its own
    /// reckoning. The one place either arm of [`Build`] is called.
    pub(crate) fn offer(&self, inp: &Inputs, run: &mut RunState) -> Vec<TdConversion> {
        match self.build {
            Build::Own(build) => build(inp, run, self),
            Build::OfView(build, view) => build(inp, run, self, view),
        }
    }

    /// Every `--vtree` spec this entry can publish as a winner, in the order it
    /// offers them: its own, then one per tree past the first.
    pub(crate) fn published_specs(&self) -> impl Iterator<Item = String> + '_ {
        (0..self.offers as usize)
            .map(|index| candidate_spec(self.name, candidate_param(index).or(self.param)))
    }
}

/// The `--vtree` spec that rebuilds one candidate: its name, plus the
/// parameter the name needs to mean THIS build. The one place a published
/// candidate identity is assembled — `winning_spec` and `built_by` are read
/// back as specs, so a name that dropped the parameter it was built at would
/// send its reader to a different tree.
pub(crate) fn candidate_spec(name: &str, param: Option<&str>) -> String {
    crate::spec::spec_string(name, param)
}

/// What an entry's gate is allowed to consult, and therefore when the
/// driver must have the derived signals computed.
pub(crate) enum Gate {
    Always,
    FromInputs(fn(&Inputs) -> bool),
    FromDerived(fn(&Inputs, &Derived) -> bool),
}

/// How an entry builds its trees. Both arms are handed the entry, whose `name`
/// is the spec every tree it offers is published under.
pub(crate) enum Build {
    /// A build of this entry's own.
    Own(fn(&Inputs, &mut RunState, &CatalogEntry) -> Vec<TdConversion>),
    /// A build shared with the entry of the other view, which is the only thing
    /// the two differ in.
    OfView(
        fn(&Inputs, &mut RunState, &CatalogEntry, GraphKind) -> Vec<TdConversion>,
        GraphKind,
    ),
}

/// Catalog entries 1 and 2, flowcutter-incidence and flowcutter-primal — a
/// FlowCutter decomposition of the entry's view, converted.
///
/// The incidence decomposition is kept on the run for `guided-bisect`, which
/// guides a bisection with the same one rather than paying for a second. Above
/// [`PORTFOLIO_HEAVY_MAX_VARS`] it is dropped instead: that is where the entries
/// that would read it are gated off.
pub(crate) fn build_flowcutter(
    inp: &Inputs,
    run: &mut RunState,
    entry: &CatalogEntry,
    view: GraphKind,
) -> Vec<TdConversion> {
    let formula = inp.formula;
    let td = crate::decompose::flowcutter::flowcutter_td(formula, view, run.fc_budget(inp)).ok();
    let vtree = td
        .as_ref()
        .map(|td| convert_td(formula, td, inp.conversion(entry.name)));
    if view == GraphKind::Incidence {
        run.flowcutter_incidence_td_cache =
            td.filter(|_| inp.num_vars() <= PORTFOLIO_HEAVY_MAX_VARS);
    }
    vtree.into_iter().collect()
}

/// Gate for both goatd entries: once the cap has tripped there is no time for
/// a scheduled decomposition. Shared, so the two views are admitted on the same
/// condition; it runs once per entry, so a trace shows one line per skip.
pub(crate) fn gate_goatd(inp: &Inputs) -> bool {
    if !inp.cap_tripped() {
        true
    } else {
        if inp.trace {
            diag!(
                "[portfolio] cap tripped ({}ms) \u{2192} skip goatd",
                work_ms_since(inp.t_build)
            );
        }
        false
    }
}

/// Catalog entries 3 and 4, goatd-incidence and goatd-primal — goatd's
/// refinement schedule on the entry's view. Offers as many of the schedule's
/// decompositions as [`GoatdKnobs::candidates`] asks for.
///
/// Both views are in the catalog because they reach different trees: the
/// incidence graph separates a clause from its variables and the primal graph
/// does not, so a formula whose structure survives one projection can be
/// flattened by the other, and which tree scores better is not decidable from
/// the formula. A default build leaves the primal view out
/// ([`DEFAULT_SKIP`](super::super::DEFAULT_SKIP)): its build costs every component a
/// quarter of the construction, and on the model-counting competition
/// benchmarks the trees it wins with are as often larger as smaller than the
/// ranker's next choice.
///
/// [`GoatdKnobs::candidates`]: crate::decompose::GoatdKnobs::candidates
pub(crate) fn build_goatd(
    inp: &Inputs,
    run: &mut RunState,
    entry: &CatalogEntry,
    view: GraphKind,
) -> Vec<TdConversion> {
    crate::decompose::goatd::vtrees_from_goatd_refined(
        inp.formula,
        view,
        inp.seed,
        run.goatd_budget_ms(),
        inp.goatd,
        inp.trace,
        inp.conversion(entry.name),
    )
    .unwrap_or_default()
}

/// Catalog entry 5, force gate.
///
/// FORCE takes no budget — it runs its embedding to completion — so the only
/// bound on it is the formula size, and it is skipped once the cap has tripped
/// for the same reason the goatd entries are.
pub(crate) fn gate_force(inp: &Inputs) -> bool {
    inp.num_vars() <= PORTFOLIO_HEAVY_MAX_VARS && !inp.cap_tripped()
}

/// Catalog entry 5, force — a 2-D FORCE embedding, tree-ified by MST.
///
/// The one entry that is not a converted decomposition. It lays the variables
/// out by attraction to the clauses they share and builds the tree from that
/// layout, so it can beat the conversions on a formula no decomposition
/// separates well — which is the case the rest of the catalog has no answer
/// for.
pub(crate) fn build_force(
    inp: &Inputs,
    _run: &mut RunState,
    _entry: &CatalogEntry,
) -> Vec<TdConversion> {
    let cfg = crate::decompose::ForceConfig::new(crate::decompose::ForceMode::Mst);
    crate::decompose::vtree_from_force(inp.formula, cfg)
        .ok()
        .map(TdConversion::bare)
        .into_iter()
        .collect()
}

/// Catalog entry 6, hypergraph-bisect gate.
pub(crate) fn gate_hypergraph_bisect(inp: &Inputs, derived: &Derived) -> bool {
    inp.num_vars() <= PORTFOLIO_HEAVY_MAX_VARS
        // Dropping the plain-mode prefilter wouldn't change what's adoptable —
        // it only adds build cost.
        && derived.coloring_like
        && (inp.peak_mode || derived.hypergraph_bisect_gen_gate)
}

/// Catalog entry 6, hypergraph-bisect@0.40.
pub(crate) fn build_hypergraph_bisect(
    inp: &Inputs,
    _run: &mut RunState,
    _entry: &CatalogEntry,
) -> Vec<TdConversion> {
    let dials = crate::decompose::BisectDials {
        imbalance: crate::decompose::multilevel_hg_bisect::IMBALANCE_PORTFOLIO_RELAXED,
        base_seed: 0,
        deadline: inp.deadline,
    };
    crate::decompose::multilevel_hg_bisect::vtree_from_hg_bisect(
        inp.formula,
        dials,
        inp.effort_scale,
    )
    .ok()
    .map(TdConversion::bare)
    .into_iter()
    .collect()
}

/// Catalog entry 7, guided-bisect gate.
pub(crate) fn gate_guided_bisect(inp: &Inputs, derived: &Derived) -> bool {
    derived.coloring_like && inp.num_vars() <= PORTFOLIO_HEAVY_MAX_VARS
}

/// Catalog entry 7, guided-bisect — reuses the flowcutter-incidence TD.
pub(crate) fn build_guided_bisect(
    inp: &Inputs,
    run: &mut RunState,
    entry: &CatalogEntry,
) -> Vec<TdConversion> {
    run.flowcutter_incidence_td_cache
        .as_ref()
        .and_then(|td| {
            crate::decompose::guided_bisect_from_incidence_td(
                inp.formula,
                td,
                inp.conversion(entry.name),
            )
            .ok()
        })
        .into_iter()
        .collect()
}
