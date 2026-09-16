"""Run vitri.prepare under a hard wall-clock limit.

`budget_ms` is checked between the steps of a run, so a run can pass it. To
stop a call at a fixed time, make it in a child process and kill the child at
the limit. The `spawn` start method gives the child a fresh interpreter, which
has one thread, so the library's own forked Arjun stage stays available in it,
and on Linux a process it forks is killed with the child.

    python hard_timeout.py formula.cnf 30
"""

import multiprocessing
import sys

import vitri


def _call_in_child(connection, function, args):
    try:
        connection.send((True, function(*args)))
    except Exception as error:
        connection.send((False, error))
    finally:
        connection.close()


def call_with_timeout(function, seconds, *args):
    """Return `function(*args)`, called in a child process started with
    `spawn`.

    `function` must be importable by name, and its arguments, result and any
    exception it raises must pickle. Raises `TimeoutError` if the call has not
    finished after `seconds`, and the exception the call raised if it failed.
    The child is killed either way.
    """
    context = multiprocessing.get_context("spawn")
    receiver, sender = context.Pipe(duplex=False)
    child = context.Process(
        target=_call_in_child, args=(sender, function, args), daemon=True
    )
    child.start()
    sender.close()
    try:
        if not receiver.poll(seconds):
            raise TimeoutError(f"the call did not finish in {seconds} s")
        succeeded, value = receiver.recv()
    except EOFError:
        raise RuntimeError(
            f"the child process ended without an answer (exit code {child.exitcode})"
        ) from None
    finally:
        receiver.close()
        child.kill()
        child.join()
    if not succeeded:
        raise value
    return value


def _prepare(dimacs, settings):
    result = vitri.prepare(dimacs, **settings)
    return result.summary, result.files


def prepare_with_timeout(dimacs, seconds, **settings):
    """Return `(summary, files)` of `vitri.prepare(dimacs, **settings)`.

    Raises `TimeoutError` if the call has not finished after `seconds`, and
    the `vitri.VitriError` subclass the call raised if it failed.
    """
    return call_with_timeout(_prepare, seconds, dimacs, settings)


if __name__ == "__main__":
    path, seconds = sys.argv[1], float(sys.argv[2])
    with open(path, "rb") as stream:
        summary, files = prepare_with_timeout(stream.read(), seconds)
    print(summary["status"], "with", len(files), "files")
