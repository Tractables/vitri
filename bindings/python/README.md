<p align="center">
  <img src="https://raw.githubusercontent.com/Tractables/vitri/assets/logo/vitri-logo.svg"
       alt="vitri" width="300">
</p>

<p align="center">
  <a href="https://pypi.org/project/pyvitri/"><img
     src="https://img.shields.io/pypi/v/pyvitri.svg" alt="PyPI"></a>
  <a href="https://github.com/Tractables/vitri/actions/workflows/python.yml"><img
     src="https://github.com/Tractables/vitri/actions/workflows/python.yml/badge.svg" alt="Python"></a>
</p>

# vitri for Python

**Prepare Boolean constraints for counting and circuit compilation.**

Compilers of decision diagrams and circuits, SDDs and TDDs among them, take a
formula in CNF and a **vtree**: a binary tree over the formula's variables that
fixes the shape of the circuit they build. vitri simplifies the formula, builds
several vtrees for what is left and returns the one that scores best, with the
record that maps a count of the reduced formula back to the original. This
package runs the same Rust library as the
[command-line tool and crate](https://github.com/Tractables/vitri).

## Install

```sh
pip install pyvitri
```

The package is `pyvitri` on PyPI and imports as `vitri`.

## Getting started

This prepares the
[tutorial](https://github.com/Tractables/vitri/blob/main/docs/getting-started.md)'s
formula, saved as
[`choices.cnf`](https://raw.githubusercontent.com/Tractables/vitri/main/docs/example.cnf),
for model counting:

```python
import vitri

with open("choices.cnf", "rb") as stream:
    result = vitri.prepare(stream.read(), mode="mc", budget_ms=60_000)

summary = result.summary
print(summary["status"])
print(summary["input"], "->", summary["reduced"])
print(summary["lift"]["factor"])
result.write("bundle/")
```

Output:

```text
built
{'variables': 12, 'clauses': 25} -> {'variables': 9, 'clauses': 13}
2^0
```

A compiler reads `result.reduced_cnf` with `result.vtree`, both as text. Its
count of the reduced formula, times the lift factor, is the count of the input.
The status is `fully_resolved` or `refuted` instead when preprocessing settles
the formula by itself; there is then no vtree. `result.write` saves the files
the `vitri` command writes.

`help(vitri.prepare)` covers the settings, the errors, calls from threads and
budgets. [`examples/`](https://github.com/Tractables/vitri/tree/main/bindings/python/examples)
counts models with PySDD over the emitted vtree, and runs a call under a hard
time limit.

What vitri does:

- **Preprocess for the task.** `mode` states what preprocessing must preserve:
  model counting, weighted or projected counting, or compilation.
  [Preprocessing](https://github.com/Tractables/vitri/blob/main/docs/preprocessing.md)
  lists the modes, the stages each one runs and how the record restores the
  count.
- **Choose a vtree.** A portfolio of constructions builds candidates and ranks
  them on scores read off the formula, without compiling.
  [Vtree construction](https://github.com/Tractables/vitri/blob/main/docs/vtrees.md)
  covers the constructions and how to bring your own decomposition.
- **Hand any compiler a standard bundle.** DIMACS and the SDD vtree format,
  split into connected components; the
  [bundle reference](https://github.com/Tractables/vitri/blob/main/docs/bundle.md)
  gives every file and field.

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

## Applications built on vitri

- [tididi-cnf](https://github.com/Tractables/tididi-cnf) compiles CNF formulas
  into Tree Decision Diagrams for model counting, weighted model counting and
  projected model counting, with vitri's preprocessing and vtrees, also
  [in the browser](https://tractables.github.io/tididi-cnf/).

## Acknowledgements

This material is based upon work supported by the Defense Advanced Research
Projects Agency (DARPA) under Agreement No. HR0011269E224.
