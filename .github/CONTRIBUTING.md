# Contributing

Build prerequisites: [`docs/building.md`](../docs/building.md).

## Checks

Before opening a pull request, run:

```sh
cargo test --all-targets && cargo test --doc
cargo clippy --all-targets -- -D warnings
DOCS_RS=1 RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
cargo fmt --check
cargo fmt --manifest-path examples/rsdd-count/Cargo.toml --check
cargo clippy --manifest-path examples/rsdd-count/Cargo.toml -- -D warnings
```

CI runs these `--release --locked`, plus a `cargo check` on the MSRV from
`Cargo.toml`, `cargo package` and the tests of the packaged crate, and the slow
guards. It also runs `.github/scripts/check_docs.py`, which runs each `sh`
block of the README and `docs/*.md` that is followed by an `Output:` or
`Stderr:` block and checks that block against what the command printed; the
script's docstring says what a local run needs. CI runs on each pull request,
nightly and on manual dispatch.

The C, Python and Wasm bindings are separate Cargo workspaces. After a change
to the core, run their format and lint checks:

```sh
(cd bindings/c && cargo fmt --check && cargo clippy --release --all-targets --locked -- -D warnings)
(cd bindings/python && cargo fmt --check && PYO3_PYTHON=$(command -v python3) cargo clippy --release --locked -- -D warnings)
(cd bindings/wasm && cargo fmt --check)
```

The Wasm clippy run needs the Emscripten setup in
[`docs/building.md`](../docs/building.md#building-for-emscripten). The C,
Python, release and Web build workflows run on pull requests that touch their
inputs; they also run the C examples, the Python tests and the built Wasm
module. Each binding's README covers its build.

## Code

- Invalid caller input returns a `VitriError` that names the input; for a flag
  that has no effect in the current mode, the error also names the mode it
  needs. Library code panics only on internal invariants.
- Modes, component policies, spec names, CLI flags and env variables are each
  defined in one table that drives parsing, `--help` and error messages. Add to
  the table rather than keeping a second list. Each env variable is read in one
  place.
- The library does not spawn threads; callers run many instances in parallel.
- `vendor/arjun/upstream/` is modified only as its `PROVENANCE.md` records.
  Further changes go through `src/preprocess/arjun_lib/` and `build/`.

## Tests

- Tests that use only `pub`/`pub(crate)` items go in `src/tests/<module>/`;
  tests that need a module's private items go in `src/<module>/tests/`; CLI
  and end-to-end tests go in `tests/`. Production files contain no
  `#[cfg(test)]` code other than the `mod tests;` line, and `lib.rs`'s
  `extern crate self as vitri;`, which lets the shared helpers name the crate.
- A test name states the fact being checked
  (`a_weight_with_a_zero_denominator_is_refused_by_name`), one per test.
  Error-path tests check the `VitriError` variant and that the message names
  the input; don't assert on output the docs don't promise.
- Fixed seeds; no sleeps or external binaries. Slow and wall-clock guards use `#[ignore]`; CI runs them separately.
- Fixtures are small hand-written CNFs or generated in-tree
  (`src/tests/circuit_fixture/`). The one exception is `tests/data/agg/`, whose
  CNFs are components of public model-counting competition instances; the
  aggregate ranker is fitted and checked against recorded pairs.
- A bug fix comes with a regression test that fails on the parent commit, in
  the same commit.
- Shared helpers live in `tests/common/` (integration) and under `src/tests/`
  (unit).

## Docs and commits

- Docs state what exists, what to watch out for, and what is guaranteed. No
  numbers that go stale (test counts, runtimes, sizes).
- An item's rustdoc is its documentation. A guide in `docs/` says the
  capability exists and names the item; how it behaves goes on the item.
- Commit subjects are imperative and describe the behaviour changed, e.g.
  "Refuse a spec token the family cannot honor".
