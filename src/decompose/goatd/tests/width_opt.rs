//! Test-side entry point into the search shell.
//!
//! `goatd`'s own tests drive single `(config, seed)` pairs straight from raw
//! edges. Production never does — it always goes through `prebuild` +
//! `run_config_prebuilt`, which amortize graph construction and preprocessing
//! across a whole schedule — so the raw-edges entry point lives here, next to
//! the module's private internals it needs.

use crate::decompose::GraphKind;
use crate::decompose::goatd::graph::Graph;
use crate::decompose::goatd::minfill_core::ElimStop;
use crate::decompose::goatd::preprocess::preprocess;
use crate::decompose::goatd::width_opt::*;

/// Run a single `(config, seed)` pair from raw edges. Builds the graph and
/// runs preprocessing.
pub(super) fn run_config(
    kind: GraphKind,
    num_vars: u32,
    edges: &[(u32, u32)],
    total_vertices: u32,
    config: Config<'_>,
    seed: u64,
) -> ConfigRun {
    let graph = Graph::from_edges(total_vertices, edges);
    let reduced = preprocess(graph);
    run_config_on_reduced(
        kind,
        num_vars,
        reduced,
        false,
        None,
        RunSpec {
            config,
            seed,
            stop: ElimStop::default(),
            force_emit: false,
        },
    )
}
