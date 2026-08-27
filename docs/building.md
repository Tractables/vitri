# Building

```sh
cargo build --release
```

Cargo builds the C++ sources carried by vitri and goatd. No build step downloads
source, and there is no install script or out-of-tree state.

`build.rs` reads this file and warns when an install command it knows about has
stopped appearing here, so the commands below stay in step with the source.

## Toolchain

- **Rust**, no older than the `rust-version` in `Cargo.toml`.
- **A C++20 compiler.** GCC 12 or newer — Arjun uses `constexpr std::vector`
  copies, which GCC 11 (still the default on Ubuntu 22.04) cannot compile.
  `build.rs` looks for `g++-14`, then `g++-13`, then `g++-12` on `PATH`, and
  falls back to plain `g++`; override with `VITRI_CXX`. The goatd dependency
  uses the same search for its FlowCutter backend and accepts
  [GOATD_CXX](https://github.com/Tractables/goatd/blob/main/docs/building.md).
- **CMake**, to build the Arjun stack.
- **`pkg-config`**, which that stack's CMake projects use to locate GMP.
- **GMP, MPFR and zlib development packages.**

```sh
sudo apt install build-essential gcc-12 g++-12 cmake pkg-config libgmp-dev libmpfr-dev zlib1g-dev   # Debian/Ubuntu
sudo dnf install gcc-c++ cmake pkgconf-pkg-config gmp-devel mpfr-devel zlib-devel                   # Fedora/RHEL
brew install cmake pkg-config gmp mpfr zlib                                                         # macOS
```

The first build takes a few minutes because it compiles Arjun, CryptoMiniSat,
CaDiCaL, cadiback and SBVA. Cargo caches the result; later builds do not repeat
it.

## Documentation builds

docs.rs has no network and cannot install system packages, so `build.rs` skips
the native build when `DOCS_RS` is set. rustdoc type-checks but never links, so
the whole API still renders; that path produces no working binary.

## The vendored Arjun stack

`vendor/arjun/upstream/` holds five third-party CMake projects — Arjun,
CryptoMiniSat, CaDiCaL, cadiback and SBVA — pinned at exact commits, with the
wall-clock-deadline modification already applied to the source here. There is no
patch step and no `.patch` file.
[`vendor/arjun/upstream/PROVENANCE.md`](../vendor/arjun/upstream/PROVENANCE.md)
records the commits, the licences, every modification and what was trimmed.

`build.rs` drives CMake over them with `FETCHCONTENT_FULLY_DISCONNECTED=ON`, so
the build never reaches the network. It builds out-of-source, into `OUT_DIR`,
and never writes to the crate's own tree.

Beside the CMake projects, `build.rs` compiles two small translation units of
its own: the C ABI shims, and one that reads CaDiCaL's internal search counters.
That second one includes CaDiCaL's internal header, whose struct layouts depend
on the preprocessor defines the solver was built with, so it is compiled with
exactly CaDiCaL's own set, and `build.rs` fails the build if one of those
defines stops appearing in the vendored `CMakeLists.txt`.

`VITRI_CXX` and the `VITRI_*` run-time knobs are all listed in
[`env.md`](env.md).

### Relocating a build

The C++ becomes static archives in Cargo's build directories: vitri's
`build.rs` produces `libvitri_arjun.a`, and the goatd dependency builds its
FlowCutter archive. Both are linked into the executable, so the binary needs no
companion library or rpath.

GMP and MPFR are linked dynamically and must be present on the running machine.
See [`THIRD-PARTY.md`](../THIRD-PARTY.md).

## The binary source file

`src/cli_main.rs`, beside the library rather than under `src/bin/`, with an
explicit `[[bin]] path` in `Cargo.toml`. `cargo build` and `cargo install` are
unaffected.
