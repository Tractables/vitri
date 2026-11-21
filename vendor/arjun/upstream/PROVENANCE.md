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
| `arjun` | github.com/meelgroup/arjun (`release/v2.7.2`) | `6747e4c7659ec7107f3a3bef6c66e7ea0e2cf802` | MIT | yes — deadline, progress checkpoint, work clock, `GIT_SHA1` |
| `cadical` | github.com/meelgroup/cadical | `394c3f72858c2fe8cd35321f74f11f0f61c91123` | MIT | yes — friend declarations, search ticks, work counts |
| `cryptominisat` | github.com/msoos/cryptominisat | `8433727f542e387336b608c724d8b0201b5dc436` | MIT (see below) | yes — deadline, work clock |
| `cadiback` | github.com/meelgroup/cadiback | `3b6a84062b1304433eb8960a4bff6b9a80de9c54` | MIT | yes — deadline, work clock |
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

The differences are the trimming listed below plus these changes:

- **The wall-clock deadline** (`arjun`, `cryptominisat`, `cadiback`). Additive:
  it gives the Arjun stack an in-process deadline (`Arjun::set_deadline`, driven
  from Rust through `arjun_shim_set_deadline_ms`) so a runaway preprocessing pass
  returns control instead of burning the whole budget. It is checked only where
  stopping early leaves a sound, less-reduced result. One such place is
  CryptoMiniSat's fast backward search (`Searcher::new_decision_fast_backw`),
  the single solve in which Arjun walks every independent-support candidate:
  once the deadline has passed, each remaining candidate takes the branch the
  search already takes for a candidate that runs out of conflicts and is kept
  as independent, so only candidates proven defined leave the support. The
  search reads the clock once every 1024 free decisions, as the oracle does
  every 1024 propagations, since where `clock_gettime` has no fast path a read
  at every decision can cost more than the search itself; `FastBackwData`
  carries the count and a flag that stays set once the deadline is seen.
  CryptoMiniSat's inprocessing token loops (`Solver::execute_inprocess_strategy`
  and `OccSimplifier::execute_simplifier_strategy`) also stop at the deadline,
  between two tokens, while `SATSolver::set_deadline_ends_simplify` is on
  (`SolverConf::deadline_ends_simplify`). `Puura::get_fully_simplified_renumbered_cnf`
  turns it on around its two loops of simplification rounds, which hold no
  token the result needs and otherwise run to the end however late it is;
  everywhere else it is off, since a stop there could skip `must-renumber`.
- **The stage-1 progress checkpoint** (`arjun`). Additive, and inert unless set:
  `Arjun::set_progress_checkpoint` (driven from Rust through
  `arjun_shim_set_progress_checkpoint_ms`) places a wall-clock checkpoint and a
  minimum shrink fraction on independent-support minimization. In
  `Minimize::backward_round`, at the top of the loop and right after the
  deadline check (the one place stopping is sound, for the reason given in the
  comment there), the first visit after the checkpoint compares the support
  size minimization started with (before its own simplification) against the most it could shrink to now (the
  variables proven independent plus those still unknown). If it shrank by less
  than the fraction, the round breaks and `Arjun::stopped_no_progress()`
  reports it; the caller decides what to do. Files: `arjun/src/{config.h,
  minimize.h,minimize.cpp,backward.cpp,arjun.h,arjun.cpp}`. Unset (the default) it is
  bit-identical.
- **The work clock** (`cryptominisat`, `arjun`, `cadiback`). Inert unless
  started: `cryptominisat/src/work_clock.h` (new, header-only, so the oracle
  library shares it without a link dependency) holds one per-thread clock (a
  plain object in an Emscripten build, which links without threads) that every deadline and checkpoint above is a point on. Each wall comparison
  against a deadline (in Arjun's `backward.cpp`, `extend.cpp`, `arjun.cpp` and
  `puura.cpp`, and in CryptoMiniSat's `searcher.cpp`, `oracle_use.cpp` and
  `oracle/oracle.cpp`) reads it instead, and `Arjun::set_deadline` and
  `set_progress_checkpoint` set their points on it. Until
  `CMSat::start_work_clock` (declared in `cryptominisat.h`) is called it is
  `real_time_sec()`, so every comparison is the one it replaced. Started, it
  counts each kind of work (`WorkKind`) and charges each a caller-given
  number of units: the literals `PropEngine::propagate_any_order` and the
  oracle's `Propagate` propagate, the watches they visit, the binary watches
  among them, the long clauses they dereference past the blocking literal and,
  in the oracle, the literals scanned for a new watch; the search's conflicts
  (`Searcher::handle_conflict`); the oracle's conflicts (`Oracle::CDCLBT`) and
  the literals and trail entries its conflict analysis and minimization visit
  (`LearnUip`, `LitReduntant`), the solution cache entries it checks and
  stores (`SatByCache`, `AddSolToCache`) and the assignments it undoes
  (`UnDecide`); the bogoprops of `propagate_light`,
  `propagate_occur` and `hyperengine.cpp`; the oracle's calls (`Oracle::Solve`)
  and the variables and literals each oracle is built from; the clauses a flip
  of the oracle's local search (`ccnr_oracle.cpp`) visits and the flips;
  CaDiCaL's propagations (search and inprocessing), conflicts, search ticks,
  learned literals and subsumption checks, read through a new inline
  `vitri_cadical_work` in `cadical/src/cadical.hpp` (over
  `vitri_cadical_search_stats`), after each solve of Arjun's `MetaSolver`
  (`CMSat::charge_cadical_solve`, which also counts the solve) and at each poll
  of CadiBack's terminator; the literals of the formula at each
  `OccSimplifier::simplify` and each `Puura::run_sbva`
  (`CMSat::charge_sbva_run`); and the polls themselves. `CadiBack::doit` takes
  the clock's test as a new optional `passed` hook (`backbone.cpp` passes the
  clock's), called with the CaDiCaL work since the previous test; the deadline
  test before each of CadiBack's solver calls makes the same poll. A deadline
  then also passes once the wall time since the start is a caller-given
  multiple of it, and the clock records that it did.
- **`arjun/CMakeLists.txt`** — the `GIT_SHA1` block is wrapped in
  `if(NOT GIT_SHA1)`, so a value passed as `-DGIT_SHA1=` is honoured. Upstream
  derives it by running `git` in the source tree; a vendored tree has no `.git`,
  so the probe finds nothing and the built binary reports an **empty**
  `Arjun SHA1:`, which silently disables the version check consumers make on
  that string.
- **`cadical/src/cadical.hpp`** — two `vitri_cadical_*` functions are declared at
  global scope and named as friends of `CaDiCaL::Solver`, and an inline
  `vitri_cadical_work` built on one of them reads the counts the work clock
  charges. Additive, and nothing
  upstream calls them: they are defined in
  `vendor/arjun/cadical_internal_stats.cpp`, which is vitri's own file, and they
  are what lets the crate read a variable's search activity and the CDCL
  counters that the solver's public class does not expose.
- **`cadical/src/stats.hpp`, `cadical/src/propagate.cpp`** — a counter
  `stats.searchticks`: search propagation adds one per propagated literal, one
  per cache line of the watch list it visits and one per long clause it
  dereferences, as later CaDiCaL releases count their ticks. It is kept in a
  local and added once per call, beside `stats.propagations.search`, and
  nothing upstream reads it.

To re-vendor at a newer upstream: re-clone at the new commits, re-apply all
changes listed above, re-run the trimming below, and update this table.

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
