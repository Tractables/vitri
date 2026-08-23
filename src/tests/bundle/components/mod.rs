//! Per-component bundles.

use crate::bundle::components::*;
use crate::cnf::CnfFormula;
use crate::cnf::ShowSet;
use crate::cnf::{Clause, Literal};
use crate::component::VtreeBuild;
use crate::component::build_vtree;
use crate::config::{ComponentPolicy, RunConfig};
use crate::decompose::SelectionCtx;
use crate::tests::common::Scratch;
use crate::vtree::VarId;
use crate::vtree::Vtree;
use std::path::PathBuf;
use std::sync::Arc;

mod candidates;
mod manifest;
