//! The decomposition path through the public API, one module per subject: the
//! graphs a formula builds, and the PACE `.td` text a decomposer's output is
//! read from.
//!
//! The imports are here because more than one module uses them; each module
//! states what it is about at its own top.

use std::collections::HashSet;

use vitri::decompose::{GraphKind, parse_pace_td};

#[path = "../common/mod.rs"]
mod common;
use common::make_formula;

mod graph_building;
mod incidence_graph;
mod td_parsing;
