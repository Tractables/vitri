//! Preprocessing over its crate-root-nameable surface. A pass reached only
//! through an item its own module keeps is tested beside that module instead.

mod arjun;
mod backbone_pipeline;
mod simplify_contract;
mod var_map;
