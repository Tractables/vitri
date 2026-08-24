# Vendored Arjun stack — provenance

These five trees are third-party source, vendored verbatim at the pinned commits
below and then trimmed (see § Trimming). They exist so `cargo build` can build
the whole Arjun preprocessing stack itself, with **no network access at build
time** — a published crate cannot shell out to `git clone`, and docs.rs builds
offline.

Upstream's CMake would `FetchContent` these at `GIT_TAG master`. `build.rs`
overrides that with `FETCHCONTENT_SOURCE_DIR_<NAME>` pointing here, and sets
`FETCHCONTENT_FULLY_DISCONNECTED=ON` so a missed override fails loudly instead of
silently reaching the network and building something we did not pin.

## Pins

| tree | origin | commit | licence | modified here |
|---|---|---|---|---|
| `arjun` | github.com/meelgroup/arjun (`release/v2.7.2`) | `6747e4c7659ec7107f3a3bef6c66e7ea0e2cf802` | MIT | yes — deadline, `GIT_SHA1` |
| `cadical` | github.com/meelgroup/cadical | `394c3f72858c2fe8cd35321f74f11f0f61c91123` | MIT | yes — friend declarations |
| `cryptominisat` | github.com/msoos/cryptominisat | `8433727f542e387336b608c724d8b0201b5dc436` | MIT (see below) | yes — deadline |
| `cadiback` | github.com/meelgroup/cadiback | `3b6a84062b1304433eb8960a4bff6b9a80de9c54` | MIT | yes — deadline |
| `sbva` | github.com/meelgroup/SBVA | `a41a3044cdbad2c5a99c4830568c73600636fca0` | MIT | no |

The dependency set is **closed** at these five: Arjun fetches CaDiCaL,
CryptoMiniSat, SBVA and (via CMS) CadiBack; CMS and CadiBack each fetch CaDiCaL,
which Arjun's `if(NOT TARGET cadical)` guard makes a single shared build. SBVA
fetches nothing. EvalMaxSAT is reachable only under `EXTRA_SYNTH`, which we do
not enable.

SBVA's pin was floating upstream (`GIT_TAG master`) and is pinned here for the
first time — a floating preprocessor dependency makes the *reduction itself*
irreproducible between installs.

## Modifications

**The source in this directory is already modified.** There is no patch step —
no `.patch` files ship, and `build.rs` applies nothing at build time. What builds
is exactly what you can read here. That is deliberate: a patch that silently
failed to apply would leave every caller believing the deadline below is armed
when it is not.

Every modified region is discoverable by diffing a tree here against a clean
checkout of its pinned commit in the table above, for example:

```sh
git clone https://github.com/meelgroup/arjun /tmp/arjun-pristine
git -C /tmp/arjun-pristine checkout $(cat vendor/arjun/upstream/ARJUN_PIN_SHA1)
diff -ru /tmp/arjun-pristine vendor/arjun/upstream/arjun
```

The differences are the trimming listed below plus these three changes:

- **The wall-clock deadline** (`arjun`, `cryptominisat`, `cadiback`). Additive:
  it gives the Arjun stack an in-process deadline (`Arjun::set_deadline`, driven
  from Rust through `arjun_shim_set_deadline_ms`) so a runaway preprocessing pass
  returns control instead of burning the whole budget.
- **`arjun/CMakeLists.txt`** — the `GIT_SHA1` block is wrapped in
  `if(NOT GIT_SHA1)`, so a value passed as `-DGIT_SHA1=` is honoured. Upstream
  derives it by running `git` in the source tree; a vendored tree has no `.git`,
  so the probe finds nothing and the built binary reports an **empty**
  `Arjun SHA1:`, which silently disables the version check consumers make on
  that string.
- **`cadical/src/cadical.hpp`** — two `vitri_cadical_*` functions are declared at
  global scope and named as friends of `CaDiCaL::Solver`. Additive, and nothing
  upstream calls them: they are defined in
  `vendor/arjun/cadical_internal_stats.cpp`, which is vitri's own file, and they
  are what lets the crate read a variable's search activity and the CDCL
  counters that the solver's public class does not expose.

To re-vendor at a newer upstream: re-clone at the new commits, re-apply all
three changes, re-run the trimming below, and update this table.

`ARJUN_PIN_SHA1` in this directory holds the pinned Arjun commit as a plain text
file and is the single source of truth for it. `build.rs` reads it (`arjun_pin`)
and passes it to CMake as `-DGIT_SHA1=`, so the version the built library reports
is the commit recorded here and cannot drift from it. It lives inside the package
because Cargo's `include` allowlist cannot reach outside the crate root.

## Licences

All five trees are MIT. CryptoMiniSat's `LICENSE.txt` states the principle
explicitly: "Everything that's needed to run/build/install/link the system is MIT
licensed" — the GPL-licensed parts of that repository are optional components we
neither build nor link.

Two non-MIT items are worth stating plainly:

- **Eigen 3.4.0**, bundled inside `sbva/eigen-3.4.0/`, is **MPL-2.0** primarily,
  with some files under BSD or LGPL 2.1. We compile SBVA with `EIGEN_MPL2_ONLY`,
  which makes `#include`-ing any LGPL-licensed Eigen header a **compile error**.
  So the MPL2-only property is enforced by the build, not by inspection. MPL-2.0
  is file-level copyleft with no relinking obligation, and is compatible with
  redistributing this crate under Apache-2.0.
- **GMP, MPFR and zlib** are NOT vendored. They are system libraries and link
  **dynamically, always**. GMP and MPFR are LGPL; folding them statically would
  attach LGPL relinking obligations to every binary built from this
  Apache-2.0 crate. There is deliberately no feature, env var or flag that
  switches to a static fold.

## Trimming

Removed from the upstream trees, to keep the published crate small. Nothing
removed is reachable from the build: `cargo build` from a freshly unpacked
`.crate`, offline, is what proves it.

- every `.git/` and `.github/`
- editor and assistant scratch files carried in the upstream repositories:
  `.vimsettings.vim`, `CLAUDE.md`, and Arjun's `ideas.md`,
  `IDEAS-3-categories.md`, `bug_real.md` and `bug_real.cnf`
- CryptoMiniSat `tests/`, `scripts/fuzz/`, `utils/{gtest,minimal_cms,lingeling-ala}`
  — reached only under `ENABLE_TESTING`, which `build.rs` leaves off
- documentation, bindings and developer tooling with no CMake reference:
  `cadical/{test,scripts}`, `cryptominisat/{docs,html,documents,python,scripts,utils}`,
  `arjun/{scripts,html,documents}`, `sbva/{examples,python,scripts}`,
  `cadiback/{test,fuzzing}`. `cadical/contrib` is **kept** — CMake `GLOB`s it into
  the build.
- Eigen, in two steps. First everything outside `Eigen/` and the `COPYING.*`
  files: SBVA consumes the tree as a bare include directory
  (`sbva/src/CMakeLists.txt`), so Eigen's own benchmarks, tests, docs, BLAS and
  LAPACK trees were unreachable. Then the unreachable *modules*: the header
  closure of SBVA's three translation units (`g++ -M`, normalised) is exactly
  `Eigen/{Core,SparseCore}` plus `Eigen/src/{Core,plugins,SparseCore}`, so the
  ~20 solver modules (LU, QR, SVD, Cholesky, Eigenvalues, Geometry, the
  `*Support` backends, …) and their top-level headers went. `Eigen/src/misc/` is
  kept apart from `lapacke.h` (1.1 MB, reachable only from the deleted LAPACKE
  module headers) — `src/Core/util/MKL_support.h` includes `../../misc/blas.h`
  under `#if defined(EIGEN_USE_BLAS)`, which we never define but which stays
  intact rather than dangling.

Together: 20 MB of Eigen to 5 MB, and the whole vendored stack to about 11 MB.

When computing a closure like this with `g++ -M`, note that GCC does **not**
normalise `..` in its output — Eigen's `#include "../plugins/…"` lines appear as
`Eigen/src/Core/../plugins/…`, so a naive path split attributes them to `Core`
and reports `plugins` as unreachable. It is not.
