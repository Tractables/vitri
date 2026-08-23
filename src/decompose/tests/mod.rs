//! Decomposition machinery tested through items its own modules keep to
//! themselves, at the `decompose` level rather than any one backend's.
//!
//! The decompositions these run over, and the checks they make on one, are
//! shared with the backends' own test trees and live in
//! [`crate::tests::td_fixture`].

mod bisect_seed;
mod td_ops;
