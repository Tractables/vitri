use crate::cnf::Clause;
use crate::cnf::CnfFormula;
use crate::cnf::Weights;
use crate::preprocess::arjun::ArjunResult;
use crate::preprocess::arjun::ArjunWeightedResult;
use crate::preprocess::fork_budget::*;
use crate::preprocess::fork_payload::*;
use crate::preprocess::var_map::VarMap;
use crate::tests::common::lit;
use num_rational::BigRational;
use num_traits::One;
use std::time::Duration;
use std::time::Instant;

/// A representative `ArjunResult`: every field non-empty, so a codec that
/// silently drops one is caught.
fn sample_result() -> ArjunResult {
    ArjunResult {
        formula: CnfFormula {
            // `num_vars` deliberately larger than the max var used, so a
            // decoder that recomputes it from the clauses is caught.
            num_vars: 7,
            clauses: vec![
                Clause::new(vec![lit(0, true), lit(1, false), lit(4, true)]),
                Clause::new(vec![lit(2, false)]),
                Clause::new(vec![]),
            ],
        },
        multiplier_exp: 13,
        backbone: vec![lit(0, true), lit(3, false)],
        equiv: vec![(lit(1, true), lit(2, false))],
        learnt_clauses: vec![vec![1, -2, 3], vec![-4]],
        // Both entry shapes present: a mapped var, a NEGATED mapped var, and
        // an absent one — a codec that collapses `None` and a real literal,
        // or that drops the sign, is caught here.
        input_to_reduced_lit: VarMap::from_entries(vec![Some(2), None, Some(-5), Some(1)]),
    }
}

fn roundtrip<T: ForkPayload + PartialEq + std::fmt::Debug>(v: &T) -> T {
    let mut buf = Vec::new();
    v.encode(&mut buf);
    let mut d = Dec::new(&buf);
    let back = T::decode(&mut d).expect("decode");
    assert!(
        d.rest.is_empty(),
        "decoder left {} trailing bytes",
        d.rest.len()
    );
    back
}

#[test]
fn an_arjun_result_decodes_to_exactly_what_was_encoded() {
    let v = sample_result();
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn a_weighted_arjun_result_decodes_to_exactly_what_was_encoded() {
    let w = |s: &str| crate::cnf::parse_weight(s).unwrap();
    let v = ArjunWeightedResult {
        formula: CnfFormula {
            num_vars: 4,
            clauses: vec![Clause::new(vec![lit(0, true), lit(2, false)])],
        },
        weights: Weights::from_dimacs_pairs(
            &[
                (1, w("3/7")),
                (-1, w("4/7")),
                (3, w("-5/2")),
                (-3, BigRational::one()),
            ],
            4,
        ),
        multiplier: w("1024/3"),
        // Mapped, negated-mapped and absent entries, so a codec that
        // collapses `None` or drops the sign is caught.
        input_to_reduced_lit: VarMap::from_entries(vec![Some(1), None, Some(-3), Some(4)]),
    };
    assert_eq!(roundtrip(&v), v);
}

/// A truncated stream must decode to `None`, never panic or over-reserve.
#[test]
fn truncated_stream_decodes_to_none() {
    let mut buf = Vec::new();
    sample_result().encode(&mut buf);
    for cut in [0, 1, 4, 9, buf.len() / 2, buf.len() - 1] {
        let mut d = Dec::new(&buf[..cut]);
        assert!(
            ArjunResult::decode(&mut d).is_none(),
            "truncation at {cut} decoded as a value"
        );
    }
    // A bogus length prefix (u64::MAX) must be rejected, not reserved.
    let mut bogus = vec![0xffu8; 8];
    bogus.extend_from_slice(&[0u8; 8]);
    let mut d = Dec::new(&bogus);
    assert!(get_vec(&mut d, |d| d.get_u32()).is_none());
}

/// Fork parity: a closure returning data through the harness must produce
/// exactly what calling it directly produces.
///
/// Through the internal entry, like every fork test below it: a test binary has
/// more than one thread, so the public entry answers this inline and the fork
/// would go untested. What these tests hand a child is one allocation, which is
/// what the allocator's own `pthread_atfork` handlers already make safe — not
/// the native stage the soundness check exists for.
#[test]
fn forked_result_matches_direct_call() {
    let direct = sample_result();
    let out = fork_with_kill_deadline(Instant::now() + Duration::from_secs(30), || {
        Some(sample_result())
    });
    match out {
        ForkOutcome::Completed(Some(v)) => assert_eq!(v, direct),
        other => panic!("expected Completed(Some(..)), got {other:?}"),
    }
}

/// `None` from the closure is a legitimate result, not a failure, so callers
/// can tell "gave up cleanly" from "killed".
#[test]
fn forked_none_is_completed_none() {
    let out = fork_with_kill_deadline(Instant::now() + Duration::from_secs(30), || {
        None::<ArjunResult>
    });
    assert_eq!(out, ForkOutcome::Completed(None));
}

/// A closure that ignores the deadline is killed at it, and the child is
/// reaped (no zombie). Uses the internal kill-deadline entry so the test
/// does not wait out `KILL_GRACE`.
#[test]
fn forked_overrun_is_killed_and_reaped() {
    let budget = Duration::from_millis(200);
    let started = Instant::now();
    let out = fork_with_kill_deadline(started + budget, || {
        // Stands in for an uninterruptible native stage: no deadline checks.
        std::thread::sleep(Duration::from_secs(30));
        Some(sample_result())
    });
    let elapsed = started.elapsed();
    let pid = match out {
        ForkOutcome::Killed { pid } => pid,
        other => panic!("expected Killed, got {other:?}"),
    };
    assert!(elapsed >= budget, "killed early: {elapsed:?}");
    assert!(
        elapsed < Duration::from_secs(10),
        "kill was not prompt: {elapsed:?}"
    );
    // Reaped: waiting on that exact pid must report "no such child".
    let mut status = 0;
    // SAFETY: `status` is a live `c_int` the call fills in, and `WNOHANG` means
    // it returns rather than blocks. Asking about an already-reaped pid is what
    // the assertion below is for: the call reports `ECHILD`, it does not reap a
    // second time.
    let r = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
    assert_eq!(r, -1, "child {pid} was not reaped (waitpid returned {r})");
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
}

/// A payload far larger than the pipe buffer must still arrive: the parent
/// drains while it waits, so the child never blocks in `write`.
#[test]
fn forked_large_payload_survives_pipe_buffer() {
    // ~1 MiB of clauses, comfortably past the 64 KiB default pipe capacity.
    let big = || {
        let clauses: Vec<Clause> = (0..40_000u32)
            .map(|i| Clause::new(vec![lit(i % 1000, true), lit((i + 7) % 1000, false)]))
            .collect();
        ArjunResult {
            formula: CnfFormula {
                num_vars: 1000,
                clauses,
            },
            multiplier_exp: 1,
            backbone: Vec::new(),
            equiv: Vec::new(),
            learnt_clauses: Vec::new(),
            input_to_reduced_lit: VarMap::from_entries(Vec::new()),
        }
    };
    let out = fork_with_kill_deadline(Instant::now() + Duration::from_secs(60), || Some(big()));
    match out {
        ForkOutcome::Completed(Some(v)) => assert_eq!(v, big()),
        other => panic!("expected Completed(Some(..)), got {other:?}"),
    }
}

/// A panicking closure must become `Failed`, not a second copy of the
/// process escaping through `fork()`'s frame.
#[test]
fn forked_panic_is_failed_not_escape() {
    let out = fork_with_kill_deadline(
        Instant::now() + Duration::from_secs(30),
        || -> Option<ArjunResult> { panic!("intentional panic inside the forked child") },
    );
    match out {
        ForkOutcome::Failed(why) => {
            assert!(why.contains("panicked"), "unexpected reason: {why}")
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

/// A process with a second thread runs the closure itself instead of forking.
/// This is the module's own safety premise, enforced rather than assumed.
///
/// Asserted by side effect, because that is the only thing that tells the two
/// routes apart: a fork runs the closure in a copy of this process, where every
/// write it makes is invisible here. A run that still sees the write is a run
/// that stayed in one process.
///
/// The second thread is created here rather than taken from the harness, so the
/// premise under test is a fact of the test and not of how the suite was
/// invoked.
#[test]
fn a_process_with_another_thread_runs_the_closure_inline() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;

    let stop = Arc::new(AtomicBool::new(false));
    let second = std::thread::spawn({
        let stop = Arc::clone(&stop);
        move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::yield_now()
            }
        }
    });
    // Waits until the count this reads is the one the assertion is about, rather
    // than assuming a test binary has threads.
    while threads_in_this_process().unwrap_or(1) < 2 {
        std::thread::yield_now();
    }
    assert!(
        !forking_is_sound(),
        "a second thread is running and forking is still called sound"
    );

    let ran_here = Arc::new(AtomicBool::new(false));
    let out = run_forked_with_deadline(Instant::now() + Duration::from_secs(30), {
        let ran_here = Arc::clone(&ran_here);
        move || {
            ran_here.store(true, Ordering::SeqCst);
            Some(sample_result())
        }
    });
    stop.store(true, Ordering::Relaxed);
    second.join().expect("the second thread panicked");

    assert_eq!(out, ForkOutcome::Completed(Some(sample_result())));
    assert!(
        ran_here.load(Ordering::SeqCst),
        "the closure ran in a forked child — its write is invisible here, and so is any lock it \
         inherited from the thread above"
    );
}
