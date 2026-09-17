# The output bundle

What `vitri instance.cnf --out-dir bundle/` writes.
`vitri::request::prepare` returns the same files in memory.

| file | what it holds |
| --- | --- |
| `reduced.cnf` | the formula to compile: standard DIMACS, renumbered, and self-describing. It carries its own `c t`, `c p show` and `c p weight` lines in its own numbering |
| `preprocess.json` | `bundle::PreprocessRecord`: how to lift a result on `reduced.cnf` back to the original |
| `vtree.vtree` | the vtree over `reduced.cnf`, standard SDD text format. `--dot` writes a Graphviz sibling beside every `.vtree` |
| `components.json` | `bundle::components::ComponentsManifest`: the connected components of `reduced.cnf`, each with its files and its local-to-reduced variable map |
| `components/compNNN.{cnf,vtree}` | one component, in its own local numbering |
| `candidates/compNNN.rankRR.vtree` | under `--candidates N`, the runners-up the portfolio already built and scored |

Every variable id and literal is 1-based DIMACS. Fields with nothing to report
are omitted rather than written empty. Both JSON files carry a `format` tag
naming the shape they are written in, and reading one refuses a tag this
version does not know. Both JSON files deserialize as well as
serialize, so a Rust consumer reads a bundle back into those two structs instead
of redeclaring them, and each field's rustdoc states what its name does not: the
variable space it is in, what preprocessing did to it, and what a consumer must
not re-derive from the input.

## The lift

```text
count(original) == count(reduced) × 2^count_lift_pow2 × weight_lift
```

Both factors apply under every mode: an unweighted mode leaves `weight_lift` at
`"1/1"`, a weighted one leaves `count_lift_pow2` at `0`.
[`preprocessing.md`](preprocessing.md) has the preprocessing semantics.

A Rust caller also gets what the file does not carry, because the file describes
the lift rather than the call: `PreprocessBundle::stages`, `::count_lift`,
`::arjun_input`, `::telemetry` and `::independent_support_reduced`. The whole
vtree construction wall is `VtreeBuild::construction_ms`.

## Composing components

There are three numbering spaces, not two. Local is neither reduced nor
original: a local id reaches original space through `local_to_reduced_dimacs`
and then `reduced_to_original_dimacs`.

Components are variable- and clause-disjoint, so:

```text
count(reduced)      = 2^|free_vars_reduced_dimacs| * Π_c count(compNNN.cnf)
count_proj(reduced) = 2^|free_vars_reduced_dimacs ∩ show| * Π_c count_proj(compNNN.cnf)
```

A projected-out free variable contributes ×1, not ×2 — hence the intersection.
Under weights it is no power of two at all: each free variable contributes
`(w⁻ + w⁺)`, a projected-out one 1.

`components.json` is written even for a connected formula (one entry, identity
map, pointing at the top-level files), so read it unconditionally.
[`vtrees.md`](vtrees.md) covers the scores and choosing among candidates.

## When there is no vtree

Preprocessing can settle the instance on its own: every variable resolved, so
the count is the lift, or the instance refuted, so the count is 0. Neither
leaves anything to compile, and the bundle is `reduced.cnf` and
`preprocess.json` alone.

Several construction stages read a wall clock with or without `--budget-ms`
(see [`vtrees.md`](vtrees.md#reproducibility)), so the same CNF on a different
machine, under a different load, or under a different budget can give a
different vtree: the emitted file is the reliable artifact, not a recipe for
regenerating it.

## Writing a formula back out

`CnfFormula::write_dimacs` is the inverse of `CnfFormula::from_dimacs` and
`write_dimacs_clauses` writes the clause body alone, for a caller assembling a
file with a preamble of its own.
