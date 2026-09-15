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


def _prepare_in_child(connection, dimacs, settings):
    try:
        result = vitri.prepare(dimacs, **settings)
        connection.send((result.summary, result.files))
    except vitri.VitriError as error:
        connection.send(error)
    finally:
        connection.close()


def prepare_with_timeout(dimacs, seconds, **settings):
    """Return `(summary, files)` of `vitri.prepare(dimacs, **settings)`.

    Raises `TimeoutError` if the call has not finished after `seconds`, and
    the `vitri.VitriError` subclass the call raised if it failed.
    """
    context = multiprocessing.get_context("spawn")
    receiver, sender = context.Pipe(duplex=False)
    child = context.Process(
        target=_prepare_in_child, args=(sender, dimacs, settings), daemon=True
    )
    child.start()
    sender.close()
    try:
        if not receiver.poll(seconds):
            raise TimeoutError(f"vitri.prepare did not finish in {seconds} s")
        answer = receiver.recv()
    except EOFError:
        raise RuntimeError(
            f"the child process ended without an answer (exit code {child.exitcode})"
        ) from None
    finally:
        receiver.close()
        child.kill()
        child.join()
    if isinstance(answer, vitri.VitriError):
        raise answer
    return answer


if __name__ == "__main__":
    path, seconds = sys.argv[1], float(sys.argv[2])
    with open(path, "rb") as stream:
        summary, files = prepare_with_timeout(stream.read(), seconds)
    print(summary["status"], "with", len(files), "files")
