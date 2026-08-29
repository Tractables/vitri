# Agent Instructions — vitri

`CLAUDE.md` is the one real instruction file in this repository.
`AGENTS.md` is a relative symlink to it so Claude and Codex read the same
rules. Edit this file, not the symlink.

This is the public source repository for vitri, a Rust library and CLI for CNF
preprocessing and vtree construction. Library code, CLI code, tests, consumer
documentation and vendored dependencies live here. Every tracked file and
commit is public.

**[`CONTRIBUTING.md`](docs/CONTRIBUTING.md) binds in full** — the gate set, the
behaviour rules, the test layout and style, the doc and commit conventions.
Operational notes on top of it:

- Run the whole gate set before reporting a change done, and run tests with
  `--all-targets` — the lib target alone misses the CLI, decompose and
  round-trip suites.
- For a bug fix, confirm the new regression test fails on the unfixed parent
  commit before writing the fix.
- Prefer extending an existing table, test file, or fixture over standing up
  a parallel one; if a helper you need exists anywhere, lift it to the shared
  spot rather than copying it.
- The first build compiles the vendored C++ stack and takes minutes;
  subsequent builds are cached. If the C++ compiler is too old, set
  `VITRI_CXX` (see `docs/building.md`).
- User-facing behaviour lives in `docs/` and `--help`; when you change
  behaviour, change its documentation in the same commit.
