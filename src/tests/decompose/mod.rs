//! Decomposition over its crate-root-nameable surface. Anything needing a
//! private or `pub(super)` item is tested beside its module instead.

mod bisect;
mod embedding;
mod flowcutter_heap;
mod preference;
mod primal_width;
mod selection_ctx;
mod td_parse;
mod td_to_vtree;
