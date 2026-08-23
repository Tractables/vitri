//! Diagnostic output control.
//!
//! This is a published library: an arbitrary consumer calls into it (e.g.
//! [`crate::bundle::preprocess`]) as one step inside their
//! own program, and a library must not write to that consumer's stderr
//! uninvited — the caller did not ask for progress chatter and has no say over
//! its format. So the default here is QUIET: the crate-internal `diag!` macro
//! is a no-op unless a caller explicitly opts in via [`set_verbose`].
//!
//! # What the prefixes mean
//!
//! Every line this channel carries opens with the name of what emitted it, in
//! one of two shapes:
//!
//! - `c note: …` — the export chain's running commentary on the file it is
//!   producing: what preprocessing did, or declined to do and why (`skipping
//!   arjun (stage disabled)`, `reverting dve (…)`). `c ` is the DIMACS comment
//!   marker, so such a line pastes into the CNF unedited.
//! - `[<stage>] …` — everything else: one named stage reporting on itself, its
//!   name in lower case and in brackets — `[portfolio]` for candidate
//!   construction and selection, `[arjun-anytime]` for the anytime Arjun
//!   wrapper, `[dve-budget]` for the DVE stage reaching its time limit. A
//!   consumer can therefore split a line on its first `]`.
//!
//! A prefix names the emitter, never a severity: nothing here is a failure
//! report. A failure comes back to the caller as a [`crate::VitriError`] and is
//! never announced on this channel. The `error: …` line and the pointer under
//! it that a user sees from the standalone binary are the BINARY's own stderr,
//! written where an error becomes an exit code — no `diag!` produces them.

use std::sync::atomic::{AtomicBool, Ordering};

static VERBOSE: AtomicBool = AtomicBool::new(false);

/// Turn library diagnostics on or off for the rest of this process. Returns
/// the previous value, so a caller that wants the switch back can restore it.
pub fn set_verbose(on: bool) -> bool {
    VERBOSE.swap(on, Ordering::Relaxed)
}

/// Whether library diagnostics (the crate-internal `diag!` macro) are
/// currently enabled.
pub fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

/// Emit a library diagnostic to stderr, but only when diagnostics are
/// enabled (see the module doc). `crate`-internal only — a library consumer
/// never wants to CALL this, only toggle it via [`set_verbose`].
macro_rules! diag {
    ($($arg:tt)*) => { if $crate::diagnostics::verbose() { eprintln!($($arg)*); } };
}
pub(crate) use diag;
