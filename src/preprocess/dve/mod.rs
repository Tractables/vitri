//! Definite Variable Elimination (DVE) — GPMC-style preprocessing.
//!
//! Reimplements GPMC's `Simplify()` loop: an interleaving of equivalence
//! merging (SCC-based; GPMC uses SAT probing), definability-based variable
//! elimination, and clause strengthening (vivification).
//!
//! References:
//! - GPMC: <https://github.com/System-Verification-Lab/GPMC>
//! - Lagniez, Lonca, Marquis. "Improving Model Counting by Leveraging Definability." IJCAI 2016
//! - Korhonen. "Integrating Tree Decompositions..." CP 2021

mod definability;
mod elim;
mod pipeline;
mod strengthen;
pub(super) mod types;

pub(crate) use definability::build_dual_cnf_with_indicators;
pub(crate) use pipeline::{DveConfig, preprocess_dve, preprocess_dve_with_meter};
pub(crate) use strengthen::FrozenEquiv;
#[cfg(test)]
pub(crate) use strengthen::post_dve_strengthen;
pub(crate) use strengthen::post_dve_strengthen_with_meter;

#[cfg(test)]
mod tests;
