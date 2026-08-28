//! The DIMACS reader and writer and the vocabulary they share, over the
//! surface a consumer of the crate sees.

use crate::cnf::*;
use crate::vtree::VarId;

mod components;
mod meta_lines;
mod occ;
mod parse;
mod propagation;
mod round_trip;
mod show_set;
mod stats;
mod types;
mod var_range;

/// Four variables in two clauses sharing none of them: two components under
/// the connectivity rule, and a variable space wide enough that a show set can
/// name a proper subset of it.
const TWO_DISJOINT_CLAUSES: &str = "p cnf 4 2\n1 2 0\n3 4 0\n";
