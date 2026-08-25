//! Treewidth-based vtree construction.
//!
//! Builds structure-aware vtrees with treewidth solvers and hypergraph
//! partitioners: the narrower the tree decomposition a construction finds, the
//! narrower the separators of the vtree it converts to, which is what
//! [`crate::score`] ranks one vtree against another on.

use std::sync::Arc;

use crate::vtree::Vtree;

mod best;
mod bisect_seed;
mod fm_common;
mod multilevel_bisect;
mod multilevel_hg_bisect;
mod rng;
pub(crate) use multilevel_hg_bisect::IMBALANCE_BALANCED;
mod flowcutter_rs;
mod force;
mod goatd;
mod hybrid;
mod portfolio;

// The individual construction backends: each is one candidate the portfolio
// scores, or one arm of the spec dispatch. A consumer names a backend by spec
// string (through `spec`) or lets `component` select one, so these
// stay crate-internal.
// The config type travels from `spec`'s ONE parse into the constructor, so no
// second place reads the grammar.
pub(crate) use force::{
    ClauseWeight, ForceConfig, ForceMode, InitMode, MAX_DIM as FORCE_MAX_DIM, OrientRule, RootRule,
    WeightRule, vtree_from_force,
};
pub(crate) use goatd::vtree_from_goatd_best;
pub(crate) use goatd::vtree_from_goatd_refined_best;
// The single-order elimination family (`minfill`, `mindegree`, …): the name
// table the spec grammar classifies against, and the construction every one
// of those specs builds — one implementation behind all three.
pub(crate) use goatd::{
    EliminationConversion, INTERNAL_ELIMINATION_SEED, MINFILL_SPEC, VIEW_SUFFIXES,
    elimination_order_samples, elimination_spec, elimination_spec_names, vtree_from_elimination,
    vtree_from_minfill,
};
pub(crate) use multilevel_bisect::vtree_from_primal_bisect;
pub(crate) use multilevel_hg_bisect::vtree_from_hg_bisect;

pub(crate) use portfolio::vtree_from_portfolio;

// The per-backend knob sets the selection context carries. Public because the
// context is: a caller that varies one of these sets the field on the value it
// hands construction, rather than exporting a variable into its own process.
pub use goatd::GoatdKnobs;
pub use portfolio::{PortfolioKnobs, TraceLevel};

/// Explicit selection context threaded through vtree construction: whether the
/// portfolio ranks candidates by ([`SelectionObjective`]): `plain` = ordinary
/// model counting; `peak` = projected counting with the all-var peak;
/// `projected(mask)` = projected counting with the show-aware peak. Threaded by
/// value down the construction call chain. The `Rc` a show mask travels in
/// deliberately keeps it `!Send`, so a projected ctx cannot silently leak into
/// a worker thread: moving a projected build across threads requires explicitly
/// rebuilding the ctx on the far side.
///
/// It also carries the construction RESEARCH KNOBS, and it carries them
/// explicitly: a build whose settings came from the process environment is one
/// an embedder can neither inspect ahead of time nor vary between two
/// concurrent builds. Each backend that has knobs owns its own set, so the
/// context names the backend rather than restating what the knob does; every
/// constructor below leaves them at the production defaults, and
/// [`SelectionCtx::with_env_defaults`] is the one place this crate fills THESE
/// from `VITRI_*` variables. That is a statement about construction only —
/// elsewhere in a run, notably preprocessing, other `VITRI_*` variables are
/// resolved where they apply; `docs/env.md` is the inventory.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectionCtx {
    /// What selection minimizes.
    pub objective: SelectionObjective,

    /// What the portfolio construction is configured with.
    pub portfolio: PortfolioKnobs,

    /// What the goatd schedule is configured with.
    pub goatd: GoatdKnobs,
}

/// What candidate selection ranks by — the three modes a run can be in, and no
/// fourth.
///
/// Peak context width is the projected-counting objective: a hidden variable
/// crossing a cut is ∃-forgotten when its scope completes, so the width that
/// predicts the compile is the one counted over the variables that survive to
/// the root. Which those are is the show mask, and a projected run that has no
/// show set to narrow the count falls back to every variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionObjective {
    /// Clause-load balance: ordinary model counting.
    ClauseBalance,
    /// Peak context width over every variable.
    PeakWidthAll,
    /// Peak context width over the shown variables only.
    PeakWidthShow(std::rc::Rc<crate::cnf::ShowMask>),
}

impl SelectionObjective {
    /// Is the ranking peak context width rather than clause-load balance?
    pub fn is_peak(&self) -> bool {
        !matches!(self, SelectionObjective::ClauseBalance)
    }

    /// The mask the peak is counted over, `None` when it is counted over every
    /// variable or not at all.
    pub fn show_mask(&self) -> Option<&crate::cnf::ShowMask> {
        match self {
            SelectionObjective::PeakWidthShow(mask) => Some(mask),
            _ => None,
        }
    }

    /// The same objective over `mask` instead of whatever mask it carries.
    ///
    /// What a per-component build needs: each component is scored over its own
    /// variables, so the mask is re-derived per component while the choice of
    /// objective is the run's and stays put. A projected run whose component
    /// has no shown variables counts the peak over all of them.
    pub fn with_mask(&self, mask: Option<std::rc::Rc<crate::cnf::ShowMask>>) -> Self {
        match (self, mask) {
            (SelectionObjective::ClauseBalance, _) => SelectionObjective::ClauseBalance,
            (_, Some(mask)) => SelectionObjective::PeakWidthShow(mask),
            (_, None) => SelectionObjective::PeakWidthAll,
        }
    }
}

/// What a construction may SPEND, as opposed to what it should optimize for
/// ([`SelectionCtx`]). Crate-internal and assembled once, in
/// [`crate::component::build_vtree`], from the
/// [`RunConfig`](crate::config::RunConfig) fields that state each of these: a
/// public context carrying them too would be a second place to set the same
/// knob, and the two would eventually disagree.
#[derive(Clone, Debug, Default)]
pub(crate) struct BuildLimits {
    /// ABSOLUTE wall-clock deadline for this whole vtree construction (all
    /// portfolio candidates, all components). `None` = unbounded ⇒ construction
    /// behaves exactly as it always has. `Some(t)` arms the safety net in
    /// `portfolio`: candidates still to be built are skipped once `t` passes,
    /// and each candidate gets a fair share of what is left.
    ///
    /// Derived from the run's deadline by the ONE resolver,
    /// [`RunConfig::construction_deadline`](crate::config::RunConfig::construction_deadline),
    /// which is also where the caller says how much of the run construction may
    /// have. Absolute (not a duration) so it survives being split across
    /// components: the component loop hands each component a share of the SAME
    /// budget.
    pub deadline: Option<std::time::Instant>,
    /// The whole run's wall-clock budget in milliseconds, the hint every
    /// construction-effort dial scales from through
    /// [`crate::budget::vtree_effort_scale`]. `None` is the calibration
    /// baseline: every dial keeps the count it was tuned at.
    ///
    /// Distinct from `deadline`, which cuts construction off at an absolute
    /// instant: this says how long the WHOLE run has, so a build under an
    /// hour-long budget can afford more restarts than the same build under a
    /// two-minute one.
    pub budget_ms: Option<u64>,
    /// How many scored candidates the portfolio should RETAIN for export
    /// ([`crate::candidates`]) — [`RunConfig::candidates`], where `1` (the
    /// default) retains nothing beyond the winner: no candidate is cloned, no
    /// losing vtree is kept alive, and no ranking runs.
    ///
    /// Retention never changes which candidate WINS — the candidate set is
    /// extra output off the one selection path, not a second selector.
    pub candidates: usize,
}

/// What a construction reports when it is handed a formula with nothing to
/// build over. One condition, one sentence, whichever construction hits it.
pub(crate) const EMPTY_FORMULA: &str = "the formula has no variables";

impl SelectionCtx {
    /// Plain model counting: greedy clause-load-balance selection, no show mask.
    pub fn plain() -> Self {
        SelectionCtx {
            objective: SelectionObjective::ClauseBalance,
            portfolio: PortfolioKnobs::default(),
            goatd: GoatdKnobs::default(),
        }
    }

    /// Projected counting, all-var peak: select by peak context width.
    pub fn peak() -> Self {
        SelectionCtx {
            objective: SelectionObjective::PeakWidthAll,
            ..Self::plain()
        }
    }

    /// Projected counting, show-aware peak: the peak metric scores context
    /// width over the shown variables only.
    pub fn projected(mask: std::rc::Rc<crate::cnf::ShowMask>) -> Self {
        SelectionCtx {
            objective: SelectionObjective::PeakWidthShow(mask),
            ..Self::plain()
        }
    }

    /// Fill the research knobs from the `VITRI_*` process environment — THE one
    /// place this crate reads any of them.
    ///
    /// Meant for a program that wants the environment to configure it, which is
    /// this crate's own command-line tool. An embedded caller normally skips it
    /// and sets the fields it cares about directly, so that its behaviour cannot
    /// change because of a variable someone exported in the shell that launched
    /// it. The selection mode is untouched: call it on whichever constructor
    /// above the run needs.
    ///
    /// # Errors
    ///
    /// [`VitriError`](crate::error::VitriError) naming the offending variable
    /// and the form it expects.
    pub fn with_env_defaults(self) -> Result<Self, crate::error::VitriError> {
        Ok(SelectionCtx {
            portfolio: self.portfolio.with_env_defaults()?,
            goatd: self.goatd.with_env_defaults()?,
            ..self
        })
    }

    /// Set the selection mode from `show` over a formula with `num_vars`
    /// variables, keeping every other knob this context carries. `show` must be
    /// over the formula being built, after any per-branch renumbering, so the
    /// mask is always correct for it.
    ///
    /// `None` sets plain selection: selection is show-aware exactly when the
    /// formula it is scoring vtrees for carries a show set. That is the whole
    /// rule — stated here so no consumer restates the "no show set ⇒ plain
    /// selection" branch, and THE one path that installs a show set, so no
    /// consumer sets the mode and the mask as two separate fields.
    pub fn with_show<S: crate::cnf::Space>(
        self,
        show: Option<&crate::cnf::ShowSet<S>>,
        num_vars: u32,
    ) -> Self {
        let objective = match show {
            Some(s) => SelectionObjective::PeakWidthShow(std::rc::Rc::new(s.mask(num_vars))),
            None => SelectionObjective::ClauseBalance,
        };
        SelectionCtx { objective, ..self }
    }

    /// Build the selection ctx for `show` over a formula with `num_vars`
    /// variables, with every other knob at its default.
    pub fn for_show<S: crate::cnf::Space>(
        show: Option<&crate::cnf::ShowSet<S>>,
        num_vars: u32,
    ) -> Self {
        Self::plain().with_show(show, num_vars)
    }
}

mod flowcutter;
// The FlowCutter construction and the three things a spec varies about it. One
// entry for the whole family: `spec` maps a parsed spec onto a graph view, a
// budget and a conversion, and this is what it calls. The decomposition-only and
// separator entries are reached inside `decompose` through their own module
// path, so they are not re-exported here.
// The five effort defaults travel with the budget: what a spec may SAY is the
// grammar's, how hard the search works when a spec says nothing is the
// decomposer's, and the grammar reads them from here.
pub(crate) use flowcutter::{
    Conversion, FC_BARE_TIMEOUT_MS, FC_DEFAULT_ITERS, FC_DEFAULT_STEPS_ITERS, FC_PATIENCE_MS_BARE,
    FC_PATIENCE_MS_PARAMETRIZED, FcBudget, WallCapMode, flowcutter_vtree,
};

mod td_to_vtree;
// The TD→vtree conversion: a caller holding its OWN tree decomposition (from a
// PACE file, or from a solver this crate does not wrap) converts it here, so the
// entry points, the config type and its option enums are public — including the
// formula argument the clause-aware knobs read, which is what makes the public
// conversion the same one this crate's own constructions use.
pub use td_to_vtree::{
    BagAssignment, ItemOrdering, TdRootStrategy, TdToVtreeConfig, VarOrderInBag, td_to_vtree,
    td_to_vtree_configured, td_to_vtree_with_assignment,
};
// The root/ordering sweep that picks among conversions by `score::vtree_cost`,
// and the metadata-returning conversion it is built from.
pub(crate) use td_to_vtree::{
    ConversionInput, td_to_vtree_best, td_to_vtree_best_traced, td_to_vtree_configured_traced,
};
// What a TD→vtree conversion produced beside the tree: the winner's bag
// metadata, returned by the traced entry points above.
pub(crate) use td_to_vtree::TdConversionMeta;

/// A constructed vtree together with what its construction learned about the
/// decomposition behind it.
///
/// The backends the `spec` module dispatches to return this
/// instead of a bare tree so the metadata cannot be paired with the wrong vtree:
/// `td.meta` is `Some` only when it describes exactly `vtree`. A backend that
/// converts no decomposition (balanced, linear, bisection) — or one that scored
/// several conversions and returns a recombination of them rather than any one
/// of them — leaves `td` at its default (no metadata).
pub(crate) struct TdConversion {
    /// The tree the construction selected.
    pub vtree: Arc<Vtree>,
    /// By-products of the TD → vtree conversion that produced `vtree`.
    pub td: TdConversionMeta,
}

impl TdConversion {
    /// A vtree with no decomposition behind it (or none that describes it).
    pub(crate) fn bare(vtree: Arc<Vtree>) -> Self {
        TdConversion {
            vtree,
            td: TdConversionMeta::default(),
        }
    }
}
/// TD bag metadata surviving the TD→vtree conversion. Public: a caller reads
/// the metadata of the vtree it was just handed and learns whether
/// bag-guided clause ordering is available for it.
pub use td_to_vtree::BagMetadata;

mod td_ops;
mod td_parse;
// The tree-decomposition interchange surface, public alongside the conversion
// above: the two graph projections a caller feeds an external decomposer — also
// the argument every backend that decomposes one takes — and the PACE `.gr`/
// `.td` writer and reader.
pub use td_parse::{GraphKind, PaceGraph, TdBag, TreeDecomposition, parse_pace_td};
// The primal edge set behind `GraphKind::build`, which is how a caller outside
// this crate asks for it — paired there with the vertex count that makes it a
// graph. The incidence one has no user beyond that pairing.
pub(crate) use td_parse::build_primal_edges;

// The `subset[i] -> i` renumbering shared by every construction that works on
// a variable subset.
pub(crate) use td_parse::local_index;

mod bisect;
pub(crate) use bisect::{BisectDials, Bisection, BisectionSolver, run_bisection};

/// An upper bound on the treewidth of `formula`'s primal graph once the
/// variables in `conditioned` have been removed from it.
///
/// The primal graph has a vertex per variable and an edge between every pair of
/// variables sharing a clause. Conditioning a variable — fixing it to a constant
/// — deletes its vertex, which is what a caller deciding whether to condition
/// further is asking about. Pass an empty slice for the formula as it stands.
///
/// **An upper bound, from one elimination order.** It is the width of a single
/// min-fill elimination, not the minimum over all orders, and on a large graph
/// the elimination degrades to a cheaper rule rather than running arbitrarily
/// long — so the bound gets looser, never wrong. Compare it against itself
/// across two conditioning choices, which is what it is for. It says nothing
/// about what is or is not compilable: a bound that stays high is a bound this
/// order did not lower, not a fact about the formula.
///
/// Deterministic: the same formula and the same conditioned set give the same
/// number on every machine.
///
/// # Errors
///
/// [`VitriError::Input`](crate::error::VitriError::Input) when `conditioned`
/// names a variable outside the formula's declared universe.
pub fn conditioned_primal_width_ub(
    formula: &crate::cnf::CnfFormula,
    conditioned: &[crate::cnf::VarId],
) -> Result<u32, crate::error::VitriError> {
    let mut removed = vec![false; formula.num_vars as usize];
    for v in conditioned {
        let slot = removed.get_mut(v.idx()).ok_or_else(|| {
            crate::error::VitriError::input(format!(
                "conditioned variable {} is outside the formula's {} declared variables",
                v.to_dimacs(),
                formula.num_vars,
            ))
        })?;
        *slot = true;
    }
    let remaining: Vec<u32> = (0..formula.num_vars)
        .filter(|&v| !removed[v as usize])
        .collect();
    // Nothing left to eliminate: the empty graph has width 0, and the
    // elimination core below is not asked to answer for a graph with no
    // vertices.
    if remaining.is_empty() {
        return Ok(0);
    }
    // The subset form rather than the whole primal graph and a restriction:
    // a formula whose primal graph is too dense to materialize is still cheap
    // to build the clique of each clause over the variables that remain.
    let edges = td_parse::primal_edges_on_subset(formula, &remaining);
    Ok(
        goatd::minfill_td_from_edges(remaining.len() as u32, &edges, INTERNAL_ELIMINATION_SEED)
            .treewidth(),
    )
}

#[cfg(test)]
mod tests;
