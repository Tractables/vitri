//! [`VitriError`] — the one error type every fallible entry point in this crate
//! returns.
//!
//! # Why one type
//!
//! This is a published library, called from inside somebody else's program. A
//! failure has to come back as a value that program can inspect and report; it
//! must never end that program. So nothing on a library path exits, aborts, or
//! writes to the caller's stderr — the failure travels back as a `VitriError`
//! and the caller decides. The crate's own binary (`cli_main`) is the single
//! place an error becomes a message on stderr and a process exit code.
//!
//! # Why these variants
//!
//! One variant per *thing the caller can do about it*, not one per module or
//! per call site. A [`Spec`](VitriError::Spec) error means the spec string
//! needs fixing; an [`Env`](VitriError::Env) error means a variable in the
//! environment needs fixing; a [`Construction`](VitriError::Construction) error means the
//! request was well formed and construction still could not answer it. Where no
//! caller could branch on more structure than the sentence itself, the variant
//! carries the sentence.
//!
//! One house style for that sentence, so a reader learns it once: the offending
//! token is Debug-quoted, which is what makes a stray space or an empty token
//! visible in it; a form that would be accepted follows as `expected …`, listing
//! a closed vocabulary in full; and the sentence carries no trailing period,
//! since a caller may be embedding it in one of their own.
//!
//! [`crate::diagnostics`] is an opt-in trace, quiet by default, and never the
//! error channel: every error returned from here states its whole case on its
//! own, so a caller that never turns diagnostics on still gets the full story.

use std::fmt;
use std::path::PathBuf;

/// Everything this crate's public API can fail with.
///
/// The `Display` text is the complete message: it names the offending spec,
/// variable, backend or path, so a caller can print it unchanged. The fields
/// are the same information in structured form, for a caller that wants to
/// branch on it rather than print it.
///
/// `#[non_exhaustive]`: a caller matching on this must carry a `_` arm, so a
/// failure this crate learns to distinguish later reaches that caller as an
/// error it already handles rather than as a build break. `Display` covers the
/// new case from the first release that has it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum VitriError {
    /// The request is malformed — a [`RunConfig`](crate::config::RunConfig)
    /// field or, for the binary, a command-line argument. A retained candidate
    /// set asked of a construction that builds one vtree, a candidate count
    /// above the ceiling, a mode whose preprocessing needs a declaration the
    /// instance does not carry, an unknown option, a flag given without its
    /// value. Fix the request and call again.
    Config {
        /// What is wrong with the request, in full.
        reason: String,
    },

    /// A `--vtree` spec string names a construction this crate does not have,
    /// or carries a parameter the named family cannot honor. A parameter is
    /// never dropped silently — an inert one is this error. Fix the spec
    /// string.
    Spec {
        /// The spec exactly as the caller wrote it.
        spec: String,
        /// What is wrong with it, and which form would be accepted.
        reason: String,
    },

    /// A `VITRI_*` environment variable holds a value this crate will not guess
    /// at. Unset the variable or give it one of the values the message lists.
    Env {
        /// The variable's name.
        var: &'static str,
        /// What is wrong with the value, and which values are accepted.
        reason: String,
    },

    /// The data handed in cannot be used: a vtree file that does not parse or
    /// does not match the formula, a CNF with nothing to build over. Fix the
    /// input.
    Input {
        /// What is wrong with the input, in full.
        reason: String,
    },

    /// Two arguments of one call that have to be about the same formula are
    /// not: a vtree build handed to a writer along with a formula it was not
    /// made from. Nothing is wrong with either of them on its own, and no data
    /// needs fixing — the call does.
    Mismatch {
        /// Which two things disagree, and how, in full.
        reason: String,
    },

    /// A vtree construction was asked a well formed question and could not
    /// answer it. Nothing about the request needs fixing — a different
    /// spec, or a larger budget, may still succeed.
    Construction {
        /// Which construction was running, in the vocabulary of the `--vtree`
        /// specs (e.g. `minfill-primal`, `portfolio`, `flowcutter-incidence`) —
        /// the base the caller wrote, where they wrote one.
        spec: String,
        /// What the construction reported.
        reason: String,
    },

    /// A file could not be read or written.
    Io {
        /// The file in question.
        path: PathBuf,
        /// What was being attempted, as a verb — `open`, `read`, `write`,
        /// `create`.
        action: &'static str,
        /// The operating system's account of the failure.
        reason: String,
    },
}

impl VitriError {
    /// For rejecting a `--vtree` spec string: an unknown family, or a token the
    /// named family cannot honor.
    pub fn spec(spec: impl Into<String>, reason: impl Into<String>) -> Self {
        VitriError::Spec {
            spec: spec.into(),
            reason: reason.into(),
        }
    }

    /// For refusing a `VITRI_*` variable's value instead of guessing at it;
    /// `reason` should list the values that would be accepted.
    pub fn env(var: &'static str, reason: impl Into<String>) -> Self {
        VitriError::Env {
            var,
            reason: reason.into(),
        }
    }

    /// For a [`RunConfig`](crate::config::RunConfig) that asks for something this
    /// crate cannot deliver, where the caller's fix is to change the configuration.
    pub fn config(reason: impl Into<String>) -> Self {
        VitriError::Config {
            reason: reason.into(),
        }
    }

    /// For data handed in that cannot be used at all, as opposed to a request that
    /// is merely unanswerable.
    pub fn input(reason: impl Into<String>) -> Self {
        VitriError::Input {
            reason: reason.into(),
        }
    }

    /// For two arguments that must describe the same formula and do not, where
    /// the caller's fix is to pass the ones that belong together.
    pub fn mismatch(reason: impl Into<String>) -> Self {
        VitriError::Mismatch {
            reason: reason.into(),
        }
    }

    /// For a construction that gave up on a well formed request; `spec` is the
    /// name the caller would recognise from `--vtree`, which the construction
    /// itself does not know.
    pub fn construction(spec: impl Into<String>, reason: impl Into<String>) -> Self {
        VitriError::Construction {
            spec: spec.into(),
            reason: reason.into(),
        }
    }

    /// For a failed file operation; `source` is flattened to its message here, so
    /// the [`std::io::Error`] is not carried on and `action` is all that says what
    /// was being attempted.
    pub fn io(path: impl Into<PathBuf>, action: &'static str, source: &std::io::Error) -> Self {
        VitriError::Io {
            path: path.into(),
            action,
            reason: source.to_string(),
        }
    }
}

impl fmt::Display for VitriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VitriError::Config { reason } => write!(f, "{reason}"),
            VitriError::Spec { spec, reason } => write!(f, "vtree spec '{spec}': {reason}"),
            VitriError::Env { var, reason } => write!(f, "environment variable {var}: {reason}"),
            VitriError::Input { reason } => write!(f, "{reason}"),
            VitriError::Mismatch { reason } => write!(f, "{reason}"),
            VitriError::Construction { spec, reason } => write!(f, "{spec} failed: {reason}"),
            VitriError::Io {
                path,
                action,
                reason,
            } => write!(f, "cannot {action} {}: {reason}", path.display()),
        }
    }
}

impl std::error::Error for VitriError {}

/// Name the construction that produced a failure.
///
/// The constructions report a plain sentence and do not know which `--vtree`
/// spec asked for them; this is where that name gets attached.
pub(crate) fn from_construction<T>(result: Result<T, String>, spec: &str) -> Result<T, VitriError> {
    result.map_err(|reason| VitriError::construction(spec, reason))
}
