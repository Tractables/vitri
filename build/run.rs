//! Running a program: the two spellings every step here uses, and the probe
//! that asks whether a tool is installed at all.

use std::path::Path;
use std::process::Command;

pub(crate) fn run(mut cmd: Command, what: &str) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn {what}: {e} (command: {cmd:?})"));
    assert!(status.success(), "{what} failed (command: {cmd:?})");
}

/// Same, for a command driven by a script on stdin (`ar -M`).
pub(crate) fn run_with_stdin(mut cmd: Command, input: &str, what: &str) {
    use std::io::Write;

    let mut child = cmd
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {what}: {e} (command: {cmd:?})"));
    child
        .stdin
        .as_mut()
        .unwrap_or_else(|| panic!("{what}: stdin was not piped"))
        .write_all(input.as_bytes())
        .unwrap_or_else(|e| panic!("failed to write {what} script: {e}"));
    let status = child
        .wait()
        .unwrap_or_else(|e| panic!("failed to wait for {what}: {e}"));
    assert!(status.success(), "{what} failed (command: {cmd:?})");
}

pub(crate) fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The upstream Arjun commit these sources were vendored at, read from the
/// `ARJUN_PIN_SHA1` text file beside them and passed to CMake as
/// `-DGIT_SHA1=`. A vendored tree has no `.git`, so upstream's own probe
/// would leave the built library reporting an empty version; this makes it
/// report the commit recorded in `PROVENANCE.md`. The file lives inside the
/// package because `include` cannot reach outside the crate root.
pub(crate) fn arjun_pin(vendor: &Path) -> String {
    let p = vendor.join("ARJUN_PIN_SHA1");
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
        .trim()
        .to_string()
}
