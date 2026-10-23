<p align="center">
  <img src="https://raw.githubusercontent.com/Tractables/vitri/assets/logo/vitri-logo.svg"
       alt="vitri" width="300">
</p>

<p align="center">
  <a href="https://github.com/Tractables/vitri/actions/workflows/c.yml"><img
     src="https://github.com/Tractables/vitri/actions/workflows/c.yml/badge.svg" alt="C bindings"></a>
</p>

# vitri from C and C++

**Prepare Boolean constraints for counting and circuit compilation.**

Compilers of decision diagrams and circuits, SDDs and TDDs among them, take a
formula in CNF and a **vtree**: a binary tree over the formula's variables that
fixes the shape of the circuit they build. vitri simplifies the formula, builds
several vtrees for what is left and returns the one that scores best, with the
record that maps a count of the reduced formula back to the original. This
library gives C and C++ programs the same Rust library as the
[command-line tool and crate](../../README.md): DIMACS text and a JSON request
go in, and the files `vitri -o DIR` writes come back in memory, with a JSON
summary of the run.

## Install

Build it from source on x86-64 Linux, after the
[build prerequisites](../../docs/building.md#toolchain):

```sh
git clone https://github.com/Tractables/vitri
cd vitri/bindings/c
make
```

`make` builds `target/release/libvitri_c.so`, `target/release/libvitri_c.a`
and the `vitri` executable the tests compare against. The header is
`include/vitri.h`. Releases after 0.3.0 also carry an x86-64 Linux archive,
`vitri-c-<tag>-x86_64-unknown-linux-gnu.tar.gz`, with the shared library, the
GMP it loads, the header, a pkg-config file and the examples; its README shows
how to build against it.

## Getting started

From `bindings/c`, this builds the example program against the shared library
and prepares the
[tutorial](../../docs/getting-started.md)'s formula for model counting:

```sh
cc -Iinclude examples/prepare.c -Ltarget/release -lvitri_c \
   -Wl,-rpath,"$PWD/target/release" -o prepare
./prepare ../../docs/example.cnf bundle '{"mode": "mc"}' > summary.json
ls bundle
```

Output:

```text
components.json  preprocess.json  reduced.cnf  vtree.vtree
```

The program writes the bundle as `vitri -o bundle` does and prints the run's
summary, the JSON form of
[`Summary`](https://docs.rs/vitri/latest/vitri/request/struct.Summary.html):
its `status`, the input and reduced sizes, and the `lift` factor that turns a
count of `reduced.cnf` into a count of the input. `include/vitri.h` documents
every function, including who owns each buffer, how calls from several threads
behave and what `budget_ms` guarantees. `examples/prepare.cpp` does the same
from C++.

What vitri does:

- **Preprocess for the task.** The request's `mode` states what preprocessing
  must preserve: model counting, weighted or projected counting, or
  compilation. [Preprocessing](../../docs/preprocessing.md) lists the modes,
  the stages each one runs and how the record restores the count.
- **Choose a vtree.** A portfolio of constructions builds candidates and ranks
  them on scores read off the formula, without compiling. `vtree` picks a
  construction and `"dot": true` adds a drawing beside each vtree;
  [Vtree construction](../../docs/vtrees.md) covers the constructions.
- **Hand any compiler a standard bundle.** DIMACS and the SDD vtree format,
  split into connected components; the [bundle reference](../../docs/bundle.md)
  gives every file and field.

## Linking

Against the shared library, nothing else is needed. Link it by name, so the
program records `libvitri_c.so` rather than the path it was built from, and
tell the loader where to find it, as above.

A static link has to name the system libraries the archive uses, which is the
`Libs.private` line of `vitri-c.pc.in`. GMP and MPFR are always linked
dynamically, whichever library you link. From `bindings/c`:

```sh
cc -Iinclude prog.c target/release/libvitri_c.a \
   $(sed -n 's/^Libs.private: //p' vitri-c.pc.in) -o prog
```

The static archive carries the vendored CaDiCaL, CryptoMiniSat and Arjun with
their symbols visible. A program that links its own copy of any of them should
use the shared library.

## Applications built on vitri

- [tididi-cnf](https://github.com/Tractables/tididi-cnf) compiles CNF formulas
  into Tree Decision Diagrams for model counting, weighted model counting and
  projected model counting, with vitri's preprocessing and vtrees, also
  [in the browser](https://tractables.github.io/tididi-cnf/).

## Develop

From `bindings/c`:

```sh
make check      # the examples and tests/test_vitri.c, against each library
make valgrind   # the tests under valgrind
make linkage    # GMP, MPFR and the C++ runtime stay dynamic
```

`make check` compares the bundles the examples and the test driver write with
those the `vitri` executable writes for the same input and flags.

`include/vitri.h` is generated from `src/lib.rs` by
[cbindgen](https://github.com/mozilla/cbindgen) and committed. CI regenerates
it and fails if the committed copy differs, so change `src/lib.rs` and then:

```sh
cargo install cbindgen --version 0.29.0 --locked
make header
```

## Acknowledgements

This material is based upon work supported by the Defense Advanced Research
Projects Agency (DARPA) under Agreement No. HR0011269E224.
