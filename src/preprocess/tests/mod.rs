//! Preprocessing passes tested through items their own modules keep to
//! themselves. A pass with its own directory carries its tests there instead;
//! what reaches only the crate root is tested from `src/tests/preprocess/`.

mod bve_project;
mod cadical;
mod cadical_ffi;
mod count_preserve;
mod equivalence;
mod fork_budget;
mod gates;
mod pipelines;
mod probe_engine;
mod projected;
mod renumber;
mod simplify;
mod tarjan;
mod unit_propagation;
mod weighted_lift;
