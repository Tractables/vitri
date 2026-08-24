<p align="center">
  <img src="https://raw.githubusercontent.com/Tractables/vitri/assets/logo/vitri-logo.png"
       alt="vitri" width="260">
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

Nothing here depends on a particular diagram compiler; a d-DNNF, SDD or tree
decision diagram (TDD) compiler consumes the same bundle. The `vitri` binary is
a shell over the public API: every flag that shapes the preprocessing or the
vtree becomes a field of one `RunConfig`, and only where the output goes (`-o`,
`--dot`) is the binary's own.

## Build

```sh
cargo build --release   # ./target/release/vitri
cargo install --path .  # …or put `vitri` on your PATH, as the examples below assume
```

Needs a Rust toolchain no older than `Cargo.toml`'s `rust-version`, CMake,
`pkg-config`, a C++20 compiler (GCC 12+) and
the GMP, MPFR and zlib development packages: the Arjun stack is
vendored, and `build.rs` compiles it (a few minutes, then cached). A missing
prerequisite fails with the install command for it. Details in
[`docs/building.md`](docs/building.md).

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

The progress lines are **stderr** and the summary is **stdout**, so either can
be redirected without losing the other; a larger instance prints a great many
more of the first kind. A formula preprocessing splits also gets a
`bundle/components/` line, one `.cnf` and one `.vtree` per component.

Compile `bundle/reduced.cnf` under `bundle/vtree.vtree`, get a count *c*, and
multiply by the lift `preprocess.json` states — `2^0` above, so here *c* is the
answer already. Or compile the components separately, each under its own vtree,
and multiply the results.

Every mode lifts through one identity, whose two factors are disjoint — an
unweighted mode leaves `weight_lift` at `"1/1"`, a weighted one leaves the
exponent at 0 — so apply both and never branch on the mode:

```text
count(original) == count(reduced) * 2^count_lift_pow2 * weight_lift
```

If preprocessing resolves every variable there is nothing left to compile: the
tool prints the count as a lift and writes no vtree.

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

Fill colour is a node's clause load normalised by the largest in the tree; on
the internal nodes `c=` is that load written out and `w=` is the context width
there. [`docs/vtrees.md`](docs/vtrees.md) covers the rest.

## The five tasks

`--mode` states what preprocessing must preserve. Without it the mode is read
from the instance's own headers — the Model Counting Competition (MCC)
`c t <track>` line, and `c p show` / `c p weight`. The invocation is otherwise
identical; only the permitted stages differ.

| task | `--mode` | stages the mode permits |
| --- | --- | --- |
| model counting | `mc` | this crate's own simplify chain, then Arjun |
| weighted model counting | `wmc` | the same chain, with unequal-weight variables frozen out of DVE and every factor an exact rational |
| projected counting | `pmc` | Arjun's projection-set minimization, show-frozen strengthening, projected BVE — each exactly ×1 for the projected count |
| projected weighted counting | `pwmc` | the `pmc` chain, with the weight table carried through |
| compilation (function-preserving) | `compile` | forced-literal propagation, equivalence-preserving substitution, the equivalence reduction, free-variable removal, and clause simplification that keeps every variable — no gate detection, DVE, Arjun, BVE or SBVA. `preprocess.json` gains an `original_to_reduced_dimacs` map naming every original variable, which is what makes preprocessing undoable |

Stating `--mode` wins over the headers. A declaration the chosen mode does not
use is reported as one `c note:` line on stderr and ignored; a mode that needs
data the file does not carry — a projected mode over a file with no `c p show`
line — is an error naming both sides.
[`docs/preprocessing.md`](docs/preprocessing.md) is the per-stage contract.

## What comes out

| file | contents |
| --- | --- |
| `reduced.cnf` | the formula to compile. Renumbered, and self-describing: its own `c t`, `c p show` and `c p weight` lines, in its own numbering |
| `preprocess.json` | the way back: the lift (`count_lift_pow2`, `weight_lift`), the **signed** reduced-to-original variable map, and the forced and free variables |
| `vtree.vtree` | the selected vtree, standard SDD text format |
| `components.json` | the connected-component split: one file set per component, local↔reduced maps, and how the component counts compose |
| `components/`, `candidates/` | one `.cnf` + `.vtree` per component; runner-up vtrees, with their scores, under `--candidates` |

Every variable id in the bundle is 1-based DIMACS, in the numbering of the
`.cnf` it sits beside. [`docs/bundle.md`](docs/bundle.md) is the field-by-field
reference.

Two things that catch consumers: the show set and the weight table in the bundle
**come out of preprocessing, not out of your input** — Arjun rewrites and
renumbers the one, equivalence folding rewrites the other. Read both from the
bundle, never from your input file.
[`docs/preprocessing.md`](docs/preprocessing.md) states both in full.

## Flags

`vitri --help` is the inventory: every flag, the value it takes, its default,
and what it does — with the vocabularies of `--mode`, `--components` and
`--vtree` interpolated from the tables the parser itself matches against, so it
offers exactly what it accepts. The options select what preprocessing must
preserve, which vtree construction runs, what the run may spend, what is written
beside the bundle, and which preprocessing stages are skipped. A stage flag the
resolved mode has no stage for is refused rather than ignored.

[`docs/vtrees.md`](docs/vtrees.md) has the vtree vocabulary in full and
[`docs/preprocessing.md`](docs/preprocessing.md) what each stage does.
Every `VITRI_*` environment variable is listed in
[`docs/env.md`](docs/env.md); none is required, and unset is the production
configuration.

## Library use

Every flag except `-o` and `--dot` — which name output destinations
rather than behaviour, and belong to the binary — is a field of one `RunConfig`,
whose `Default` is the production configuration.
The [crate documentation](https://docs.rs/vitri)
opens with that flow worked end to end — CNF in, bundle out — including the one
outcome that has no vtree and is not an error: preprocessing resolved every
variable, so the lift is the whole answer. A refutation is not that outcome — a
bundle whose `record.unsat` is set carries a synthetic contradiction and a vtree
over it, and that field is what says the count is 0. Its three calls
(`vitri::CnfFormula::from_dimacs`, `vitri::run`, `vitri::VitriRun::write_to_dir`)
and the types they need (`RunConfig`, `SelectionCtx`, `ComponentWriteOptions`,
`RunVtree`) all sit at the crate root, so that example names no module.

The library's own diagnostics are off until you ask for them with
`vitri::diagnostics::set_verbose(true)`, and it does not exit the process:
every failure it reports comes back as an `Err`. The one exception is an
allocation failure inside the vendored C++, which aborts. Every fallible entry
point of the crate — the parser, the preprocessing, the construction,
`RunConfig::validate` and the writers — returns `vitri::VitriError`, the
crate's own type, and a failed read or write is a `VitriError::Io` naming the
path. That example is written against `Box<dyn Error>` only because opening the
input file is `std::io`'s failure, not the crate's.

Only the binary maps a failure to an exit status, and `vitri --help` states
which status means what. The API reference is
[docs.rs/vitri](https://docs.rs/vitri).

## Documentation

- [**`docs/bundle.md`**](docs/bundle.md) — the output files, field by field.
  The on-disk contract.
- [**`docs/preprocessing.md`**](docs/preprocessing.md) — what each stage
  removes, and what the consumer does with the record to get a correct answer
  back.
- [**`docs/vtrees.md`**](docs/vtrees.md) — what a vtree is, how the portfolio
  builds and scores candidates, how to choose one for a cost model that is not
  this crate's.
- [**`docs/showcase.md`**](docs/showcase.md) — every `--vtree` spec on one CNF.
- [**`docs/env.md`**](docs/env.md) — every `VITRI_*` variable, what it tunes, and
  its default. All optional.
- [**`docs/building.md`**](docs/building.md) — toolchain, prerequisites, and the
  vendored C++ build.

## Limits

- **A `pmc` / `pwmc` bundle preserves variable ids**, so its `p cnf` header can
  keep the input's variable count even when the chain eliminated a great deal.
  Judge that chain by clause count and by which variables still occur, not by
  the header.
- **`--candidates N` applies to every component.** There is no way to ask for
  the runners-up of one component only.

## Licence

`vitri` is licensed under the **Apache License 2.0** — see [`LICENSE`](LICENSE).

Third-party components that ship inside the crate or are linked into a build of
it are listed in [**`THIRD-PARTY.md`**](THIRD-PARTY.md) with the licence and
copyright of each:
the vendored FlowCutter tree is BSD-2-Clause and MIT, and the vendored Arjun
stack is MIT apart from Eigen (MPL-2.0, bundled inside SBVA, no relinking
obligation) and a handful of files under BSD-2-Clause, BSD-3-Clause, zlib,
BSL-1.0 or `MIT OR Apache-2.0`;
GMP and MPFR (LGPL) are linked dynamically.
[**`ACKNOWLEDGEMENTS.md`**](ACKNOWLEDGEMENTS.md) credits the algorithms this tool
is built on and the work that introduced them.

Contributions are welcome — [**`CONTRIBUTING.md`**](CONTRIBUTING.md) has the
gate set, the behaviour rules, and where tests live.
