//! The portfolio catalog: what can be built, and under what conditions.
//! [`driver`](super::driver) runs what is described here.
//!
//! Three parts, one per file: what the run is handed ([`Inputs`], and the
//! [`Derived`] measurements a gate reads), what the catalog offers
//! ([`CatalogEntry`] and the gate and build functions its rows point at), and
//! what the walk carries from one entry to the next ([`RunState`], the
//! [`Incumbent`] and the fold that decides it).

mod entry;
// Visible to the portfolio's tests, which check the selection reading on its
// own rather than through a build. Production code reads it through the
// re-exports below.
pub(super) mod inputs;
mod run;

pub(super) use entry::{
    Build, CatalogEntry, Gate, HG_BISECT, HG_BISECT_PARAM, PORTFOLIO_HEAVY_MAX_VARS, TraceRow,
    build_flowcutter, build_force, build_goatd, build_guided_bisect, build_hypergraph_bisect,
    candidate_spec, gate_force, gate_goatd, gate_guided_bisect, gate_hypergraph_bisect,
};
pub(super) use inputs::{Derived, Inputs};
pub(super) use run::{Incumbent, RunState, ScoredCandidate, outspent, work_ms_since};
