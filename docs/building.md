# Building

```sh
cargo build --release
```

Cargo builds the C++ sources carried by vitri and goatd. No build step downloads
source or writes outside Cargo's build directory.

## Toolchain

- **Rust**, no older than the `rust-version` in `Cargo.toml`.
- **A C++20 compiler.** GCC 12 or newer — Arjun uses `constexpr std::vector`
  copies, which GCC 11 (still the default on Ubuntu 22.04) cannot compile. The
  build script tries `g++-14`, `g++-13`, `g++-12`, then `g++`. `VITRI_CXX`
  overrides the choice; the C compiler is the same name with `g++` replaced by
  `gcc`, or `clang++` by `clang`. The goatd dependency compiles its FlowCutter
  backend with the same search and its own override,
  [`GOATD_CXX`](https://github.com/Tractables/goatd/blob/main/docs/building.md),
  so set both.
- **CMake**, to build the Arjun stack.
- **`pkg-config`**, which that stack's CMake projects use to locate GMP and
  MPFR.
- **GMP, MPFR and zlib development packages.**

```sh
sudo apt install build-essential gcc-12 g++-12 cmake pkg-config libgmp-dev libmpfr-dev zlib1g-dev   # Debian/Ubuntu
sudo dnf install gcc-c++ cmake pkgconf-pkg-config gmp-devel mpfr-devel zlib-devel                   # Fedora/RHEL
```

vitri builds on Linux only: merging the vendored archives needs the MRI script
mode of GNU `ar`, which Apple's `ar` does not have, and Windows is not
supported. `pip install pyvitri` and the
[release archives](https://github.com/Tractables/vitri/releases) are prebuilt
for x86-64 Linux.

To build against a GMP installed under another prefix, set `PKG_CONFIG_PATH`,
`CPATH` and `LIBRARY_PATH` to its `lib/pkgconfig`, `include` and `lib`
directories; the binary then needs that `lib` directory on its library search
path at run time.

The first build takes a few minutes because it compiles Arjun, CryptoMiniSat,
CaDiCaL, cadiback and SBVA. Cargo caches the result; later builds do not repeat
it. `cargo build -j N` also limits how many C++ compilations run at once.

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

## Build-time variables

Build scripts read these, so a change takes effect on the next build. The
run-time variables are in [`env.md`](env.md).

| variable | what it does |
|---|---|
| `VITRI_CXX` | the C++ compiler for the vendored stack, as above; refused by a build for Emscripten, which compiles with the SDK's `em++` |
| `GOATD_CXX` | the C++ compiler for goatd's FlowCutter backend |
| `VITRI_EMSCRIPTEN_PREFIX` | the prefix holding GMP and MPFR built for Emscripten, with GMP's side modules; required by a build for Emscripten and refused by a native build |
| `AR` | the archiver that merges the vendored archives into the one this crate links, `ar` by default; it must accept GNU `ar -M` scripts. A build for Emscripten uses the SDK's `emar` |
| `DOCS_RS` | set by docs.rs; the build script skips the C++ build, so `cargo doc` works without CMake or GMP, but the build cannot link a binary |

## The vendored Arjun stack

`vendor/arjun/upstream/` holds five third-party CMake projects — Arjun,
CryptoMiniSat, CaDiCaL, cadiback and SBVA — pinned at exact commits, with the
modifications `PROVENANCE.md` records already applied to the source here.
[`vendor/arjun/upstream/PROVENANCE.md`](../vendor/arjun/upstream/PROVENANCE.md)
records the commits, the licences, every modification and what was trimmed.

## Running the binary elsewhere

The vendored C++, vitri's Arjun stack and goatd's FlowCutter, is linked
statically. GMP (with `libgmpxx`), MPFR, zlib and the C++ runtime are linked
dynamically; `ldd` on the binary lists the libraries the machine that runs it
needs. See [`THIRD-PARTY.md`](THIRD-PARTY.md).
