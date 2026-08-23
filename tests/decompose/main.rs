//! The decomposition path through the public API, one module per subject: the
//! graphs a formula builds, the PACE `.td` text a decomposer's output is read
//! from, and the vtree a decomposition converts to.
//!
//! The imports are here because more than one module uses them; each module
//! states what it is about at its own top.

use std::collections::HashSet;

use vitri::decompose::{
    BagAssignment, GraphKind, ItemOrdering, TdRootStrategy, TdToVtreeConfig, VarOrderInBag,
    parse_pace_td, td_to_vtree, td_to_vtree_configured, td_to_vtree_with_assignment,
};

#[path = "../common/mod.rs"]
mod common;
use common::{assert_covers_all_vars, make_formula, make_td};

mod conversion;
mod graph_building;
mod incidence_graph;
mod td_parsing;
