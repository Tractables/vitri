# vitri from C and C++

A C ABI over the vitri library: DIMACS text and a JSON request go in, and the
files `vitri -o DIR` writes come back in memory, with a JSON summary of the
run. The request takes the same settings as the command line's flags.

This is a separate crate with its own `Cargo.toml`. It builds the vitri
sources beside it; `vitri_version()` reports their version.

## Building

```sh
cd bindings/c
make
```

This runs `cargo` for both libraries and the `vitri` executable the tests
compare against. The first build compiles the vendored C++ stack;
[docs/building.md](../../docs/building.md) lists the prerequisites. The
libraries land in `bindings/c/target/release`: `libvitri_c.so` and
`libvitri_c.a`. The header is `bindings/c/include/vitri.h`.

The Makefile and the CI workflow cover Linux x86_64.

## Linking

Against the shared library, nothing else is needed. Link it by name, so the
program records `libvitri_c.so` rather than the path it was built from, and
tell the loader where to find it:

```sh
cc -Ibindings/c/include prog.c -Lbindings/c/target/release -lvitri_c \
   -Wl,-rpath,"$PWD/bindings/c/target/release" -o prog
```

A static link has to name the system libraries the archive uses, which is the
`Libs.private` line of `vitri-c.pc.in`. GMP and MPFR are always linked
dynamically, whichever library you link.

```sh
cc -Ibindings/c/include prog.c bindings/c/target/release/libvitri_c.a \
   $(sed -n 's/^Libs.private: //p' bindings/c/vitri-c.pc.in) -o prog
```

That file is a pkg-config template; fill in its two placeholders when
installing, taking the version from `bindings/c/Cargo.toml`:

```sh
version=$(sed -n 's/^version = "\(.*\)"/\1/p' bindings/c/Cargo.toml | head -1)
sed -e 's|@PREFIX@|/usr/local|' -e "s|@VERSION@|$version|" \
    bindings/c/vitri-c.pc.in > /usr/local/lib/pkgconfig/vitri-c.pc
```

The static archive carries the vendored CaDiCaL, CryptoMiniSat and Arjun with
their symbols visible. A program that links its own copy of any of them should
use the shared library.

## Using the API

`include/vitri.h` documents every function, including who owns each buffer,
how calls from several threads behave and what `budget_ms` guarantees.
`examples/prepare.c` and `examples/prepare.cpp` read a CNF, prepare it, write
the bundle into a directory and print the summary.

## Tests

```sh
make check      # the examples and tests/test_vitri.c, against each library
make valgrind   # the tests under valgrind
make linkage    # GMP, MPFR and the C++ runtime stay dynamic
```

`make check` compares the examples' output and the test driver's bundles with
what the `vitri` executable writes for the same input and flags.

## Regenerating the header

`include/vitri.h` is generated from `src/lib.rs` by
[cbindgen](https://github.com/mozilla/cbindgen) and committed. CI regenerates
it and fails if the committed copy differs, so change `src/lib.rs` and then:

```sh
cargo install cbindgen --version 0.29.0 --locked
make header
```
