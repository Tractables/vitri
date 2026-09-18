<p align="center">
  <img src="https://raw.githubusercontent.com/Tractables/vitri/assets/logo/vitri-logo-horizontal.png"
       alt="vitri" width="340">
</p>

<p align="center">
  <a href="https://tractables.github.io/vitri/"><img
     src="https://img.shields.io/badge/run-in%20the%20browser-blue" alt="Run in the browser"></a>
  <a href="https://github.com/Tractables/vitri/actions/workflows/ci.yml"><img
     src="https://github.com/Tractables/vitri/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://tractables.github.io/vitri/vitri/"><img
     src="https://github.com/Tractables/vitri/actions/workflows/docs.yml/badge.svg" alt="Docs"></a>
  <a href="LICENSE"><img
     src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License: Apache-2.0"></a>
  <a href="https://crates.io/crates/vitri"><img
     src="https://img.shields.io/crates/v/vitri.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vitri"><img
     src="https://docs.rs/vitri/badge.svg" alt="docs.rs"></a>
</p>

**Prepare Boolean constraints for counting and circuit compilation.**

Compilers of decision diagrams and circuits, SDDs among them, take a formula
in CNF and a **vtree**: a binary tree over the formula's variables that fixes
the shape of the circuit they build. Their time and memory follow that shape;
on the same formula, one vtree compiles in seconds and another does not
finish. Vitri simplifies the formula first, then builds several vtrees for
what is left and hands the compiler the one that scores best. It is a Rust
library and a command-line tool.

The pipeline is: a DIMACS `.cnf` in; a reduced `.cnf`, its `.vtree` and the
record that maps the compiler's count back to the original out; the compiler
of your choice after that. The [showcase](docs/showcase.md) runs every
construction on one competition instance, before and after preprocessing, and
puts their scores side by side.

## Run it

**Command line.** Install the native [build prerequisites](docs/building.md#toolchain),
then:

```sh
cargo install vitri --locked
vitri instance.cnf --out-dir bundle/ --budget-ms 60000
```

`--budget-ms` is the wall clock the whole run may spend, and the constructions
scale their effort to it; without it the run is unbounded and each construction
keeps its default effort. The
[tutorial](docs/getting-started.md) supplies an input file and takes the
bundle through PySDD or RSDD to a checked count. A release also carries an
x86-64 Linux archive of the executable with the GMP it links, for a machine
without a Rust toolchain.

**Browser.** [Drop a DIMACS file into the page](https://tractables.github.io/vitri/)
and see what preprocessing removed, the vtree, and the scores it was chosen
on. Nothing is uploaded; the tool runs in the tab.

**Library.** Three calls — parse, `run`, write — are the API; the
[crate documentation](https://docs.rs/vitri/latest/vitri/#a-worked-example)
starts with a worked example. The same library is reachable from
[Python](bindings/python), [C and C++](bindings/c) and
[the browser](bindings/wasm).

## Modes

`--mode` states what preprocessing must preserve. Without it the mode is read
from the instance's headers (`c t <track>`, `c p show`, `c p weight`).

| task | `--mode` |
| --- | --- |
| model counting | `mc` |
| weighted model counting | `wmc` |
| projected counting | `pmc` |
| projected weighted counting | `pwmc` |
| compilation (function-preserving) | `compile` |

[`docs/preprocessing.md`](docs/preprocessing.md) lists the stages each mode
permits and what each removes.

## Output

| file | contents |
| --- | --- |
| `reduced.cnf` | the formula to compile, renumbered and self-describing |
| `preprocess.json` | the lift, the variable map, the forced and free variables |
| `vtree.vtree` | the selected vtree |
| `components.json` | the connected-component split and how the component counts compose |
| `components/`, `candidates/` | one `.cnf` + `.vtree` per component; runner-up vtrees under `--candidates` |

The show set and the weight table in the bundle come from preprocessing, not
from the input; read both from the bundle. [`docs/bundle.md`](docs/bundle.md)
documents every field.

## Read on

- [`docs/getting-started.md`](docs/getting-started.md) — count a small
  configuration problem end to end.
- [`docs/vtrees.md`](docs/vtrees.md) — the constructions, the portfolio and
  its scores, drawing a vtree, bringing your own decomposition.
- [`docs/showcase.md`](docs/showcase.md) — every `--vtree` spec on one CNF.
- [`docs/preprocessing.md`](docs/preprocessing.md) — the stages, and how the
  record restores the count.
- [`docs/bundle.md`](docs/bundle.md) — the output files, field by field.
- [`docs/env.md`](docs/env.md) — the `VITRI_*` environment variables, all
  optional.
- [`docs/sat.md`](docs/sat.md) — the SAT solver vitri links and exposes.
- [`docs/building.md`](docs/building.md) — toolchain, prerequisites, the
  vendored C++ build.

## Licence

Apache License 2.0 ([`LICENSE`](LICENSE)). Third-party components and their
licences: [`THIRD-PARTY.md`](docs/THIRD-PARTY.md). The algorithms this tool
builds on: [`ACKNOWLEDGEMENTS.md`](docs/ACKNOWLEDGEMENTS.md). Contributing:
[`CONTRIBUTING.md`](docs/CONTRIBUTING.md).
