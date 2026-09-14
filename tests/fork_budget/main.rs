//! Real fork tests run without libtest's worker thread.

use vitri::cnf;

// Compile the same private transport and codec used by the library.
#[allow(dead_code)]
#[path = "../../src/preprocess/fork_budget.rs"]
mod fork_budget;
#[allow(dead_code)]
#[path = "../../src/preprocess/fork_payload.rs"]
mod fork_payload;

use fork_payload::{Dec, ForkPayload, get_vec, put_len, put_u32};

#[derive(Debug, PartialEq)]
struct Payload {
    pid: u32,
    bytes: Vec<u8>,
}

impl Payload {
    fn new(size: usize) -> Self {
        Self {
            pid: std::process::id(),
            bytes: (0..size).map(|i| i as u8).collect(),
        }
    }
}

impl ForkPayload for Payload {
    fn encode(&self, out: &mut Vec<u8>) {
        put_u32(out, self.pid);
        put_len(out, self.bytes.len());
        out.extend_from_slice(&self.bytes);
    }

    fn decode(d: &mut Dec<'_>) -> Option<Self> {
        Some(Self {
            pid: d.get_u32()?,
            bytes: get_vec(d, |d| d.get_u8())?,
        })
    }
}

#[cfg(unix)]
mod cases;

#[cfg(unix)]
fn main() {
    cases::run();
}

#[cfg(not(unix))]
fn main() {
    use fork_budget::{ForkOutcome, run_forked_with_deadline};
    use std::time::Instant;

    assert_eq!(
        run_forked_with_deadline(Instant::now(), || Some(Payload::new(8))),
        ForkOutcome::Completed(Some(Payload::new(8)))
    );
}
