#!/usr/bin/env python3
"""Run the commands the documentation shows and compare their output with it.

Usage: python3 .github/scripts/check_docs.py [--self-test]

Each `sh` block of `README.md` and `docs/*.md` that is followed by an
`Output:` or a `Stderr:` block runs with `sh`, and the lines of that text
block must occur, consecutively and as whole lines, in what the command wrote
to stdout or to stderr. A command with an `Output:` block must also exit with
status 0. A number of milliseconds is a measurement, so `60 ms` on the page
matches any number of milliseconds. Blocks without either label do not run.

The blocks of a page run in page order in one scratch directory, as a reader
following the page would run them, so a block reads the files an earlier
block wrote. Unlabelled blocks, a page's install steps among them, do not run,
so the environment has to provide the commands:

- `vitri` is this checkout's release binary, from `target/release` or
  `$CARGO_TARGET_DIR/release`, which goes first on the PATH. Build it with
  `cargo build --release` first.
- The tutorial runs `python`, which must import this checkout's Python package
  and the compilers the tutorial installs. In a virtual environment whose
  `bin` is on the PATH, `pip install ./bindings/python tididi tididi-cnf`.
  Its RSDD route runs `cargo`.
- Downloads of this repository's files come from the checkout, so a branch is
  checked against its own files: `curl` of a URL under
  https://raw.githubusercontent.com/Tractables/vitri/main/ copies the file,
  and `git clone https://github.com/Tractables/vitri.git` links the checkout.
  Other uses of `curl` and `git` run the installed ones.

Exits nonzero, naming the page, the line and the command, when anything
differs.
"""

import os
import re
import shlex
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BLOCK = re.compile(r"^```([^\n]*)\n(.*?)^```[ \t]*$", re.M | re.S)
LABELS = {"\n\nOutput:\n\n": "stdout", "\n\nStderr:\n\n": "stderr"}
MILLISECONDS = re.compile(r"\b\d+ ms\b")
RAW = "https://raw.githubusercontent.com/Tractables/vitri/main/"
CLONE = "https://github.com/Tractables/vitri.git"

# Each stand-in drops its own directory, the first entry of the PATH, before
# running the installed command.
CURL = """#!/bin/sh
url= out= previous=
for arg; do
  [ "$previous" = -o ] && out=$arg
  case $arg in {raw}*) url=$arg ;; esac
  previous=$arg
done
if [ -n "$url" ]; then
  exec cp {root}/"${{url#{raw}}}" "${{out:-/dev/stdout}}"
fi
PATH=${{PATH#*:}}
exec curl "$@"
"""
GIT = """#!/bin/sh
if [ "$1" = clone ]; then
  dir= found=
  for arg; do
    if [ -n "$found" ]; then dir=$arg; break; fi
    [ "$arg" = {clone} ] && found=1
  done
  [ -n "$found" ] && exec ln -s {root} "${{dir:-vitri}}"
fi
PATH=${{PATH#*:}}
exec git "$@"
"""


def output_blocks(markdown):
    """Each labelled text block: the line of the block just before it, the
    stream it shows, that block's language and body, and its own body."""
    blocks = list(BLOCK.finditer(markdown))
    for before, block in zip(blocks, blocks[1:]):
        language, body = block.groups()
        stream = LABELS.get(markdown[before.end():block.start()])
        if language == "text" and stream:
            line = markdown.count("\n", 0, before.start()) + 1
            yield line, stream, before.group(1), before.group(2), body


def occurs(expected, actual):
    """Whether the expected lines occur in actual as consecutive whole lines,
    with any number of milliseconds matching any other."""
    if actual and not actual.endswith("\n"):
        actual += "\n"
    expected, actual = (MILLISECONDS.sub("N ms", text) for text in (expected, actual))
    return ("\n" + expected) in ("\n" + actual)


def environment(shims, binaries):
    """The commands' environment: the stand-ins in `shims`, then the release
    binaries, then the PATH this script was given."""
    shims.mkdir(exist_ok=True)
    values = {"raw": RAW, "clone": shlex.quote(CLONE), "root": shlex.quote(str(ROOT))}
    for name, script in (("curl", CURL), ("git", GIT)):
        path = shims / name
        path.write_text(script.format(**values), encoding="utf-8")
        path.chmod(0o755)
    return dict(os.environ, PATH=os.pathsep.join([str(shims), str(binaries), os.environ["PATH"]]))


def check_page(name, markdown, env, failures):
    """Run the page's labelled commands in order in one scratch directory.
    Returns how many ran."""
    checked = 0
    with tempfile.TemporaryDirectory() as scratch:
        for line, stream, language, command, expected in output_blocks(markdown):
            if language != "sh":
                continue
            run = subprocess.run(["sh", "-c", command], cwd=scratch, env=env,
                                 stdin=subprocess.DEVNULL, capture_output=True, text=True)
            actual = run.stdout if stream == "stdout" else run.stderr
            if stream == "stdout" and run.returncode != 0:
                failures.append(f"{name}:{line}: `{command.strip()}` exited with status {run.returncode}\n"
                                f"stdout:\n{run.stdout}stderr:\n{run.stderr[-4000:]}")
            elif not occurs(expected, actual):
                failures.append(f"{name}:{line}: `{command.strip()}` printed on {stream}\n"
                                f"{actual}which does not contain\n{expected}")
            checked += 1
    return checked


def self_test():
    page = ("```sh\nvitri a.cnf\n```\n\nOutput:\n\n```text\nreduced: 9 vars\n```\n\n```text\nnot output\n```\n"
            "\n```sh\nvitri b.cnf\n```\n\nStderr:\n\n```text\nerror: no\n```\n")
    assert list(output_blocks(page)) == [(1, "stdout", "sh", "vitri a.cnf\n", "reduced: 9 vars\n"),
                                         (15, "stderr", "sh", "vitri b.cnf\n", "error: no\n")]
    assert occurs("b\nc\n", "a\nb\nc\n")
    assert occurs("c\n", "a\nc")
    assert not occurs("a\nc\n", "a\nb\nc\n"), "the lines are consecutive"
    assert not occurs("b\n", "ab\n"), "a block matches whole lines only"
    assert occurs("elapsed: 60 ms\n", "elapsed: 7 ms\n")
    assert not occurs("9 vars\n", "8 vars\n")
    with tempfile.TemporaryDirectory() as scratch:
        env = environment(Path(scratch) / "bin", Path(scratch))
        fetch = f"curl -fL {RAW}docs/example.cnf -o a.cnf\ngit clone --depth 1 {CLONE}\ncat a.cnf\n"
        pages = ("```sh\necho written > f; echo done >&2\n```\n\nStderr:\n\n```text\ndone\n```\n"
                 "\n```sh\ncat f\n```\n\nOutput:\n\n```text\nwritten\n```\n"
                 f"\n```sh\n{fetch}```\n\nOutput:\n\n```text\n{(ROOT / 'docs/example.cnf').read_text()}```\n"
                 "\n```sh\nls vitri/.github/scripts/check_docs.py\n```\n\nOutput:\n\n```text\n"
                 "vitri/.github/scripts/check_docs.py\n```\n")
        failures = []
        assert check_page("pages.md", pages, env, failures) == 4 and not failures, failures
        failures = []
        wrong = ("```sh\necho 8 vars\n```\n\nOutput:\n\n```text\n9 vars\n```\n"
                 "\n```sh\necho 9 vars; false\n```\n\nOutput:\n\n```text\n9 vars\n```\n")
        assert check_page("wrong.md", wrong, env, failures) == 2
        assert [failure.split(":")[1] for failure in failures] == ["1", "11"], failures
    print("self-test: ok")


def main():
    if sys.argv[1:] == ["--self-test"]:
        self_test()
        return
    if sys.argv[1:]:
        sys.exit(__doc__)
    binaries = Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target").resolve() / "release"
    if not (binaries / "vitri").exists():
        sys.exit(f"{binaries / 'vitri'} is missing: run cargo build --release first")
    failures = []
    with tempfile.TemporaryDirectory() as shims:
        env = environment(Path(shims), binaries)
        for page in [ROOT / "README.md", *sorted((ROOT / "docs").glob("*.md"))]:
            name = page.relative_to(ROOT)
            checked = check_page(name, page.read_text(encoding="utf-8"), env, failures)
            if checked:
                print(f"{name}: {checked} command(s) run")
    for failure in failures:
        print(failure, file=sys.stderr)
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
