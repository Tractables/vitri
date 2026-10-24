//! `vitri`: CNF preprocessing and vtree construction for circuit compilation.
//!
//! Compilers of decision diagrams and circuits, SDDs and TDDs among them, take
//! a CNF and a vtree: a binary tree over the formula's variables that fixes the
//! shape of the circuit they build. On the same formula, one vtree compiles in
//! seconds and another does not finish. vitri simplifies the formula, builds
//! several vtrees for what is left and returns the reduced CNF, the vtree that
//! scores best, and the record that lifts a count of the reduced formula back
//! to the original. It depends on no particular compiler.
//!
//! The [counting tutorial](https://github.com/Tractables/vitri/blob/main/docs/getting-started.md)
//! connects vitri to tididi, PySDD and RSDD, from a small CNF to its model
//! count.
//!
//! # Start here
//!
//! Three calls, in order:
//!
//! 1. [`CnfFormula::from_dimacs`] parses the instance.
//! 2. [`run`] preprocesses it and builds the vtree over what preprocessing
//!    left; [`frontend`] and [`FrontendSession::prepare`] are the same run in
//!    two calls, for a caller that establishes the run first.
//! 3. [`VitriRun::write_to_dir`] writes every file the result can name.
//!
//! Those three, and the types they take and hand back, are re-exported at the
//! crate root: the example below names no module.
//!
//! The two halves are also callable on their own — [`bundle::preprocess`] and
//! [`component::build_vtree`] — for a caller that compiles from the values
//! rather than from files.
//!
//! # A worked example
//!
//! The flow the standalone binary is a shell over. Every flag that configures
//! the run is a field of the one [`RunConfig`], whose `Default` is the
//! production configuration.
//!
//! ```no_run
//! use std::fs::File;
//! use std::io::BufReader;
//! use std::path::Path;
//!
//! use vitri::{
//!     CnfFormula, CnfMeta, ComponentWriteOptions, RunConfig, RunPaths, RunVtree, SelectionCtx,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let out_dir = Path::new("bundle");
//!
//! let config = RunConfig { budget_ms: Some(60_000), ..RunConfig::default() };
//! config.validate()?;
//!
//! let reader = BufReader::new(File::open("instance.cnf")?);
//! let (formula, meta): (CnfFormula, CnfMeta) = CnfFormula::from_dimacs(reader)?;
//! let run = vitri::run(&formula, &meta, &config, &SelectionCtx::plain())?;
//! let paths: RunPaths = run.write_to_dir(out_dir, ComponentWriteOptions::default())?;
//! println!("wrote {}", paths.bundle.reduced_cnf.display());
//!
//! // Preprocessing can settle the instance by itself, either by resolving every
//! // variable — the lift is then the whole answer — or by refuting it. Both are
//! // outcomes rather than errors, and neither has a vtree.
//! match &run.vtree {
//!     RunVtree::Built(built) => println!("{} vtree nodes", built.vtree.num_nodes()),
//!     RunVtree::FullyResolved => println!("count(original) = the record's lift"),
//!     RunVtree::Refuted => println!("count(original) = 0"),
//! }
//! # Ok(())
//! # }
//! ```
//!
//! It is written against `Box<dyn Error>` because opening the file is
//! [`std::io`]'s failure, not this crate's: every fallible entry point here,
//! the writers included, returns [`VitriError`].
//!
//! # Process model
//!
//! This crate creates no threads, and nothing in it is known to be safe to
//! enter from two threads at once: a program with threads makes one call into
//! this crate at a time, which the C and Python bindings arrange themselves.
//!
//! On unix, [`bundle::preprocess`] runs its budgeted Arjun stage, one
//! uninterruptible native call, in a child process created with `fork()`. The
//! parent `SIGKILL`s a child that outlives the deadline, the child's memory
//! goes with it, and the result comes back over a pipe. The stage forks only
//! when the process has one thread (it counts them on Linux and macOS) and
//! `SIGCHLD` is neither ignored nor installed with `SA_NOCLDWAIT`; anywhere
//! else it runs inline, and the budget bounds only the work between stages.
//! The projected reductions always run inline: what they hold at the deadline
//! is the answer, and an input-size gate bounds their overrun. A `SIGCHLD`
//! handler that waits for every child can reap the child before the deadline's
//! kill, without losing the result. A caller that needs a hard limit runs the
//! whole call in a process it can kill, such as the `vitri` executable.
//!
//! # Module reference
//!
//! - [`vtree`]: the vtree structure a consumer compiles against, with the two
//!   rotations in [`vtree::rotate`] for a consumer searching vtree space under
//!   a cost model of its own.
//! - [`cnf`]: the DIMACS `VarId`/`Literal`/`Clause`/`CnfFormula` types and
//!   parser; [`vtree::VarId`] and [`vtree::Literal`] re-export the first two.
//! - [`bundle`]: the composite entry point ([`bundle::run`]) and the export
//!   surface, what the `vitri` binary writes out, plus what the written bundle
//!   does not carry ([`bundle::PreprocessBundle`]).
//! - [`request`]: one run as plain values, with a JSON form for the language
//!   bindings.
//! - [`dot`]: Graphviz rendering of a vtree, bare or annotated per node.
//! - [`preprocess`]: crate-internal; a caller runs the passes through
//!   [`bundle`], which owns which chain a counting mode gets, and
//!   `docs/preprocessing.md` lists the stages in order. What it publishes is
//!   the vocabulary [`bundle::PreprocessRecord`] is written in.
//! - [`projection`]: projection-safe operations for formulas a consumer derives
//!   after the main preprocessing run.
//! - [`sat`]: the CaDiCaL handle this crate links, published because a process
//!   holds exactly one CaDiCaL.
//! - [`decompose`]: vtree construction, and
//!   [`conditioned_primal_width_ub`](decompose::conditioned_primal_width_ub),
//!   a bound on the primal graph's width after a conditioning choice.
//!   **goatd**, named throughout this crate and in the `goatd-*` vtree specs,
//!   is the `goatd` crate: a tree-decomposition solver — elimination orders,
//!   FlowCutter and multilevel bisection — with a refinement pass.
//! - [`score`]: what a vtree is ranked on, read off a `(vtree, formula)` pair
//!   without compiling anything; it depends on [`vtree`] and [`cnf`] alone, so
//!   a consumer can score a vtree of its own against the same metrics.
//! - [`spec`], [`component`]: the two orchestration layers over construction —
//!   `spec` turns a `--vtree` spec string into one vtree, `component` splits a
//!   formula into independent components, builds a vtree each and grafts the
//!   result. `component` is the path the standalone tool takes.
//! - [`candidates`]: the ranked candidate set a portfolio build can retain.
//! - [`config`]: [`RunConfig`], the explicit configuration the public entry
//!   points take; the environment is read only through
//!   [`RunConfig::from_env_defaults`](config::RunConfig::from_env_defaults),
//!   [`SelectionCtx::with_env_defaults`](decompose::SelectionCtx::with_env_defaults)
//!   and [`ArjunSbva::from_env`](preprocess::ArjunSbva::from_env), and
//!   `docs/env.md` names every variable and who reads it when.
//! - [`diagnostics`]: process-global diagnostics switch, silent by default.
//! - [`error`]: [`VitriError`], the one error type every fallible entry point
//!   here returns; nothing in this crate exits or aborts the calling process.
//!
//! The vendored C/C++ stack (CaDiCaL and Arjun) and the build script that
//! compiles it live here, and are built unconditionally: the crate has no
//! on/off feature surface.

// This is a published library: every public item carries documentation, and
// every intra-doc link resolves. Both are warnings rather than denials so a
// downstream `cargo build` is never broken by a doc lint.
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/Tractables/vitri/assets/logo/vitri-logo.svg"
)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]
#![warn(rustdoc::private_intra_doc_links)]
// `pub` on an item no path outside the crate can name says "part of the API"
// to a reader and means `pub(crate)` to the compiler. This keeps the two
// readings the same.
#![warn(unreachable_pub)]

pub(crate) mod budget;
pub mod bundle;
pub mod candidates;
pub mod cnf;
pub mod component;
pub mod config;
pub mod decompose;
pub mod diagnostics;
pub mod dot;
pub(crate) mod env;
pub mod error;
pub mod preprocess;
pub mod projection;
pub mod request;
pub mod sat;
pub mod score;
pub mod spec;
pub mod vtree;

// The "Start here" flow, at the crate root: the entry point, the types its
// three calls take and hand back, and the error they fail with. Each is
// documented where it is defined — this only shortens the path a consumer
// writes. Nothing else gets a root path; the modules above are the API.
pub use bundle::components::ComponentWriteOptions;
pub use bundle::{
    FrontendRetryConfig, FrontendSession, RetryBudget, RunPaths, RunVtree, VitriRun, frontend, run,
};
pub use cnf::{CnfFormula, CnfMeta};
pub use config::{DvePolicy, RunConfig, SimplifyPolicy};
pub use decompose::SelectionCtx;
pub use error::VitriError;

/// `num_rational`, re-exported because [`BigRational`](num_rational::BigRational)
/// appears in this crate's public weight API ([`cnf::WeightTable`]) — a consumer
/// reads those rationals through `vitri::num_rational` instead of depending on
/// the crate separately and having to match the version this one resolved.
pub use num_rational;

// The shared test helpers are written against the public API, under the crate's
// own name, so that one file serves both the unit tests here and the separate
// crates in `tests/`. This is what lets it name `vitri::…` from inside `vitri`.
#[cfg(test)]
extern crate self as vitri;

#[cfg(test)]
mod tests;
