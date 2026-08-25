# The SAT solver

vitri statically links a SAT solver and publishes it as
[`vitri::sat`](https://tractables.github.io/vitri/vitri/sat/index.html). What the
handle does — the incremental interface, bounding a search with a terminator,
and reading the search's own counters — is documented on the items.

## One CaDiCaL per process

The solver is CaDiCaL, in the copy vendored under `vendor/arjun/upstream/` and
built from source by `build.rs`. The Arjun preprocessing stack is built against
that same copy, so a vitri process holds exactly one CaDiCaL — and that is why
the solver is public rather than private.

Two CaDiCaL builds in one process do not coexist. They export the same
`CaDiCaL::` symbols, so static linking resolves every call to whichever archive
the linker reached first — while each build's own headers are already compiled
into the struct layouts its callers use. Nothing warns. The program links, runs,
and corrupts its heap the first time a call crosses the seam.

`Cargo.toml` declares `links = "vitri_arjun"`. The key names the native library
`build.rs` produces, and Cargo permits one package with a given `links` value per
dependency graph. That reservation covers the whole vendored stack — CaDiCaL,
Arjun and CryptoMiniSat — so a second crate declaring it turns the collision into
a resolve-time error instead of a corrupted heap.

Adding a solver crate beside vitri is therefore not an option, and vitri does not
publish a solver-independent interface: a consumer that wants a different solver
runs it in a different process.
