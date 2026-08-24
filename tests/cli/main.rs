//! The binary driven the way a shell drives it, one module per subject.
//!
//! The harness and the fixtures are here because every module uses them;
//! each module states what it is about at its own top.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use serde_json::Value;
use vitri::bundle::components::{
    CANDIDATES_DIR, COMPONENTS_DIR, COMPONENTS_FORMAT_TAG, COMPONENTS_JSON_NAME,
};
use vitri::bundle::{PREPROCESS_RECORD_NAME, REDUCED_CNF_NAME, VTREE_NAME};
use vitri::cnf::{CnfFormula, Mode, Reduced};
use vitri::spec::{
    DEFAULT_VTREE_SPEC, baseline_spec_names, decomposition_spec_names, elimination_spec_names,
    spec_param_docs, standalone_spec_names, vtree_spec_bases,
};

#[path = "../common/mod.rs"]
mod common;
use common::{
    CLAUSE_ID_ABOVE_COUNT, FULLY_RESOLVED, IRREDUCIBLE_5, SHOW_ID_ABOVE_COUNT, Scratch,
    tokenize_vtree_text, wide_component_dimacs,
};

mod edge_shapes;
mod env;
mod failed_runs;
mod grammar;
mod modes;
mod output_bytes;
mod success_path;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A finished run of the binary.
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    /// Assert the exit status and hand the run back, so a caller reads
    /// `run(...).exit(2).stderr` as one sentence. The message quotes both
    /// streams: a wrong code is nearly always explained by what was printed.
    fn exit(self, want: i32) -> Self {
        assert_eq!(
            self.code, want,
            "expected exit {want}, got {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            self.code, self.stdout, self.stderr,
        );
        self
    }

    fn assert_stderr(&self, needle: &str) {
        assert!(
            self.stderr.contains(needle),
            "stderr must contain {needle:?}, got:\n{}",
            self.stderr,
        );
    }

    fn assert_stdout(&self, needle: &str) {
        assert!(
            self.stdout.contains(needle),
            "stdout must contain {needle:?}, got:\n{}",
            self.stdout,
        );
    }
}

/// Run the binary with `args`.
///
/// Every `VITRI_*` variable is stripped from the child's environment first:
/// they are real knobs this binary reads at startup, so a value exported in
/// whatever shell launched `cargo test` would otherwise silently change what
/// these tests measure. The env-boundary tests below put back exactly the one
/// variable they are about.
fn run(args: &[&str]) -> Run {
    run_with_env(args, &[])
}

fn run_with_env(args: &[&str], env: &[(&str, &str)]) -> Run {
    let raw: Vec<(&str, &OsStr)> = env.iter().map(|&(k, v)| (k, OsStr::new(v))).collect();
    run_with_raw_env(args, &raw)
}

/// [`run_with_env`] for a value that is not text.
///
/// The environment holds bytes, not strings, so a knob set to bytes that are
/// not UTF-8 is a value the reader has to answer for; this is the only spelling
/// that can hand one over.
fn run_with_raw_env(args: &[&str], env: &[(&str, &OsStr)]) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vitri"));
    cmd.args(args);
    for (name, _) in std::env::vars() {
        if name.starts_with("VITRI_") {
            cmd.env_remove(name);
        }
    }
    for (name, value) in env {
        cmd.env(name, value);
    }
    let out = cmd.output().expect("the vitri binary must be spawnable");
    Run {
        code: out.status.code().unwrap_or_else(|| {
            panic!(
                "the run was killed by a signal rather than exiting: {:?}",
                out.status
            )
        }),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn s(p: &Path) -> &str {
    p.to_str().expect("scratch paths are UTF-8")
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("reading {}: {e}", p.display()))
}

fn json(p: &Path) -> Value {
    serde_json::from_str(&read(p)).unwrap_or_else(|e| panic!("parsing {}: {e}", p.display()))
}

/// The keys of a JSON object, sorted — for comparing a written document
/// against the field list `docs/bundle.md` publishes.
fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object()
        .expect("a JSON object")
        .keys()
        .cloned()
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

/// The names of the entries directly inside `dir`.
fn entries(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("listing {}: {e}", dir.display()))
        .map(|e| {
            let name = e.expect("dir entry").file_name();
            name.to_string_lossy().into_owned()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Fixtures. Every one is tiny, written from the test, and chosen for a
// structural property rather than for size.
// ---------------------------------------------------------------------------

/// The same formula as [`IRREDUCIBLE_5`] declared as track 4: a show set and a
/// weight table, both of which need reducing to lowest terms and renumbering.
const PROJECTED_WEIGHTED: &str = "c t pwmc\n\
     p cnf 5 5\n\
     c p show 2 4 5 0\n\
     c p weight 1 2/4 0\n\
     c p weight -1 1/2 0\n\
     c p weight 2 -1/-3 0\n\
     c p weight -2 2/3 0\n\
     c p weight 3 6/-8 0\n\
     c p weight -3 1/4 0\n\
     1 2 0\n\
     -1 3 0\n\
     -2 -3 4 0\n\
     2 3 -4 0\n\
     4 5 0\n";

/// Two independent chains: the split has something to split.
const TWO_COMPONENTS: &str = "p cnf 6 4\n1 2 0\n-1 3 0\n4 5 0\n-4 6 0\n";

/// Refuted by unit propagation, declared on the weighted track and carrying a
/// weight table over the variable the refutation turns on. What distinguishes
/// it from the plain [`common::REFUTED`] is that there are weights to reduce
/// and to lift: a refutation has to be reported the same way whether or not
/// preprocessing collected any.
const REFUTED_WEIGHTED: &str = "c t wmc\n\
     p cnf 2 2\n\
     c p weight 1 3/4 0\n\
     c p weight -1 1/4 0\n\
     1 0\n\
     -1 0\n";

/// `count` independent 3-variable sub-problems, none of them reducible to
/// nothing — used with both preprocessing stages off, so the split is exactly the
/// one written here.
fn many_components(count: u32) -> String {
    let mut clauses = Vec::new();
    for i in 0..count {
        let (a, b, c) = (3 * i + 1, 3 * i + 2, 3 * i + 3);
        clauses.push(format!("{a} {b} {c} 0\n"));
        clauses.push(format!("-{a} -{b} 0\n"));
        clauses.push(format!("-{b} -{c} 0\n"));
        clauses.push(format!("-{a} -{c} 0\n"));
        clauses.push(format!("{a} {b} 0\n"));
    }
    let mut text = format!("p cnf {} {}\n", 3 * count, clauses.len());
    for c in &clauses {
        text.push_str(c);
    }
    text
}
