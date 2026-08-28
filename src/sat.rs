//! The SAT solver vitri uses, exposed so a consumer does not link a second one.
//!
//! vitri statically links one CaDiCaL: the meelgroup fork the vendored Arjun
//! stack is built against. Two CaDiCaL builds in one process do not coexist.
//! Static-archive linking resolves every `CaDiCaL::` symbol to whichever archive
//! the linker reached first, while the other build's headers are already
//! compiled into its callers' struct layouts — so the program links cleanly, no
//! warning, and then corrupts its heap the first time a call crosses the seam.
//! Link order chooses which build wins; it does not stop one from winning.
//!
//! A consumer that needs a SAT solver in the same process must therefore use
//! *this* one rather than adding a solver crate of its own. `docs/sat.md`
//! records that constraint, and this crate's `links` key is what makes the
//! collision a resolve-time error when the other side declares one too.
//!
//! The handle is deliberately narrow. It is the incremental interface —
//! [`CaDiCal::add`], [`CaDiCal::assume`], [`CaDiCal::solve`], [`CaDiCal::val`]
//! — plus the inprocessing and introspection hooks vitri's own preprocessing
//! needed. It is not a general solver abstraction and does not try to be.
//!
//! ```no_run
//! use vitri::sat::{CaDiCal, Status};
//!
//! let mut solver = CaDiCal::new().expect("a solver");
//! for lit in [1, 2, 0] {
//!     solver.add(lit);
//! }
//! assert_eq!(solver.solve(), Status::Satisfiable);
//! ```

pub use crate::preprocess::cadical::{DeadlineHandle, WallClockTerminator};
pub use crate::preprocess::cadical_ffi::{
    Bounded, CaDiCal, ClauseIterator, SearchStats, Status, Terminator,
};
