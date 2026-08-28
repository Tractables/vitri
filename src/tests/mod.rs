//! Tests of the crate's own surface — everything reachable from the crate
//! root — one module per subject.
//!
//! A test that needs an item its own module keeps to itself lives beside that
//! module instead, as `src/<module>/tests/`, where the privacy rules let it
//! in. When a subject is tested from both places, the shape both are worked
//! out over lives here in a `*_fixture` module rather than being written twice;
//! those hold no tests of their own. The `tests/` directory at the repository
//! root is a third place: those
//! are separate crates, so they see vitri exactly as a dependent would, and
//! they are where the binary itself gets driven. The helpers every test
//! reaches for live in one file, `tests/common/mod.rs`, pulled in below and by
//! each of those crates in turn.

mod budget;
mod bundle;
mod candidates;
pub(crate) mod circuit_fixture;
mod cnf;
#[path = "../../tests/common/mod.rs"]
pub(crate) mod common;
mod component;
mod config;
mod decompose;
mod diagnostics;
mod dot;
pub(crate) mod dot_fixture;
mod env;
mod error;
pub(crate) mod learnt_clauses;
pub(crate) mod pmc_oracle;
mod preprocess;
mod projection;
mod sat;
mod score;
pub(crate) mod score_fixture;
mod spec;
pub(crate) mod td_fixture;
mod vtree;
