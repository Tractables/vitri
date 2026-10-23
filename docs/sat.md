# The SAT solver

vitri links one build of CaDiCaL, which its Arjun stack also uses, and
publishes it as [`vitri::sat`](https://docs.rs/vitri/latest/vitri/sat/); the
module documents the solver and why a program that uses vitri must not link a
second build of CaDiCaL. `Cargo.toml` declares `links = "vitri_arjun"` for the
whole vendored stack, so Cargo refuses to build vitri beside a crate that
declares the same key.
