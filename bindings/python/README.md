<p align="center">
  <img src="https://raw.githubusercontent.com/Tractables/vitri/assets/logo/vitri-logo.svg"
       alt="vitri" width="300">
</p>

<p align="center">
  <a href="https://pypi.org/project/pyvitri/"><img
     src="https://img.shields.io/pypi/v/pyvitri.svg" alt="PyPI"></a>
  <a href="https://github.com/Tractables/vitri/actions/workflows/python.yml"><img
     src="https://github.com/Tractables/vitri/actions/workflows/python.yml/badge.svg" alt="Python"></a>
  <a href="https://tractables.github.io/vitri/"><img
     src="https://img.shields.io/badge/run-in%20the%20browser-blue" alt="Run in the browser"></a>
  <a href="https://github.com/Tractables/vitri/blob/main/LICENSE"><img
     src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License: Apache-2.0"></a>
</p>

# vitri for Python

**Prepare Boolean constraints for counting and circuit compilation.**

Compilers of decision diagrams and circuits, SDDs among them, take a formula
in CNF and a **vtree**: a binary tree over the formula's variables that fixes
the shape of the circuit they build. vitri simplifies the formula, builds
several vtrees for what is left and returns the one that scores best, with the
record that maps a count of the reduced formula back to the original. This
package runs the same Rust library as the
[command-line tool and crate](https://github.com/Tractables/vitri).

## Install

```sh
pip install pyvitri
```

The package is `pyvitri` on PyPI and imports as `vitri`.

## Use

```python
import vitri

with open("formula.cnf", "rb") as stream:
    result = vitri.prepare(stream.read(), mode="mc", budget_ms=60_000)

result.status            # "built", "fully_resolved" or "refuted"
result.summary["lift"]   # how a count of the reduced formula becomes the input's
result.reduced_cnf       # the formula to compile, as DIMACS text
result.vtree             # the vtree over it in SDD format, or None unless built
result.write("bundle/")  # the files `vitri formula.cnf --out-dir bundle/` writes
```

`help(vitri.prepare)` covers the settings, the errors, calls from threads and
budgets. The [bundle reference](https://github.com/Tractables/vitri/blob/main/docs/bundle.md)
describes the files. [`examples/`](https://github.com/Tractables/vitri/tree/main/bindings/python/examples)
counts models with PySDD over the emitted vtree, and runs a call under a hard
time limit.

## Platforms

PyPI has a wheel for CPython 3.10 and later on x86-64 Linux with glibc 2.34
or newer. It carries GMP 6.3.0 as shared libraries, with its licence texts;
the GMP source is attached to each GitHub release. Anywhere else, pip builds
the package from the source distribution, which needs the toolchain and system
packages listed in
[`building.md`](https://github.com/Tractables/vitri/blob/main/docs/building.md).

## Building from source

The extension compiles the vitri sources in this repository, so it needs the
toolchain and system packages listed in
[`building.md`](https://github.com/Tractables/vitri/blob/main/docs/building.md),
and maturin. PEP 639 resolves `license-files` against this directory and
forbids `..`, so the notices are copied in first:

```sh
.github/scripts/collect-notices.sh bindings/python/notices
pip install maturin
maturin build --release --manifest-path bindings/python/Cargo.toml
```

`maturin develop` installs the extension into the active virtual environment
for `pytest bindings/python/tests`. Give pytest `--vitri-cli PATH` to also
compare the bundles with those of the `vitri` executable.
