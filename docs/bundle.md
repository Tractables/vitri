# The output bundle

What `vitri instance.cnf --out-dir bundle/` writes.
`vitri::request::prepare` returns the same files in memory.

| file | what it holds |
| --- | --- |
| `reduced.cnf` | the formula to compile: standard DIMACS and self-describing — its own `c t` line (none under `compile`), `c p show` and `c p weight` lines, in its own numbering |
| `preprocess.json` | `bundle::PreprocessRecord`: how to lift a result on `reduced.cnf` back to the original |
| `vtree.vtree` | the vtree over `reduced.cnf`, standard SDD text format |
| `components.json` | `bundle::components::ComponentsManifest`: the connected components of `reduced.cnf`, each with its files and its local-to-reduced variable map |
| `components/compNNN.{cnf,vtree}` | one component, in its own local numbering; absent when the formula is connected |
| `candidates/compNNN.rankRR.vtree` | under `--candidates N`, the runners-up the portfolio already built and scored |
| `*.dot` | under `--dot`, a Graphviz drawing beside each `.vtree` |

Both JSON files deserialize as well as serialize, so a Rust consumer reads a
bundle back into those two structs instead of redeclaring them; every field,
its numbering and its `format` tag are documented there.

## The lift

[`preprocessing.md`](preprocessing.md#the-lift-identity) gives the lift and how
each mode fills it.

A Rust caller also gets what the file does not carry: `bundle::PreprocessBundle`
holds what each stage did, measurements of the run, and the intermediate
formula and clause sets a caller can ask to keep.
`component::VtreeBuild::construction_ms` is the vtree construction time.

## Composing components

`bundle::components` states the three numbering spaces (local, reduced,
original) and how the component counts compose, with or without weights and
with or without a show set. `components.json` is written with every vtree,
including for a connected formula. [`vtrees.md`](vtrees.md) covers the scores
and choosing among candidates.

## When there is no vtree

Preprocessing can settle the instance on its own; `bundle::RunVtree` names the
two ways, and the bundle is then `reduced.cnf` and `preprocess.json` alone.

## Writing a formula back out

A formula goes back out through `CnfFormula::write_dimacs`, or
`write_dimacs_clauses` for the clause body alone.
