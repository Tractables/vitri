//! Definability-based variable elimination, over the passes `dve` keeps to
//! itself: the definability probe, the elimination round, and the renumbering
//! the pipeline finishes with.

use crate::cnf::VarId;
use crate::cnf::occ::appearance_mask;
use crate::cnf::{Clause, Literal};
use crate::preprocess::dve::definability::{PrimalGraph, pick_def_vars};
use crate::preprocess::dve::elim::{apply_elimination, elim_vars, sort_clause_literals};
use crate::preprocess::dve::pipeline::{preprocess_dve, renumber_formula};
use crate::preprocess::dve::strengthen::FrozenEquiv;
use crate::preprocess::dve::types::DveFate;
use crate::tests::common::make_formula;

mod count_preserve;
mod elimination;
mod primal_graph;
mod renumber;
mod strengthen;
