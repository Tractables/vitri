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
  `build.rs` looks for a versioned `g++` on `PATH`, newest first, and falls back
  to plain `g++`; override with `VITRI_CXX`. The goatd dependency uses the same
  search for its FlowCutter backend and accepts
  [GOATD_CXX](https://github.com/Tractables/goatd/blob/main/docs/building.md).
- **CMake**, to build the Arjun stack.
- **`pkg-config`**, which that stack's CMake projects use to locate GMP.
- **GMP, MPFR and zlib development packages.**

```sh
sudo apt install build-essential gcc-12 g++-12 cmake pkg-config libgmp-dev libmpfr-dev zlib1g-dev   # Debian/Ubuntu
sudo dnf install gcc-c++ cmake pkgconf-pkg-config gmp-devel mpfr-devel zlib-devel                   # Fedora/RHEL
```

macOS is not supported: the archive-merging step needs GNU `ar`, whose MRI
script mode Apple's `ar` does not have. The released binaries are Linux.

To build against a GMP installed under another prefix, set `PKG_CONFIG_PATH`,
`CPATH` and `LIBRARY_PATH` to its `lib/pkgconfig`, `include` and `lib`
directories; the binary then needs that `lib` directory on its library search
path at run time.

The first build takes a few minutes because it compiles Arjun, CryptoMiniSat,
CaDiCaL, cadiback and SBVA. Cargo caches the result; later builds do not repeat
it.

## Building for Emscripten

`bindings/wasm` builds vitri for `wasm32-unknown-emscripten`, and
`.github/workflows/wasm.yml` is a complete build from a clean machine. Besides
CMake, pkg-config and the Emscripten SDK on `PATH`, the target needs:

- **GMP and MPFR built for Emscripten**, installed into one prefix that
  `VITRI_EMSCRIPTEN_PREFIX` names. Arjun's CMake and headers need MPFR, though
  nothing vitri links calls it.
- **GMP's side modules**, `lib/libgmp.so` and `lib/libgmpxx.so` in that prefix.
  `bindings/wasm/emscripten-prefix.sh` builds both libraries and links the side
  modules into a prefix.
- **A main module that names the side modules**: the program is linked with
  `-sMAIN_MODULE=2` and with the paths of both side modules, which vitri's build
  script publishes to dependents as `DEP_VITRI_ARJUN_SIDE_MODULES`.
  `bindings/wasm` does this in `.cargo/config.toml` and `build.rs`. C++ from
  other crates is compiled with `-fwasm-exceptions -fPIC`.

The program loads `libgmp.so` and `libgmpxx.so` at run time from the directory
`vitri.js` is served from.

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

Beside the CMake projects, `build.rs` compiles small translation units of its
own: the C ABI shims for CaDiCaL and Arjun, one that reads CaDiCaL's internal
search counters, and, for Emscripten, a `getrusage` the SDK does not provide.
The counters one includes CaDiCaL's internal header, whose struct layouts depend
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
See [`THIRD-PARTY.md`](THIRD-PARTY.md).

## The binary source file

`src/cli_main.rs`, beside the library rather than under `src/bin/`, with an
explicit `[[bin]] path` in `Cargo.toml`. `cargo build` and `cargo install` are
unaffected.
