<p align="center">
  <img src="https://raw.githubusercontent.com/Tractables/vitri/assets/logo/vitri-logo-horizontal.png"
       alt="vitri" width="340">
</p>

<p align="center">
  <a href="https://github.com/Tractables/vitri/actions/workflows/ci.yml"><img
     src="https://github.com/Tractables/vitri/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://tractables.github.io/vitri/"><img
     src="https://github.com/Tractables/vitri/actions/workflows/docs.yml/badge.svg" alt="Docs"></a>
  <a href="LICENSE"><img
     src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License: Apache-2.0"></a>
  <!-- Once the crate is on crates.io, add:
       <a href="https://crates.io/crates/vitri"><img src="https://img.shields.io/crates/v/vitri.svg" alt="crates.io"></a>
       <a href="https://docs.rs/vitri"><img src="https://docs.rs/vitri/badge.svg" alt="docs.rs"></a> -->
</p>

**CNF preprocessing and vtree (variable tree) construction for circuit compilation and model counting.**

`vitri` is a Rust library with a command-line front end. Hand it a raw DIMACS
CNF and you get back:

- a **reduced CNF** — the formula to compile, renumbered and self-describing;
- **vtrees** over it — one per independent component;
- a **preprocessing record** — the arithmetic that lifts a count over the
  reduced formula back to the original.

Nothing here depends on a particular back end: the output can be used with
d-DNNF, SDD and tree decision diagram (TDD) compilers alike, or with any model
counter that takes a vtree.

## Build

```sh
cargo build --release   # ./target/release/vitri
cargo install --path .  # …or put `vitri` on your PATH, as the examples below assume
```

Prerequisites and the vendored C++ build: [`docs/building.md`](docs/building.md).

## Run

```sh
$ vitri docs/example.cnf --out-dir bundle/ --budget-ms 60000
[simplify] 0 clauses removed, 0 literals shortened, 0 forced vars
[dve-round 1] 0 equiv + 3 dve eliminated, 13 clauses
[dve-total] 3 defined + 0 equiv + 0 free eliminated, 12 → 9 vars, 13 clauses
[portfolio] selected: flowcutter-incidence (metric=stddev, stddev=0.52, cost=41)
input:        docs/example.cnf (12 vars, 25 clauses, mode mc)
reduced:      9 vars, 13 clauses  (count(original) = count(reduced) * 2^0)
vtree:        portfolio (9 leaves, 17 nodes)
components:   1 (0 free variables)
wrote:        bundle/reduced.cnf
              bundle/preprocess.json
              bundle/vtree.vtree
              bundle/components.json
elapsed:      20 ms
```

Compile `bundle/reduced.cnf` under `bundle/vtree.vtree`, get a count *c*, and
multiply by the lift `preprocess.json` states:

```text
count(original) == count(reduced) * 2^count_lift_pow2 * weight_lift
```

Or compile the components separately, each under its own vtree, and multiply
the results.

## A vtree, drawn

`--dot` writes a Graphviz sibling of every `.vtree` a run emits — this one over
[`docs/example.cnf`](docs/example.cnf), twelve variables in three groups of
four:

```sh
vitri docs/example.cnf --out-dir bundle/ --mode compile --vtree force --dot
dot -Tpng -Gbgcolor=white -Gsplines=ortho -Nwidth=0.75 -Gnodesep=0.5 \
    bundle/vtree.dot -o bundle/vtree.png
```

![A vtree over twelve variables: boxed leaves, circular internal nodes filled by clause load](docs/images/vtree-example.png)

Fill colour is a node's clause load; [`docs/vtrees.md`](docs/vtrees.md) covers
the rest.

## The five tasks

`--mode` states what preprocessing must preserve. Without it the mode is read
from the instance's own headers (`c t <track>`, `c p show`, `c p weight`).

| task | `--mode` |
| --- | --- |
| model counting | `mc` |
| weighted model counting | `wmc` |
| projected counting | `pmc` |
| projected weighted counting | `pwmc` |
| compilation (function-preserving) | `compile` |

Which stages each mode permits is in
[`docs/preprocessing.md`](docs/preprocessing.md).

## What comes out

| file | contents |
| --- | --- |
| `reduced.cnf` | the formula to compile, renumbered and self-describing |
| `preprocess.json` | the way back: the lift, the variable map, the forced and free variables |
| `vtree.vtree` | the selected vtree |
| `components.json` | the connected-component split and how the component counts compose |
| `components/`, `candidates/` | one `.cnf` + `.vtree` per component; runner-up vtrees under `--candidates` |

One thing that catches consumers: the show set and the weight table in the
bundle **come out of preprocessing, not out of your input** — read both from
the bundle. [`docs/bundle.md`](docs/bundle.md) is the field-by-field reference.

## Flags and library use

`vitri --help` lists every flag with its default. The binary is a thin shell
over the library: `CnfFormula::from_dimacs` → `vitri::run` →
`VitriRun::write_to_dir`, configured by one `RunConfig` whose `Default` is the
production configuration. The API reference is
[tractables.github.io/vitri](https://tractables.github.io/vitri/).

## Documentation

- [**`docs/bundle.md`**](docs/bundle.md) — the output files, field by field.
- [**`docs/preprocessing.md`**](docs/preprocessing.md) — what each stage
  removes, and how the record gets a correct answer back.
- [**`docs/vtrees.md`**](docs/vtrees.md) — what a vtree is, how the portfolio
  builds and scores candidates, how to bring your own.
- [**`docs/showcase.md`**](docs/showcase.md) — every `--vtree` spec on one CNF.
- [**`docs/env.md`**](docs/env.md) — every `VITRI_*` variable. All optional.
- [**`docs/sat.md`**](docs/sat.md) — the SAT solver vitri links, and why a
  consumer uses it rather than adding one.
- [**`docs/building.md`**](docs/building.md) — toolchain, prerequisites, and the
  vendored C++ build.

## Licence

Apache License 2.0 — see [`LICENSE`](LICENSE). Vendored and linked third-party
components and their licences are listed in
[`THIRD-PARTY.md`](THIRD-PARTY.md); [`ACKNOWLEDGEMENTS.md`](ACKNOWLEDGEMENTS.md)
credits the algorithms this tool is built on.

Contributions are welcome — see [`CONTRIBUTING.md`](CONTRIBUTING.md).
