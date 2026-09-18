# The output bundle

What `vitri instance.cnf --out-dir bundle/` writes.
`vitri::request::prepare` returns the same files in memory.

| file | what it holds |
| --- | --- |
| `reduced.cnf` | the formula to compile: standard DIMACS, renumbered, and self-describing — its own `c t` line (none under `compile`), `c p show` and `c p weight` lines, in its own numbering |
| `preprocess.json` | `bundle::PreprocessRecord`: how to lift a result on `reduced.cnf` back to the original |
| `vtree.vtree` | the vtree over `reduced.cnf`, standard SDD text format. `--dot` writes a Graphviz sibling beside every `.vtree` |
| `components.json` | `bundle::components::ComponentsManifest`: the connected components of `reduced.cnf`, each with its files and its local-to-reduced variable map |
| `components/compNNN.{cnf,vtree}` | one component, in its own local numbering |
| `candidates/compNNN.rankRR.vtree` | under `--candidates N`, the runners-up the portfolio already built and scored |

Both JSON files deserialize as well as serialize, so a Rust consumer reads a
bundle back into those two structs instead of redeclaring them; every field,
its numbering and its `format` tag are documented there.

## The lift

```text
count(original) == count(reduced) × 2^count_lift_pow2 × weight_lift
```

[`preprocessing.md`](preprocessing.md) has what each mode does and how a
consumer applies the record.

A Rust caller also gets what the file does not carry, because the file
describes the lift rather than the call: `PreprocessBundle::stages`,
`::count_lift`, `::telemetry`, `::decision_trace`, `::arjun_input`,
`::independent_support_reduced` and `::learnt_clauses_reduced_dimacs`. The
whole vtree construction wall is `VtreeBuild::construction_ms`.

## Composing components

`bundle::components` states the three numbering spaces (local, reduced,
original) and how the component counts compose, with or without weights and
with or without a show set. `components.json` is written for a connected
formula too, so read it unconditionally. [`vtrees.md`](vtrees.md) covers the
scores and choosing among candidates.

## When there is no vtree

Preprocessing can settle the instance on its own; `bundle::RunVtree` names the
two ways, and the bundle is then `reduced.cnf` and `preprocess.json` alone.

## Writing a formula back out

A formula goes back out through `CnfFormula::write_dimacs`, or
`write_dimacs_clauses` for the clause body alone.
