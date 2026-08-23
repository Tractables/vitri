//! Soundness gate for the export bundle, in every mode.
//!
//! The counts on both sides come from the crate's own brute-force oracles
//! ([`crate::tests::pmc_oracle`]) — ground truth, not hand-written constants
//! — and the reduced side is counted over the file that was actually WRITTEN and
//! re-parsed, so the DIMACS writer, the `c t` / `c p show` / `c p weight`
//! carry-forward and the record all sit inside the assertion rather than beside
//! it.
//!
//! The hand-written cases below pin specific structures (backbone, equivalences,
//! free vars, definitions, UNSAT); [`property`] then sweeps randomized small
//! instances through every mode, which is what catches the composition bugs
//! a fixed corpus never reaches.

use super::common::{Scratch, parse, rat};
use crate::bundle::*;
use crate::cnf::CnfFormula;
use crate::cnf::CnfMeta;
use crate::cnf::Mode;
use crate::cnf::rational_string;
use crate::cnf::{Clause, Literal};
use crate::cnf::{DimacsHeader, write_dimacs};
use crate::cnf::{Original, Reduced, ShowSet, VarId, Weights};
use crate::config::RunConfig;
use crate::preprocess::OriginalTarget;
use crate::tests::pmc_oracle::{brute_force_mc, brute_force_pmc, brute_force_pwmc};
use num_bigint::BigUint;
use num_rational::BigRational;

mod arjun;
mod compile;
mod components;
mod harness;
mod mc;
mod mode_selection;
mod projected;
mod property;
mod run;
mod stages;
mod weighted;
mod writer;

use harness::*;
