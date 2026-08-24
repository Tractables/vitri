//! Vtree specification strings: what they parse to, and what they build.
//!
//! Shared by the parsing topics: [`parse_ok`], which parses a spec and fails
//! the case rather than the assertion if it does not parse at all.

use crate::cnf::CnfFormula;
use crate::cnf::{Clause, Literal};
use crate::decompose::BagAssignment;
use crate::decompose::BuildLimits;
use crate::decompose::ItemOrdering;
use crate::decompose::SelectionCtx;
use crate::decompose::TdRootStrategy;
use crate::decompose::TdToVtreeConfig;
use crate::decompose::VarOrderInBag;
use crate::spec::*;
use crate::vtree::VarId;

mod build;
mod conversion;
mod params;
mod vocabulary;

fn parse_ok(spec: &str) -> ParsedSpec<'_> {
    parse_vtree_spec(spec).unwrap_or_else(|e| panic!("{spec} must parse: {e}"))
}
