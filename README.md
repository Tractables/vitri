<p align="center">
  <img src="https://raw.githubusercontent.com/Tractables/vitri/assets/logo/vitri-logo-horizontal.png"
       alt="vitri" width="340">
</p>

<p align="center">
  <a href="https://crates.io/crates/vitri"><img
     src="https://img.shields.io/crates/v/vitri.svg" alt="crates.io"></a>
  <a href="https://pypi.org/project/pyvitri/"><img
     src="https://img.shields.io/pypi/v/pyvitri.svg" alt="PyPI"></a>
  <a href="https://github.com/Tractables/vitri/actions/workflows/ci.yml"><img
     src="https://github.com/Tractables/vitri/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://docs.rs/vitri"><img
     src="https://docs.rs/vitri/badge.svg" alt="docs.rs"></a>
  <a href="https://tractables.github.io/vitri/"><img
     src="https://img.shields.io/badge/run-in%20the%20browser-blue" alt="Run in the browser"></a>
</p>

**Prepare Boolean constraints for counting and circuit compilation.**

Compilers of decision diagrams and circuits, SDDs among them, take a formula
in CNF and a **vtree**: a binary tree over the formula's variables that fixes
the shape of the circuit they build. Their time and memory follow that shape;
on the same formula, one vtree compiles in seconds and another does not
finish. Vitri simplifies the formula first, then builds several vtrees for
what is left and hands the compiler the one that scores best. One Rust
library serves the command line, Rust, Python, C and the browser.

<p align="center">
  <img src="https://raw.githubusercontent.com/Tractables/vitri/main/docs/images/vtree-example.png"
       width="640" alt="A vtree over twelve variables: boxed leaves, circular internal nodes shaded by clause load">
</p>

The vtree `--vtree force` builds for the [tutorial](docs/getting-started.md)'s
twelve-variable formula, drawn from its `--dot` output. A darker node has a
higher clause load; [`docs/vtrees.md`](docs/vtrees.md#drawing-a-vtree) explains
the labels.

## Install

- **Command line:** `cargo install vitri --locked`, after the
  [build prerequisites](docs/building.md#toolchain). Each
  [release](https://github.com/Tractables/vitri/releases) also has an x86-64
  Linux archive of the executable.
- **Rust:** `cargo add vitri`. The
  [crate documentation](https://docs.rs/vitri/latest/vitri/#a-worked-example)
  starts with a worked example.
- **Python:** `pip install pyvitri`, imported as `vitri`. See the
  [Python README](bindings/python/README.md).
- **C and C++:** the x86-64 Linux archive on each release, or a build from
  source. See the [C README](bindings/c/README.md).
- **Browser:** [drop a DIMACS file into the page](https://tractables.github.io/vitri/).
  Nothing is uploaded; the tool runs in the tab.

## Getting started

This prepares the [tutorial](docs/getting-started.md)'s formula, which encodes
the rules of a small scheduling problem in twelve variables:

```sh
curl -fL https://raw.githubusercontent.com/Tractables/vitri/main/docs/example.cnf -o choices.cnf
vitri choices.cnf --out-dir bundle/ --budget-ms 60000
```

Output:

```text
input:        choices.cnf (12 vars, 25 clauses, mode mc)
reduced:      9 vars, 13 clauses  (count(original) = count(reduced) * 2^0)
vtree:        portfolio (9 leaves, 17 nodes)
components:   1 (0 free variables)
wrote:        bundle/reduced.cnf
              bundle/preprocess.json
              bundle/vtree.vtree
              bundle/components.json
elapsed:      60 ms
```

A compiler reads `reduced.cnf` with `vtree.vtree`. Its count of the reduced
formula, times the factor on the `reduced` line, is the count of the input;
`preprocess.json` records that factor. `--budget-ms` bounds the whole run, and
the constructions scale their effort to it. The tutorial compiles the bundle
with PySDD or RSDD and checks the count.

What vitri does:

- **Preprocess for the task.** `--mode` states what preprocessing must
  preserve: model counting, weighted or projected counting, or compilation.
  [`docs/preprocessing.md`](docs/preprocessing.md) lists the modes, the stages
  each one runs and how the record restores the count.
- **Choose a vtree.** A portfolio of constructions builds candidates and ranks
  them on scores read off the formula, without compiling. `--vtree` picks a
  construction and `--dot` draws the result;
  [`docs/vtrees.md`](docs/vtrees.md) covers both and how to bring your own
  decomposition. The [showcase](docs/showcase.md) runs every construction on
  one competition instance and compares their scores.
- **Hand any compiler a standard bundle.** DIMACS and the SDD vtree format,
  split into connected components; [`docs/bundle.md`](docs/bundle.md) gives
  every file and field.

[`docs/env.md`](docs/env.md) lists the optional `VITRI_*` environment
variables, [`docs/sat.md`](docs/sat.md) the SAT solver vitri links and exposes,
and [`docs/building.md`](docs/building.md) the toolchain and the vendored C++
build.

## Applications built on vitri

- [tididi-cnf](https://github.com/Tractables/tididi-cnf) compiles CNF formulas
  into Tree Decision Diagrams for model counting, weighted model counting and
  projected model counting, with vitri's preprocessing and vtrees, also
  [in the browser](https://tractables.github.io/tididi-cnf/).

## Acknowledgements

Vitri builds on the algorithms listed in
[`ACKNOWLEDGEMENTS.md`](docs/ACKNOWLEDGEMENTS.md);
[`THIRD-PARTY.md`](docs/THIRD-PARTY.md) lists the components it vendors and
their licences.

This material is based upon work supported by the Defense Advanced Research
Projects Agency (DARPA) under Agreement No. HR0011269E224.
