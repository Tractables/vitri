//! `vitri`: CNF preprocessing and vtree construction for circuit compilation.
//!
//! A raw DIMACS CNF goes in; a reduced CNF, the arithmetic to lift a model count
//! over it back to the original, and a ranked set of vtrees over it come out.
//! Nothing here depends on a particular diagram compiler — the crate is
//! standalone, and a d-DNNF, SDD or tree-decision-diagram (TDD) compiler
//! consumes its output.
//!
//! # Start here
//!
//! Three calls, in order:
//!
//! 1. [`CnfFormula::from_dimacs`] parses the instance.
//! 2. [`run`] preprocesses it and builds the vtree over what preprocessing
//!    left, in the one order those two run in. The returned [`VitriRun`] also
//!    reports the raw input's structural profile; `run` owns that measurement
//!    and uses it for structure-sensitive vtree selection. A caller that needs
//!    to establish the run before beginning this work uses [`frontend`] and
//!    then [`FrontendSession::prepare`]; `run` is that pair in one call.
//! 3. [`VitriRun::write_to_dir`] writes every file the result can name.
//!
//! Those three, and the types they take and hand back, are re-exported at the
//! crate root: the example below names no module.
//!
//! The two halves are also callable on their own — [`bundle::preprocess`] and
//! [`component::build_vtree`] — for a caller that compiles from the values
//! rather than from files.
//!
//! # Process model
//!
//! On unix, [`bundle::preprocess`] runs its budgeted preprocessing stage in a child
//! process created with `fork()`. The stage is a single uninterruptible native
//! call, so the fork is what makes its budget real: the parent `SIGKILL`s a
//! child that outlives the deadline, and the child's page tables bound how much
//! memory the stage can commit to the parent. Everything else runs in the
//! calling process, and the result comes back over a pipe. The two projected
//! reductions are the exception and run inline: what they hold when the
//! deadline passes is the deliverable, so killing them would throw away the
//! answer rather than bound it, and their overrun is bounded by an input-size
//! gate instead.
//!
//! This crate creates no threads, and the fork requires the caller to be
//! single-threaded at that call: a lock held by another of the caller's threads
//! when the fork happens is held forever in the child. On non-unix targets the
//! stage runs inline instead, and the budget bounds only the work between
//! stages.
//!
//! # A worked example
//!
//! The flow the standalone binary is a shell over. Every flag it parses is a
//! field of the one [`RunConfig`], whose `Default` is the
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
//! // A refuted instance is not a missing vtree: its bundle is a synthetic
//! // contradiction with a vtree over it, and this field is the only thing that
//! // says the count is 0.
//! if run.preprocessed.record.unsat {
//!     println!("preprocessing refuted the instance: count(original) = 0");
//! }
//!
//! // Preprocessing can resolve every variable outright, in which case the lift
//! // is the whole answer and there is no vtree — an outcome, not an error.
//! match &run.vtree {
//!     RunVtree::Built(built) => println!("{} vtree nodes", built.vtree.num_nodes()),
//!     RunVtree::FullyResolved => println!("count(original) = the record's lift"),
//! }
//! # Ok(())
//! # }
//! ```
//!
//! It is written against `Box<dyn Error>` because opening the file is
//! [`std::io`]'s failure, not this crate's: every fallible entry point here,
//! the writers included, returns [`VitriError`].
//!
//! # Module reference
//!
//! - [`vtree`]: the vtree *structure* itself — nodes, topology, ordering, LCA,
//!   (de)serialization, and the two rotations
//!   ([`rotate_left`](vtree::rotate::rotate_left) /
//!   [`rotate_right`](vtree::rotate::rotate_right)) a consumer searches vtree
//!   space with under a cost model of its own. Two trees compare with
//!   [`Vtree::same_tree`](vtree::Vtree::same_tree); a rotation renumbers, so
//!   the serialization is not the comparison. The type a consumer compiles
//!   against.
//! - [`cnf`]: the DIMACS `VarId`/`Literal`/`Clause`/`CnfFormula` types +
//!   parser ([`vtree::VarId`] and [`vtree::Literal`] re-export the first two —
//!   one definition).
//! - [`bundle`]: the composite entry point ([`bundle::run`]) and the export
//!   surface — reduced CNF + count-lift record + vtree serialization, i.e.
//!   what the standalone `vitri` binary writes out. A library caller also gets
//!   what the written bundle does not carry: what each preprocessing step did
//!   ([`bundle::StageReport`]) and the count lift split across the steps that
//!   earned it ([`bundle::CountLift`]), plus preprocessing wall/probe telemetry
//!   ([`bundle::PreprocessTelemetry`]). Vtree results likewise report the whole
//!   construction wall on [`component::VtreeBuild::construction_ms`].
//! - [`dot`]: Graphviz rendering of a vtree — the bare structure, or heat-mapped
//!   and labelled from a per-node annotation table the caller fills (this
//!   crate's own clause-load/context-width numbers, or a compiler's own).
//! - [`preprocess`]: the CNF preprocessing passes — this crate's own simplify
//!   chain and Arjun, whose stages `docs/preprocessing.md` lists in order. They
//!   are crate-internal — a caller runs them through [`bundle`], which owns
//!   which chain a counting mode gets.
//!   What it does publish is the vocabulary [`bundle::PreprocessRecord`] is
//!   written in — the variable correspondences and the Arjun policy the config
//!   carries — and the [`preprocess`] module documents which.
//! - [`sat`]: the CaDiCaL handle those passes are built on, published because
//!   a process holds exactly one CaDiCaL — a consumer that adds a second SAT
//!   solver beside this crate links cleanly and then corrupts its heap, so it
//!   uses this one. `docs/sat.md` records the constraint.
//! - [`decompose`]: vtree *construction* heuristics
//!   (treewidth/partition-driven), the CNF-facing counterpart to the vtree
//!   *structure* in [`vtree`]. It also answers one question about a formula
//!   without building anything:
//!   [`conditioned_primal_width_ub`](decompose::conditioned_primal_width_ub),
//!   an upper bound on the primal graph's width after a conditioning choice.
//!   **goatd**, named throughout this crate and in
//!   the `goatd-*` vtree specs, is this crate's own pure-Rust
//!   tree-decomposition solver: min-fill / min-degree elimination with safe
//!   reductions and a refinement pass.
//! - [`score`]: what a vtree is *ranked* on — clause load, context width and
//!   the composite [`vtree_cost`](score::vtree_cost), read off a
//!   `(vtree, formula)` pair without compiling anything, every one of them
//!   lower-is-better. [`VtreeScores`](score::VtreeScores) fuses the five that
//!   selection reads and that an emitted candidate set carries. It depends on
//!   [`vtree`] and [`cnf`] alone, so a consumer can score a vtree of its own
//!   against the same metrics this crate selected by. It also publishes
//!   [`StructureProfile`](score::StructureProfile), the formula-only shape
//!   measurement two decisions in this crate read.
//! - [`spec`], [`component`]: the two orchestration layers over construction —
//!   `spec` turns a `--vtree` spec string into ONE vtree, `component` splits a
//!   formula into independent components, apportions the budget across
//!   them, builds a vtree each, and grafts the result. `component` is the
//!   selection path the standalone tool takes.
//! - [`candidates`]: the ranked set of scored candidate vtrees a portfolio
//!   construction can retain beside its winner.
//! - [`config`]: [`RunConfig`], the explicit configuration the public entry
//!   points take — budget, vtree spec, which preprocessing stages run,
//!   [`SimplifyPolicy`](config::SimplifyPolicy), component handling. One budget
//!   covers the whole run, and
//!   [`ConstructionBudget`](config::ConstructionBudget) says how much of what is
//!   left vtree construction may spend — a share of it by default, the whole of
//!   it for a caller that has already carved the window itself, or a count of
//!   the WORK construction may do
//!   ([`Deterministic`](config::ConstructionBudget::Deterministic)), which is
//!   what makes the vtree it selects the same on every machine. An embedded caller uses
//!   `Default` and sets the fields it needs; nothing it *configures* comes
//!   from the environment unless it asks, through the two opt-in constructors
//!   [`RunConfig::from_env_defaults`](config::RunConfig::from_env_defaults)
//!   and [`SelectionCtx::with_env_defaults`](decompose::SelectionCtx::with_env_defaults).
//!   Configuration is not the whole story: however the config was built, the
//!   vendored stack reads three `VITRI_*` variables of its own with `getenv`,
//!   which this crate validates before any shim exists. A caller that wants a
//!   run sealed off from the shell clears `VITRI_*` from the environment;
//!   `docs/env.md` names every variable and who reads it when.
//!   `VITRI_BUDGET_MS` supplies
//!   [`RunConfig::budget_ms`](config::RunConfig::budget_ms)'s default there,
//!   and is the one variable whose unusable value reads as unset rather than
//!   failing. The rules that scale every internal sub-budget from that field —
//!   which travels to them as an argument — and `env`, which parses every
//!   `VITRI_*` value the crate reads, are crate-internal.
//! - [`diagnostics`]: process-global diagnostics switch — library output is
//!   silent by default; the crate's own binary opts in.
//! - [`error`]: [`VitriError`], the one error type every fallible entry point
//!   here returns. Nothing in this crate exits or aborts the calling process —
//!   a failure comes back as a value.
//!
//! The vendored C/C++ stacks (FlowCutter tree decomposition, CaDiCaL/Arjun
//! preprocessing) and the associated build.rs live here. All are built
//! unconditionally; the crate has no on/off feature surface.

// This is a published library: every public item carries documentation, and
// every intra-doc link resolves. Both are warnings rather than denials so a
// downstream `cargo build` is never broken by a doc lint.
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
pub mod sat;
pub mod score;
pub mod spec;
pub mod vtree;

// The "Start here" flow, at the crate root: the entry point, the types its
// three calls take and hand back, and the error they fail with. Each is
// documented where it is defined — this only shortens the path a consumer
// writes. Nothing else gets a root path; the modules above are the API.
pub use bundle::components::ComponentWriteOptions;
pub use bundle::{FrontendSession, RunPaths, RunVtree, VitriRun, frontend, run};
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
