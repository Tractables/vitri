use std::time::{Duration, Instant};

use super::Payload;
use super::fork_budget::{ForkOutcome, fork_with_kill_deadline, run_forked_with_deadline};

pub(super) fn run() {
    for (name, case) in [
        (
            "the_guarded_entry_returns_the_childs_payload",
            returned_payload as fn(),
        ),
        ("none_is_completed_none", completed_none),
        ("a_large_payload_crosses_the_pipe", large_payload),
        ("a_panicking_child_is_failed", panicking_child),
        (
            "an_expired_deadline_kills_and_reaps_the_child",
            killed_and_reaped,
        ),
        (
            "an_ignored_sigchld_runs_the_closure_inline",
            ignored_sigchld_runs_inline,
        ),
        (
            "a_child_reaped_by_someone_else_still_delivers_its_payload",
            reaped_elsewhere_still_delivers,
        ),
    ] {
        #[cfg(target_os = "linux")]
        assert_eq!(
            super::fork_budget::threads_in_this_process(),
            Some(1),
            "fork tests require one process thread"
        );
        case();
        println!("test {name} ... ok");
    }
}

fn check_payload(size: usize) {
    let parent = std::process::id();
    let out = run_forked_with_deadline(Instant::now() + Duration::from_secs(30), || {
        Some(Payload::new(size))
    });
    let ForkOutcome::Completed(Some(payload)) = out else {
        panic!("expected a completed payload, got {out:?}");
    };
    assert_ne!(payload.pid, parent, "the guarded entry ran inline");
    assert_eq!(payload.bytes, Payload::new(size).bytes);
}

fn returned_payload() {
    check_payload(8);
}

fn large_payload() {
    check_payload(1 << 20);
}

fn completed_none() {
    let out =
        run_forked_with_deadline(Instant::now() + Duration::from_secs(30), || None::<Payload>);
    assert_eq!(out, ForkOutcome::Completed(None));
}

fn panicking_child() {
    let out = run_forked_with_deadline(
        Instant::now() + Duration::from_secs(30),
        || -> Option<Payload> { panic!("intentional panic inside the forked child") },
    );
    let ForkOutcome::Failed(why) = out else {
        panic!("expected a failed child, got {out:?}");
    };
    assert!(why.contains("panicked"), "unexpected reason: {why}");
}

/// Runs `body` with `SIGCHLD` ignored, so the kernel reaps every child as it
/// exits, and restores the default disposition afterwards.
fn with_sigchld_ignored(body: impl FnOnce()) {
    // SAFETY: `signal` with a constant disposition; this process has one thread.
    unsafe { libc::signal(libc::SIGCHLD, libc::SIG_IGN) };
    body();
    // SAFETY: as above.
    unsafe { libc::signal(libc::SIGCHLD, libc::SIG_DFL) };
}

fn ignored_sigchld_runs_inline() {
    with_sigchld_ignored(|| {
        assert!(
            !super::fork_budget::forking_is_sound(),
            "a process whose children are reaped on exit must not fork"
        );
        let out = run_forked_with_deadline(Instant::now() + Duration::from_secs(30), || {
            Some(Payload::new(8))
        });
        let ForkOutcome::Completed(Some(payload)) = out else {
            panic!("expected a completed payload, got {out:?}");
        };
        assert_eq!(payload.pid, std::process::id(), "the closure was forked");
    });
    assert!(super::fork_budget::forking_is_sound());
}

fn reaped_elsewhere_still_delivers() {
    // The guarded entry would run inline here, so call the transport directly:
    // this is the path a host reaping every child in its own handler takes.
    with_sigchld_ignored(|| {
        let out = fork_with_kill_deadline(Instant::now() + Duration::from_secs(30), || {
            Some(Payload::new(1 << 16))
        });
        let ForkOutcome::Completed(Some(payload)) = out else {
            panic!("expected a completed payload, got {out:?}");
        };
        assert_ne!(payload.pid, std::process::id(), "the closure ran inline");
        assert_eq!(payload.bytes, Payload::new(1 << 16).bytes);
    });
}

fn killed_and_reaped() {
    let out = fork_with_kill_deadline(Instant::now(), || -> Option<Payload> {
        loop {
            // SAFETY: pause takes no arguments; the parent terminates this child.
            unsafe { libc::pause() };
        }
    });
    let ForkOutcome::Killed { pid } = out else {
        panic!("expected a killed child, got {out:?}");
    };
    let mut status = 0;
    // SAFETY: status is writable and WNOHANG cannot block. The PID is the child
    // just reaped by the transport, so this must report ECHILD.
    let ret = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
    assert_eq!(ret, -1, "child {pid} was not reaped");
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
}
