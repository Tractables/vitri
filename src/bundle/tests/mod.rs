//! Tests of items `bundle` keeps to itself. Everything reachable from the
//! crate root is tested from `src/tests/bundle/` instead; this tree is for the
//! `pub(super)` and `pub(crate)` decisions the chains make internally, which
//! the privacy rules put out of reach from there.

mod chains;
