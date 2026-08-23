//! The in-process Arjun binding, over the budget policy and the shim
//! `arjun_lib` keeps to itself.

use crate::cnf::Clause;
use crate::cnf::CnfFormula;
use crate::cnf::Literal;
use crate::cnf::VarId;
use crate::error::VitriError;
use crate::preprocess::arjun::ArjunOptions;
use crate::preprocess::arjun_lib::budget_class::*;
use crate::preprocess::arjun_lib::shim::*;
use crate::preprocess::arjun_lib::*;
use num_rational::BigRational;
use std::time::Duration;
use std::time::Instant;

mod anytime;
mod count_preserve;
mod knobs;
mod learnt_clauses;
