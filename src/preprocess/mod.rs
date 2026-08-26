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
//! original→reduced, whose entries are [`OriginalTarget`] — and
//! [`ArjunOptions`] with [`ArjunEffort`], [`ArjunSbva`] and [`OracleCaps`],
//! which is what [`RunConfig`](crate::config::RunConfig) carries for the Arjun
//! stage.

pub(crate) mod arjun;
mod arjun_lib;
mod backbone;
mod backbone_pipeline;
mod bve_project;
/// CaDiCaL driving helpers: the wall-clock terminator and its movable deadline.
pub(crate) mod cadical;
/// Safe bindings to the one CaDiCaL in the process.
pub(crate) mod cadical_ffi;
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

pub use arjun::{ArjunEffort, ArjunOptions, ArjunSbva, OracleCaps};
pub use var_map::{OriginalMap, OriginalTarget, VarMap};

pub(crate) use arjun_lib::export_learned_clauses_enabled;
pub(crate) use backbone_pipeline::BackboneStats;
pub(crate) use backbone_pipeline::preprocess_backbone_eq_iter;
// What the pipeline above returns, beside it: a caller outside `pipelines` can
// name the type it is handed rather than only the fields it reads off it.
pub(crate) use pipelines::PipelineOutput;

/// The Arjun options a default configuration takes from the environment.
///
/// Each variable is read beside the parser that owns its spellings; this is
/// where the answers are collected, so
/// [`RunConfig::from_env_defaults`](crate::config::RunConfig::from_env_defaults)
/// asks preprocessing once instead of naming preprocessing's variables itself.
/// A field with no variable — [`OracleCaps::plain`] — keeps its default here.
///
/// # Errors
///
/// [`VitriError::Env`](crate::error::VitriError::Env) naming whichever variable
/// is set to a value its parser rejects.
pub(crate) fn env_defaults() -> Result<ArjunOptions, crate::error::VitriError> {
    let default = ArjunOptions::default();
    Ok(ArjunOptions {
        effort: arjun_lib::resolve_arjun_effort()?,
        sbva: ArjunSbva::from_env()?,
        oracle_max_vars: OracleCaps {
            projected: Some(arjun_lib::projected_oracle_max_vars(
                "VITRI_PMC_ARJUN_ORACLE_MAX_VARS",
            )?),
            weighted_projected: Some(arjun_lib::projected_oracle_max_vars(
                "VITRI_PWMC_ARJUN_ORACLE_MAX_VARS",
            )?),
            ..default.oracle_max_vars
        },
        keep_overrun: arjun_lib::keep_overrun_enabled()?,
        seed: crate::env::parse("VITRI_ARJUN_SEED", default.seed, "a seed, a whole number")?,
        export_learned_clauses: export_learned_clauses_enabled()?,
        ..default
    })
}

#[cfg(test)]
mod tests;
