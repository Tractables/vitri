//! Hard wall-clock enforcement for **uninterruptible** native work, via `fork()`.
//!
//! Some native preprocessing is a single C/C++ call with no interrupt or time
//! hook — the in-process Arjun stages (`arjun_lib::reduce_anytime`) are the
//! motivating case. Once such a stage starts, no Rust-side deadline check can
//! stop it: the orchestrator can only test its budget *between* stages, so a
//! stage that runs long simply burns the rest of the budget and the caller's
//! printed budget is a polite fiction.
//!
//! The fix is a process boundary. This library is strictly single-threaded by
//! design, which makes `fork()` both safe and cheap here:
//!
//! * **Safe** — the classic fork hazard is a lock held by *another* thread at
//!   fork time, which no one in the child can ever release. With one thread
//!   there is no other lock holder, and glibc malloc additionally registers
//!   `pthread_atfork` handlers that quiesce its own mutexes across the call.
//!   Those handlers cover the allocator and nothing else, so the one-thread
//!   premise is what carries the argument. [`forking_is_sound`] checks that
//!   premise on every call instead of assuming it, and a process that has other
//!   threads running gets the inline path.
//! * **Cheap** — the child is copy-on-write, so no formula is copied up front;
//!   only pages the native work actually writes are duplicated. While the child
//!   works the parent sleeps in `poll()`, so the one-CPU-per-CNF discipline is
//!   preserved (no threads are created anywhere in this module).
//!
//! The child runs the closure, serializes its result into a pipe and `_exit`s
//! (never running the parent image's `atexit` handlers or destructors). The
//! parent drains the pipe while polling for the deadline and `SIGKILL`s + reaps
//! the child if it is still running when the deadline (plus [`KILL_GRACE`])
//! passes. `SIGKILL` is uncatchable, so a budget enforced this way is real no
//! matter what the native code is doing.
//!
//! On non-unix targets the harness simply calls the closure inline: the budget
//! degrades to the caller's own between-stage checks, exactly as before.
//!
//! # Safety
//!
//! Every `unsafe` block here is one libc call, and three shared facts cover
//! almost all of them; the per-site comments add only what is specific to a
//! call.
//!
//! * **`fork` itself** rests on the single-threaded caller the crate documents,
//!   for the reason spelled out above: with no second thread there is no lock
//!   another thread could hold across the call, so the child is free to run
//!   ordinary Rust — allocating, serializing, writing — instead of being
//!   confined to async-signal-safe calls. What the child must NOT do is return
//!   through `fork`'s frame, which is why it ends in `_exit` and why an unwind
//!   out of the closure is caught before it can get that far.
//! * **Descriptors** are the two ends of the pipe this function created. Each
//!   process closes the end it does not use, and each remaining end is closed
//!   exactly once on every path out — the parent's read end included on the
//!   error paths, where the close is ordered against the kill and the reap.
//! * **`kill` and `waitpid`** always name the child this call forked, which has
//!   not been reaped when they run, so its pid is still reserved for it and
//!   cannot have been recycled onto an unrelated process.
//! * **Buffers** handed to `poll`, `read` and `write` are live locals passed
//!   with their own length, and only the byte count the call returns is
//!   treated as written.
//!
//! A test binary running its suite in parallel is a multi-threaded process, so
//! the public entry answers it inline. This module's own fork tests therefore
//! call [`fork_with_kill_deadline`], the internal entry, which is what keeps the
//! fork itself covered.

use std::time::{Duration, Instant};

use super::fork_payload::{Dec, ForkPayload};

/// Extra budget the parent grants the child **beyond** the closure's logical
/// deadline before it pulls the trigger.
///
/// The closure's own between-stage deadline checks fire first: at the deadline
/// it stops starting new work and falls through to its cheap bookkeeping
/// (building the reduced formula, serializing it into the pipe). That tail is
/// memcpy-shaped, not search-shaped, so a small fixed grace keeps the kill from
/// racing a result that is already earned. Anything still *computing* at
/// `deadline + KILL_GRACE` was going to blow the budget.
pub(super) const KILL_GRACE: Duration = Duration::from_secs(2);

/// How long the parent blocks in one `poll()` before re-checking the clock. The
/// parent is idle either way; this only bounds how stale its deadline view can
/// get while the pipe is quiet.
const POLL_SLICE_MS: std::os::raw::c_int = 50;

/// Child exit code: the closure (or its serialization) panicked and unwound.
/// Distinct from a clean `None`, which is delivered *through* the pipe.
const CHILD_EXIT_PANIC: std::os::raw::c_int = 91;
/// Child exit code: the result could not be written to the pipe.
const CHILD_EXIT_WRITE_FAILED: std::os::raw::c_int = 92;

/// What happened to a [`run_forked_with_deadline`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ForkOutcome<T> {
    /// The closure ran to completion and its result was delivered. Also the
    /// outcome when the harness had to fall back to an inline call (fork/pipe
    /// unavailable, or a non-unix target) — in that case the deadline was NOT
    /// hard-enforced, matching the pre-fork behavior.
    Completed(T),
    /// The deadline (plus [`KILL_GRACE`]) passed with the child still running:
    /// it was `SIGKILL`ed and reaped. No result.
    Killed {
        /// PID of the reaped child, kept for diagnostics and for tests that
        /// assert the reap actually happened.
        pid: i32,
    },
    /// The child died without delivering a decodable result (panic, signal,
    /// short or corrupt pipe). Carries a short reason for the caller's log
    /// line.
    Failed(String),
}

/// Run `f` in a forked child and hard-enforce `deadline` on it.
///
/// The child is `SIGKILL`ed at `deadline + `[`KILL_GRACE`] if it has not
/// finished; see the module docs for why that is safe here. `f` must be
/// self-contained: it runs in a *copy* of this process, so anything it mutates
/// other than its return value (globals, caches, files) is invisible to the
/// parent. Everything the caller needs must travel through `T`.
///
/// The module's safety argument has a precondition — one thread — and this is
/// where it is checked rather than assumed ([`forking_is_sound`]). A process
/// that does not meet it runs `f` inline, which is the same fallback this
/// function already takes when there is no descriptor to spare or when `fork`
/// itself fails.
#[cfg(unix)]
pub(super) fn run_forked_with_deadline<T: ForkPayload>(
    deadline: Instant,
    f: impl FnOnce() -> Option<T>,
) -> ForkOutcome<Option<T>> {
    if !forking_is_sound() {
        return ForkOutcome::Completed(f());
    }
    fork_with_kill_deadline(deadline + KILL_GRACE, f)
}

/// Whether this process may fork and then keep running the parent's code in the
/// child — that is, whether the module docs' safety argument holds here.
///
/// It holds for one thread and no more. `fork` duplicates the calling thread and
/// nothing else, so a lock another thread held at that instant stays locked in
/// the child, owned by a thread that does not exist there to release it. A child
/// that reaches for such a lock never returns, and a native reduction reaches for
/// plenty: one-time initialisers, the allocator's bookkeeping, whatever the
/// solver keeps behind a static. POSIX states the same rule from the front —
/// after `fork` in a multi-threaded process the child may only call
/// async-signal-safe functions until it execs, and a preprocessing stage is not
/// one of those.
///
/// The single-threaded caller this crate documents is therefore unaffected: the
/// budget is still enforced by the fork exactly where it was. What has more than
/// one thread is a test binary running its suite in parallel, and a fork from
/// there could wedge the child on an inherited lock, cost the caller its whole
/// budget plus [`KILL_GRACE`], and surface as a stage that gave up.
///
/// An unreadable thread count means fork, which is the behaviour on any platform
/// without `/proc`.
#[cfg(unix)]
pub(super) fn forking_is_sound() -> bool {
    threads_in_this_process().unwrap_or(1) == 1
}

/// This process's thread count, read from `/proc/self/status`. `None` where that
/// is not readable — no `/proc`, or a kernel that does not publish the field.
#[cfg(unix)]
pub(super) fn threads_in_this_process() -> Option<usize> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("Threads:"))?
        .trim()
        .parse()
        .ok()
}

/// Non-unix stub: no `fork()`, so the closure runs inline and the deadline is
/// only as hard as the closure's own internal checks (i.e. the pre-fork
/// behavior). Documented rather than `compile_error!`ed so the crate still
/// builds everywhere; the production target is unix.
#[cfg(not(unix))]
pub(super) fn run_forked_with_deadline<T: ForkPayload>(
    _deadline: Instant,
    f: impl FnOnce() -> Option<T>,
) -> ForkOutcome<Option<T>> {
    ForkOutcome::Completed(f())
}

/// The one implementation. Takes the absolute **kill** deadline (grace already
/// folded in by [`run_forked_with_deadline`]) so tests can exercise the kill
/// path without waiting out [`KILL_GRACE`].
#[cfg(unix)]
pub(super) fn fork_with_kill_deadline<T: ForkPayload>(
    kill_deadline: Instant,
    f: impl FnOnce() -> Option<T>,
) -> ForkOutcome<Option<T>> {
    // Flush before forking: whatever is still buffered here would be duplicated
    // into the child's copy of those buffers and printed twice.
    flush_std_buffers();

    let mut fds: [std::os::raw::c_int; 2] = [0; 2];
    // SAFETY: buffers (§ Safety) — `fds` is the live two-element array `pipe`
    // fills in; on failure it is left unused.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        // Out of descriptors: fall back to an inline call rather than failing
        // preprocessing outright.
        return ForkOutcome::Completed(f());
    }
    let (rd, wr) = (fds[0], fds[1]);

    // SAFETY: the fork itself (§ Safety). Both returned branches are handled,
    // and the child branch never falls through to parent code.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        // SAFETY: descriptors (§ Safety). The fork failed, so no other process
        // holds a copy of either end.
        unsafe {
            libc::close(rd);
            libc::close(wr);
        }
        return ForkOutcome::Completed(f());
    }

    if pid == 0 {
        // ---- child ----
        // SAFETY: descriptors (§ Safety) — the child's copy of the read end,
        // unused here. Closing it is what lets the parent see EOF once the
        // child is gone.
        unsafe { libc::close(rd) };
        tie_lifetime_to_parent();
        let code = child_body(wr, f);
        // `_exit`, never `exit`: the parent image's atexit handlers and static
        // destructors must not run in this copy.
        // SAFETY: `_exit` has no precondition; it ends this process image and
        // diverges, so nothing below can observe a half-torn-down copy.
        unsafe { libc::_exit(code) };
        // Unreachable — `_exit` is declared `-> !`. Kept as a hard stop because
        // the one catastrophic failure mode of a fork harness is control
        // resuming here, i.e. two processes running the parent's code.
        //
        // SAFETY: unreachable, and `abort` has no precondition to meet.
        #[allow(unreachable_code)]
        unsafe {
            libc::abort()
        };
    }

    // ---- parent ----
    // SAFETY: descriptors (§ Safety) — the parent's copy of the write end. It
    // has to go: a second writer left open here would keep the pipe alive and
    // the read below would never see EOF.
    unsafe { libc::close(wr) };
    parent_wait(pid, rd, kill_deadline)
}

/// Make the kernel `SIGKILL` this child if the parent goes away, so a child can
/// never outlive the run that spawned it.
///
/// Without this, an outer timeout that kills the calling process leaves an
/// orphaned child grinding through the rest of its native stage — a stray CPU
/// burner in a process nobody is watching any more. Linux-only (`prctl`); elsewhere the parent
/// SIGKILL still bounds the normal path, only the orphan case is uncovered.
#[cfg(all(unix, target_os = "linux"))]
fn tie_lifetime_to_parent() {
    // SAFETY: the two-argument form of `prctl`; `PR_SET_PDEATHSIG` takes a
    // signal number by value, passes no pointer, and affects only this
    // process.
    unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) };
    // `prctl` races a parent that died between `fork` and here: the death signal
    // would then never be delivered. Re-parenting to init is the tell.
    // SAFETY: no arguments, no preconditions.
    if unsafe { libc::getppid() } == 1 {
        // SAFETY: `_exit` has no precondition and diverges; the child leaves
        // without touching the parent's shutdown path.
        unsafe { libc::_exit(0) };
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn tie_lifetime_to_parent() {}

/// Child half: run the closure, encode `Option<T>`, write it, report via exit
/// code. Returns the exit code; never returns to the caller's control flow.
///
/// The `catch_unwind` is required, not defensive: the binary installs an
/// alloc-error hook that PANICS (so the OOM-recovery cascade can catch it), and
/// this crate must build under `panic = "unwind"`. An escaping unwind would
/// return through `fork()`'s call frame and leave a second process running the
/// parent's code — the one truly catastrophic failure mode of a fork harness.
#[cfg(unix)]
fn child_body<T: ForkPayload>(
    wr: std::os::raw::c_int,
    f: impl FnOnce() -> Option<T>,
) -> std::os::raw::c_int {
    let encoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let out = f();
        let mut buf = Vec::new();
        match out {
            Some(v) => {
                buf.push(1u8);
                v.encode(&mut buf);
            }
            None => buf.push(0u8),
        }
        buf
    }));
    let buf = match encoded {
        Ok(b) => b,
        Err(_) => return CHILD_EXIT_PANIC,
    };
    // Anything the closure printed through C/C++ streams is ours alone now (the
    // parent flushed before forking), so flushing here cannot duplicate output.
    flush_std_buffers();
    if !write_all_fd(wr, &buf) {
        return CHILD_EXIT_WRITE_FAILED;
    }
    // SAFETY: descriptors (§ Safety) — the child's write end, on the one path
    // that reaches this line; the process exits immediately after.
    unsafe { libc::close(wr) };
    0
}

/// Parent half: drain the pipe while watching the clock, then reap and decode.
///
/// Draining concurrently is required, not an optimization — a payload larger
/// than the pipe buffer would otherwise block the child's `write` forever and
/// every large payload would look like a deadline miss.
#[cfg(unix)]
fn parent_wait<T: ForkPayload>(
    pid: libc::pid_t,
    rd: std::os::raw::c_int,
    kill_deadline: Instant,
) -> ForkOutcome<Option<T>> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 64 * 1024];

    loop {
        let now = Instant::now();
        if now >= kill_deadline {
            // SAFETY: `kill` (§ Safety).
            unsafe { libc::kill(pid, libc::SIGKILL) };
            let _ = reap(pid);
            // SAFETY: descriptors (§ Safety) — the parent's read end, on this path.
            unsafe { libc::close(rd) };
            return ForkOutcome::Killed { pid };
        }
        let slice = kill_deadline
            .duration_since(now)
            .as_millis()
            .min(POLL_SLICE_MS as u128)
            .max(1) as std::os::raw::c_int;

        let mut pfd = libc::pollfd {
            fd: rd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: buffers (§ Safety) — one live `pollfd` and a count of one;
        // `rd` is open for the whole call.
        let r = unsafe { libc::poll(&mut pfd, 1, slice) };
        if r < 0 {
            if last_errno() == Some(libc::EINTR) {
                continue;
            }
            // Snapshot the error before the cleanup syscalls clobber errno, and
            // kill before reaping: the child may still be running, and `reap`
            // blocks.
            let err = std::io::Error::last_os_error();
            // SAFETY: descriptors (§ Safety) — the read end, on this path.
            unsafe { libc::close(rd) };
            // SAFETY: `kill` (§ Safety).
            unsafe { libc::kill(pid, libc::SIGKILL) };
            let _ = reap(pid);
            return ForkOutcome::Failed(format!("poll failed: {err}"));
        }
        if r == 0 {
            continue;
        }

        // Readable (or hung up): one read cannot block now.
        // SAFETY: buffers (§ Safety) — only the returned prefix of `chunk` is
        // read back below.
        let n = unsafe { libc::read(rd, chunk.as_mut_ptr() as *mut libc::c_void, chunk.len()) };
        if n < 0 {
            if last_errno() == Some(libc::EINTR) {
                continue;
            }
            let err = std::io::Error::last_os_error();
            // SAFETY: descriptors (§ Safety) — the read end, on this path.
            unsafe { libc::close(rd) };
            // SAFETY: `kill` (§ Safety).
            unsafe { libc::kill(pid, libc::SIGKILL) };
            let _ = reap(pid);
            return ForkOutcome::Failed(format!("read failed: {err}"));
        }
        if n == 0 {
            break; // EOF: the child closed its end (finished or died)
        }
        buf.extend_from_slice(&chunk[..n as usize]);
    }

    // SAFETY: descriptors (§ Safety) — the read end, on the normal path out.
    unsafe { libc::close(rd) };
    let status = match reap(pid) {
        Some(s) => s,
        None => return ForkOutcome::Failed("waitpid failed".to_string()),
    };
    if !libc::WIFEXITED(status) {
        let sig = if libc::WIFSIGNALED(status) {
            libc::WTERMSIG(status)
        } else {
            -1
        };
        return ForkOutcome::Failed(format!("child died on signal {sig}"));
    }
    let code = libc::WEXITSTATUS(status);
    if code != 0 {
        let why = match code {
            CHILD_EXIT_PANIC => "panicked",
            CHILD_EXIT_WRITE_FAILED => "could not write its result",
            _ => "exited nonzero",
        };
        return ForkOutcome::Failed(format!("child {why} (exit {code})"));
    }

    let mut dec = Dec::new(&buf);
    match dec.get_u8() {
        Some(0) => ForkOutcome::Completed(None),
        Some(1) => match T::decode(&mut dec) {
            Some(v) => ForkOutcome::Completed(Some(v)),
            None => ForkOutcome::Failed("could not decode child result".to_string()),
        },
        _ => ForkOutcome::Failed("child result missing or corrupt".to_string()),
    }
}

/// Block until `pid` is reaped, returning its raw wait status. Retries `EINTR`
/// so a stray signal cannot leave a zombie behind.
#[cfg(unix)]
fn reap(pid: libc::pid_t) -> Option<std::os::raw::c_int> {
    loop {
        let mut status: std::os::raw::c_int = 0;
        // SAFETY: `waitpid` (§ Safety); `status` is a live `c_int` the call
        // fills in.
        let r = unsafe { libc::waitpid(pid, &mut status, 0) };
        if r == pid {
            return Some(status);
        }
        if r < 0 && last_errno() == Some(libc::EINTR) {
            continue;
        }
        return None;
    }
}

#[cfg(unix)]
fn write_all_fd(fd: std::os::raw::c_int, mut buf: &[u8]) -> bool {
    while !buf.is_empty() {
        // SAFETY: buffers (§ Safety) — the returned count is what the slice is
        // then advanced by, so the pointer stays inside it.
        let n = unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len()) };
        if n < 0 {
            if last_errno() == Some(libc::EINTR) {
                continue;
            }
            return false;
        }
        if n == 0 {
            return false;
        }
        buf = &buf[n as usize..];
    }
    true
}

#[cfg(unix)]
fn last_errno() -> Option<std::os::raw::c_int> {
    std::io::Error::last_os_error().raw_os_error()
}

/// Flush Rust's stdout/stderr and every C `FILE*` (Arjun's own logging goes
/// through the latter).
#[cfg(unix)]
fn flush_std_buffers() {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    // SAFETY: a null argument is C's documented "flush every open stream".
    unsafe { libc::fflush(std::ptr::null_mut()) };
}
