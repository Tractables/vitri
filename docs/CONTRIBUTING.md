# Contributing

Build prerequisites: [`building.md`](building.md).

## Checks

Before opening a pull request, run:

```sh
cargo test --all-targets && cargo test --doc
cargo clippy --all-targets -- -D warnings
DOCS_RS=1 RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
cargo fmt --check
```

CI runs these plus a build on the MSRV from `Cargo.toml`, `cargo package` and
the tests of the packaged crate, the RSDD example, and the slow guard. It runs
them `--release --locked`.

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
  Further changes go through `src/preprocess/arjun_lib/` and `build.rs`.

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
- Fixed seeds; no sleeps or external binaries. The one `#[ignore]` is a slow
  regression guard CI runs nightly.
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
- Commit subjects are imperative and describe the behaviour changed, e.g.
  "Refuse a spec token the family cannot honor".
