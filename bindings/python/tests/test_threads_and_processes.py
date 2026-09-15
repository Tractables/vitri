"""Calls from Python threads, the forked Arjun stage, and the hard time limit
of examples/hard_timeout.py."""

import json
import multiprocessing
import os
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor

import pytest

import vitri
from cnfs import EXAMPLE, EXAMPLES, MALFORMED, TWO_COMPONENTS

# The spawned child imports the example by name, and inherits this path.
sys.path.insert(0, str(EXAMPLES))
import hard_timeout  # noqa: E402

linux_only = pytest.mark.skipif(
    not sys.platform.startswith("linux"),
    reason="counts threads in /proc and reads Linux child-process accounting",
)


def test_calls_from_several_threads_return_the_bundles_of_calls_made_one_by_one():
    inputs = [EXAMPLE, TWO_COMPONENTS] * 3
    one_by_one = [vitri.prepare(dimacs, vtree="minfill-primal").files for dimacs in inputs]
    with ThreadPoolExecutor(max_workers=4) as pool:
        together = list(
            pool.map(lambda dimacs: vitri.prepare(dimacs, vtree="minfill-primal").files, inputs)
        )
    assert together == one_by_one


# Calls `prepare` in a fresh interpreter, from its main thread or from a second
# thread, and reports the thread count at the call, the Arjun stage's outcome,
# and the peak memory of any child process the interpreter reaped. Its exit
# handler records the pid of every process that runs it.
CALLER = r"""
import atexit, json, os, resource, sys, threading
import vitri

exits, where = sys.argv[1], sys.argv[2]
dimacs = sys.stdin.buffer.read()

def record_exit():
    with open(exits, "a") as stream:
        stream.write(f"{os.getpid()}\n")

atexit.register(record_exit)

def threads():
    with open("/proc/self/status") as stream:
        return next(int(l.split()[1]) for l in stream if l.startswith("Threads:"))

answer = {"pid": os.getpid()}

def call():
    answer["threads"] = threads()
    result = vitri.prepare(dimacs, vtree="minfill-primal")
    answer["arjun"] = result.summary["stages"]["arjun"]
    answer["reduced"] = result.reduced_cnf

if where == "thread":
    worker = threading.Thread(target=call)
    worker.start()
    worker.join()
else:
    call()
answer["reaped_child_kib"] = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
print(json.dumps(answer))
"""


def call_in_fresh_interpreter(directory, where):
    directory.mkdir(parents=True, exist_ok=True)
    exits = directory / "exits"
    done = subprocess.run(
        [sys.executable, "-c", CALLER, str(exits), where],
        input=EXAMPLE,
        capture_output=True,
        check=True,
        timeout=600,
    )
    return json.loads(done.stdout), exits.read_text().split()


@linux_only
def test_from_a_single_thread_the_arjun_stage_forks_and_the_child_runs_no_python(tmp_path):
    answer, exits = call_in_fresh_interpreter(tmp_path, "main")
    assert answer["threads"] == 1
    assert answer["arjun"] == "ran"
    assert answer["reaped_child_kib"] > 0, "the stage should have run in a reaped child"
    assert exits == [str(answer["pid"])], "only the interpreter runs its exit handlers"


@linux_only
def test_with_a_second_thread_the_arjun_stage_runs_inline_with_the_same_result(tmp_path):
    forked, _ = call_in_fresh_interpreter(tmp_path / "forked", "main")
    inline, exits = call_in_fresh_interpreter(tmp_path, "thread")
    assert inline["threads"] == 2
    assert inline["arjun"] == "ran"
    assert inline["reaped_child_kib"] == 0, "no child process should have been made"
    assert exits == [str(inline["pid"])]
    assert inline["reduced"] == forked["reduced"]


def test_a_call_in_a_child_process_returns_the_summary_and_files_of_a_direct_call():
    summary, files = hard_timeout.prepare_with_timeout(EXAMPLE, 600, vtree="minfill-primal")
    direct = vitri.prepare(EXAMPLE, vtree="minfill-primal")
    assert files == direct.files
    assert summary == direct.summary


def test_an_error_in_the_child_process_is_raised_as_its_kind():
    with pytest.raises(vitri.InputError):
        hard_timeout.prepare_with_timeout(MALFORMED, 600)


def live_processes_carrying(entry_bytes):
    """Pids of live processes, other than this one, whose environment holds
    `entry_bytes`. A child inherits the environment, and so does a process
    the child forks."""
    found = []
    for pid in os.listdir("/proc"):
        if not pid.isdigit() or int(pid) == os.getpid():
            continue
        try:
            with open(f"/proc/{pid}/environ", "rb") as stream:
                environment = stream.read().split(b"\0")
            with open(f"/proc/{pid}/stat", "rb") as stream:
                state = stream.read().rsplit(b")", 1)[1].split()[0]
        except OSError:
            continue
        if entry_bytes in environment and state != b"Z":
            found.append(int(pid))
    return found


@linux_only
def test_a_call_past_the_limit_is_killed_with_every_process_it_forked(monkeypatch):
    token = f"{os.getpid()}-{time.monotonic_ns()}"
    monkeypatch.setenv("HARD_TIMEOUT_TEST_TOKEN", token)
    entry = f"HARD_TIMEOUT_TEST_TOKEN={token}".encode()
    # Large enough that preprocessing is still running when the limit passes.
    dimacs = random_3cnf(variables=20_000, clauses=85_000, seed=7)
    started = time.monotonic()
    with pytest.raises(TimeoutError):
        hard_timeout.prepare_with_timeout(dimacs, 5)
    assert time.monotonic() - started < 60
    assert multiprocessing.active_children() == []
    deadline = time.monotonic() + 10
    while live_processes_carrying(entry) and time.monotonic() < deadline:
        time.sleep(0.1)
    assert live_processes_carrying(entry) == []


def random_3cnf(variables, clauses, seed):
    import random

    generator = random.Random(seed)
    lines = [f"p cnf {variables} {clauses}"]
    for _ in range(clauses):
        chosen = generator.sample(range(1, variables + 1), 3)
        lines.append(" ".join(str(v if generator.random() < 0.5 else -v) for v in chosen) + " 0")
    return "\n".join(lines) + "\n"
