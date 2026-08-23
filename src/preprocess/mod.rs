//! CNF preprocessing passes applied before compilation.
//!
//! Every pass here is crate-internal, reached through [`crate::bundle`]; the
//! passes are not a published API and their signatures change with the
//! preprocessing, not with a compatibility promise.
//!
//! Passes fall into three soundness classes, and each pass documents which one
//! it provides:
//! - **Equivalence-preserving** (e.g. unit propagation, equivalence
//!   substitution): preserve the Boolean function exactly. Eliminated variables
//!   are retained as unit clauses so the vtree variable set is unchanged.
//! - **Count-preserving**: preserve the model COUNT (not necessarily the
//!   function) under the instance's counting semantics.
//! - **Projection-aware**: preserve the projected count under the track's
//!   show/projection semantics.
//!
//! The public items are the two variable correspondences a consumer reads out
//! of `preprocess.json` — [`VarMap`], reduced→original, and [`OriginalMap`],
//! original→reduced, whose entries are [`OriginalTarget`] — and [`ArjunSbva`],
//! the bounded-variable-addition policy
//! [`RunConfig`](crate::config::RunConfig) carries for the Arjun stage.

pub(crate) mod arjun;
mod arjun_lib;
mod backbone;
mod backbone_pipeline;
mod bve_project;
/// CaDiCaL driving helpers: the wall-clock terminator and its movable deadline.
mod cadical;
/// Safe bindings to the one CaDiCaL in the process.
mod cadical_ffi;
mod count_preserve;
mod dve;
mod equivalence;
/// Hard wall-clock enforcement for uninterruptible native work: runs a closure
/// in a forked child and SIGKILLs it at its deadline.
mod fork_budget;
/// The codec carrying a forked child's result back across the process boundary.
mod fork_payload;
mod gates;
mod pipelines;
/// Unified probing engine: one CaDiCaL session shared between backbone and
/// equivalence detection, and the only such path there is.
mod probe_engine;
pub(crate) mod projected;
mod renumber;
pub(crate) mod simplify;
mod tarjan;
mod unit_propagation;
mod var_map;
pub(crate) mod weighted_lift;

pub use arjun::ArjunSbva;
pub use var_map::{OriginalMap, OriginalTarget, VarMap};

pub(crate) use arjun_lib::export_learned_clauses_enabled;
pub(crate) use backbone_pipeline::BackboneStats;
pub(crate) use backbone_pipeline::preprocess_backbone_eq_iter;
// What the pipeline above returns, beside it: a caller outside `pipelines` can
// name the type it is handed rather than only the fields it reads off it.
pub(crate) use pipelines::PipelineOutput;

/// The preprocessing knobs a default configuration takes from the environment:
/// the bounded-variable-addition policy and whether the reduce harvests learnt
/// clauses.
///
/// Each variable is read beside the parser that owns its spellings; this is
/// where the two answers are collected, so
/// [`RunConfig::from_env_defaults`](crate::config::RunConfig::from_env_defaults)
/// asks preprocessing once instead of naming preprocessing's variables itself.
///
/// # Errors
///
/// [`VitriError::Env`](crate::error::VitriError::Env) naming whichever variable
/// is set to a value its parser rejects.
pub(crate) fn env_defaults() -> Result<(arjun::ArjunSbva, bool), crate::error::VitriError> {
    Ok((
        arjun::resolve_arjun_sbva()?,
        export_learned_clauses_enabled()?,
    ))
}

#[cfg(test)]
mod tests;
