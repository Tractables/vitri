//! goatd — in-house pure-Rust tree-decomposition solver.
//!
//! Min-fill elimination with preprocessing, safe reductions, and FlowCutter's
//! flow-based separators — the last used here both as a schedule slot and as a
//! refinement pass.
//!
//! Every construction starts from the same safe-reduction preprocessing and
//! then picks an elimination order — or, for nested dissection, derives one
//! from a separator recursion. [`width_opt::Config`] holds the five that exist:
//!
//!   * **min-fill** and **min-degree** — the plain greedy orders, ties broken
//!     by a seeded salt.
//!   * **min-fill and min-degree with JW-weighted tie-set sampling** —
//!     fill-only (resp. degree-only) priority, the whole tie set sampled with
//!     `P(v) ∝ w+1` for the degree-normalized Jeroslow-Wang weight `sat_score`
//!     derives. These two are the orders the shipped schedules run.
//!   * **nested dissection** via `multilevel_bisect`, separating on a
//!     König-Egerváry minimum vertex cover (flow-theoretic).

mod build_td;
mod elimination;
mod flow_cut;
mod graph;
mod minfill_core;
mod nested_diss;
mod preprocess;
mod refine;
mod sat_score;
mod schedule;
mod width_opt;

#[cfg(test)]
mod tests;

pub(crate) use elimination::{
    INTERNAL_ELIMINATION_SEED, MINFILL_SPEC, VIEW_SUFFIXES, elimination_order_samples,
    elimination_spec, elimination_spec_names, minfill_td_from_edges, vtree_from_elimination,
    vtree_from_minfill,
};
pub(crate) use schedule::{vtree_from_goatd, vtree_from_goatd_refined};
// Public through `decompose`: it is a field of the public `SelectionCtx`.
pub use schedule::GoatdKnobs;
// Reached only by the goatd unit tests, which pin the schedule's budget
// resolution and its two selector orderings from outside the module. They are
// descendants of this module, so a private import already reaches them.
#[cfg(test)]
use schedule::{ModeConfig, refine_budget_ms, refined_select_key};
