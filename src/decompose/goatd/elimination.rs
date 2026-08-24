//! The elimination-order registry: the single-order constructions the
//! `--vtree` vocabulary names, and the one body they all run.
//!
//! Every entry point here builds ONE elimination order and stops — no
//! schedule, no refinement, no lex-min pick over candidates. That is
//! [`super::schedule`]'s job, and the two do not call each other.

use std::time::{Duration, Instant};

use crate::cnf::CnfFormula;

use super::super::flowcutter::built_from_td_best;
use super::super::{GraphKind, PaceGraph};
use super::super::{TdConversion, TreeDecomposition};
use super::minfill_core::ElimStop;
use super::{sat_score, width_opt};

/// Per-order construction budget (ms). Soft deadline at this value, hard at 2×.
/// Generous enough for one good elimination on the sampling configs, bounded
/// enough that MinFill on a dense graph can't run for minutes.
const GOATD_ELIMINATION_SOFT_MS: u64 = 10_000;

/// The core an [`ELIMINATION_ORDERS`] row names, given the weight vector.
type EliminationCore = for<'w> fn(&'w [u32]) -> width_opt::Config<'w>;

/// One elimination order, in both of its tie-breaking forms.
///
/// A core is a constructor over the SAT weight vector rather than a bare
/// config, because the JW-weighted cores carry that vector: naming one of them
/// without a weight to hand is not a state this table can express.
struct EliminationOrder {
    /// The order's name, which a `--vtree` base writes with a graph view after
    /// it.
    name: &'static str,
    /// The core that breaks ties deterministically.
    fixed: EliminationCore,
    /// The core that breaks ties by sampling weighted by the SAT-aware
    /// Jeroslow-Wang score, for the orders that have one.
    jw_sample: Option<EliminationCore>,
}

/// The table of single-order elimination constructions: order name → the two
/// elimination cores it runs. The two JW-sampling cores are the pair the
/// shipped schedules also run; the rest are there to be asked for by name.
///
/// Single source of truth for the names — a construction is named in exactly
/// one place. Which graph view an order runs on, and which of its two cores,
/// are grammar that lives in `spec`; this table only names the cores.
const ELIMINATION_ORDERS: &[EliminationOrder] = &[
    EliminationOrder {
        name: MINFILL_ORDER,
        fixed: |_| width_opt::Config::MinFill,
        jw_sample: Some(|weight| width_opt::Config::MinFillSampleJW { weight }),
    },
    EliminationOrder {
        name: "mindegree",
        fixed: |_| width_opt::Config::MinDegree,
        jw_sample: Some(|weight| width_opt::Config::MinDegreeSampleJW { weight }),
    },
    EliminationOrder {
        name: "nested-dissection",
        fixed: |_| width_opt::Config::NestedDissMinCover,
        jw_sample: None,
    },
];

/// The min-fill order's name in [`ELIMINATION_ORDERS`] — what an internal
/// min-fill caller builds through.
const MINFILL_ORDER: &str = "minfill";

/// The `--vtree` spec of that order on the primal graph: the construction a
/// component too small for the portfolio is built with, and what its bundle
/// publishes as the winner. `the_minfill_spec_names_the_minfill_order` holds
/// the two to each other.
pub(crate) const MINFILL_SPEC: &str = "minfill-primal";

/// The graph view a `--vtree` elimination base writes after the order name.
pub(crate) const VIEW_SUFFIXES: [(&str, bool); 2] = [("-primal", false), ("-incidence", true)];

/// The seed the internal min-fill callers use, fixed so those paths are
/// reproducible run to run. Equal to what a bare `minfill` spec resolves to —
/// the spec tests pin that the two produce the same vtree.
pub(crate) const INTERNAL_ELIMINATION_SEED: u64 = 0;

/// Every elimination order's name, in table order. The graph view and the
/// tie-breaking are grammar the spec parser adds on top.
pub(crate) fn elimination_spec_names() -> impl Iterator<Item = &'static str> {
    ELIMINATION_ORDERS.iter().map(|o| o.name)
}

/// Does `name` have a JW-sampling core, or only the deterministic one?
pub(crate) fn elimination_order_samples(name: &str) -> bool {
    ELIMINATION_ORDERS
        .iter()
        .any(|o| o.name == name && o.jw_sample.is_some())
}

/// Split an elimination spec base into its order name and graph view — `None`
/// when the base names no elimination construction.
///
/// The returned name is `'static` (it comes from [`ELIMINATION_ORDERS`]), so an
/// error can name the construction that failed.
pub(crate) fn elimination_spec(base: &str) -> Option<(&'static str, bool)> {
    let (order, incidence) = VIEW_SUFFIXES
        .iter()
        .find_map(|(suffix, incidence)| Some((base.strip_suffix(suffix)?, *incidence)))?;
    let name = elimination_spec_names().find(|n| *n == order)?;
    Some((name, incidence))
}

/// Build a vtree from one elimination-order construction — no schedule, no
/// refinement, no lex-min selection. This is what the `minfill` / `mindegree` /
/// `nested-dissection` specs build, in either graph view.
///
/// `name` is a name from [`ELIMINATION_ORDERS`]; `incidence` picks the graph
/// view; `jw_sample` picks the sampling core over the deterministic one; `seed`
/// drives the randomized tie-breaking either core does.
pub(crate) fn vtree_from_elimination(
    formula: &CnfFormula,
    name: &str,
    incidence: bool,
    jw_sample: bool,
    seed: u64,
    effort_scale: f64,
) -> Result<TdConversion, String> {
    let order = ELIMINATION_ORDERS
        .iter()
        .find(|o| o.name == name)
        .ok_or_else(|| format!("unknown elimination-order construction: {}", name))?;
    let core = if jw_sample {
        order
            .jw_sample
            .ok_or_else(|| format!("{name} breaks ties deterministically only"))?
    } else {
        order.fixed
    };
    let view = if incidence {
        GraphKind::Incidence
    } else {
        GraphKind::Primal
    };
    let PaceGraph {
        num_vertices: total_vertices,
        edges,
        ..
    } = view.build(formula);
    // The JW-weighted cores read this; the others ignore the argument.
    let jw_q = sat_score::compute_weight(formula, total_vertices);
    let td = elimination_td(
        view,
        formula.num_vars,
        total_vertices,
        &edges,
        core(&jw_q),
        seed,
    );
    Ok(built_from_td_best(formula, &td, effort_scale))
}

/// The min-fill construction, for callers that need it without going through
/// a spec string. Exactly what `--vtree minfill` builds at the same seed —
/// one implementation, reached two ways.
pub(crate) fn vtree_from_minfill(
    formula: &CnfFormula,
    seed: u64,
    effort_scale: f64,
) -> Result<TdConversion, String> {
    vtree_from_elimination(formula, MINFILL_ORDER, false, false, seed, effort_scale)
}

/// A min-fill tree decomposition of an already-built graph — the seam for a
/// caller holding an edge list over a local vertex set rather than a formula.
/// Runs the same preprocessing and the same elimination core as
/// [`vtree_from_minfill`]; only the graph comes from somewhere else, and there
/// is no formula to derive a SAT-aware weight from (min-fill does not read one).
pub(crate) fn minfill_td_from_edges(
    num_vertices: u32,
    edges: &[(u32, u32)],
    seed: u64,
) -> TreeDecomposition {
    elimination_td(
        GraphKind::Primal,
        num_vertices,
        num_vertices,
        edges,
        width_opt::Config::MinFill,
        seed,
    )
}

/// Run one elimination config over a graph and return its tree decomposition —
/// the single body behind every single-order entry point above. `num_vars` is
/// how many of the `total_vertices` are variables (they differ on the incidence
/// view, where the clause vertices are dropped from the vtree).
///
/// Bounds construction: the soft deadline triggers the stale-heap fallback;
/// the hard deadline (2×) emergency-bails to a path decomposition, so a TD is
/// always produced fast.
fn elimination_td(
    kind: GraphKind,
    num_vars: u32,
    total_vertices: u32,
    edges: &[(u32, u32)],
    config: width_opt::Config<'_>,
    seed: u64,
) -> TreeDecomposition {
    let prebuilt = width_opt::prebuild(total_vertices, edges);
    let start = Instant::now();
    let soft = Some(start + Duration::from_millis(GOATD_ELIMINATION_SOFT_MS));
    let hard = Some(start + Duration::from_millis(GOATD_ELIMINATION_SOFT_MS.saturating_mul(2)));
    width_opt::run_config_prebuilt(
        kind,
        num_vars,
        &prebuilt,
        width_opt::RunSpec {
            config,
            seed,
            stop: ElimStop {
                deadline: soft,
                hard_deadline: hard,
                width_bound: None,
            },
            // Always produce a valid TD.
            force_emit: true,
        },
    )
    .td
}
