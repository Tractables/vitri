# Contributing

Build prerequisites and the vendored C++ stack are covered in
[`docs/building.md`](docs/building.md). Everything below is about getting a
change accepted.

## The gate set

Every change must pass all four, warning-free:

```sh
cargo test --all-targets && cargo test --doc
cargo clippy --all-targets -- -D warnings
DOCS_RS=1 RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
cargo fmt --check
```

CI runs the same set (plus the MSRV in `Cargo.toml` and `cargo package`), so
there is no second standard to guess at.

## Behaviour rules

- **Caller-supplied input never panics a library path and is never silently
  ignored.** A bad value — a malformed spec, a flag meaningless in the current
  mode, an env knob the grammar cannot read, a build that does not belong to
  its formula — returns a `VitriError` naming the offending input (and, for an
  inert flag, the mode it needs). Panics are reserved for internal invariants
  a caller cannot violate.
- **One owner per vocabulary.** Modes, component policies, spec names, CLI
  flags and env knobs each live in one table that drives parsing, `--help`,
  and error messages alike; each env knob is read in exactly one place. Extend
  the table — never add a parallel list that would have to be kept in sync.
- **The library spawns no threads.** Consumers run many instances in parallel
  and own that decision; an extra thread here steals CPU from a peer.
- `vendor/<project>/upstream/` is third-party source; everything beside it in
  that directory is ours. `vendor/arjun/upstream/` is verbatim upstream —
  never edit it; behaviour changes go through the shim
  (`src/preprocess/arjun_lib/`, `build.rs`). `vendor/treedecomp/upstream/`
  carries a short list of in-place fixes, each marked `// vitri:` and stated
  in `THIRD-PARTY.md`; anything past that list belongs in `ffi.cpp` instead.

## Tests

- **Where a test lives is decided by what it needs to see.** Reaching only
  `pub`/`pub(crate)` items: the crate-root tree, `src/tests/<module>/`.
  Needing a module's private or `pub(super)` items: beside the module, in a
  `src/<module>/tests/` directory. CLI and end-to-end: `tests/`. No other
  `#[cfg(test)]` code belongs in a production file beyond the `mod tests;`
  registration line.
- **Names state the fact pinned**, as a sentence
  (`a_weight_with_a_zero_denominator_is_refused_by_name`) — one fact per
  test. Error-path tests assert the `VitriError` variant *and* that the
  message names the offending input; don't pin incidental output the docs do
  not promise.
- **Deterministic and fast**: fixed seeds, no wall-clock dependence, no
  sleeps, no `#[ignore]`, no external binaries.
- **No third-party test data ships.** Test formulas are hand-written tiny
  CNFs or generated in-tree (see `src/tests/circuit_fixture/`).
- **Bug fixes are test-first**: a regression test that fails on the parent
  commit, landed in the same commit as the fix, with the body saying so.
- One copy of every helper — `tests/common/` for integration tests, the
  `src/tests/**` helper modules for unit tests.

## Docs and commits

- A doc line earns its place by saying something **exists**, flagging a
  **trap**, or stating a **contract** — and never contains numbers that rot
  (test counts, runtimes, sizes).
- Commit subjects are imperative and name the behaviour or contract changed
  ("Refuse a spec token the family cannot honor"), not the mechanics of the
  edit.
