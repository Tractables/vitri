# The output bundle

What `vitri instance.cnf --out-dir bundle/` writes.
[`preprocessing.md`](preprocessing.md) has the preprocessing semantics,
[`vtrees.md`](vtrees.md) the scores. The complete
field lists are the serialized structs — `bundle::PreprocessRecord` and
`bundle::components::ComponentsManifest` — so this page covers only what their
names do not already say, plus the traps. Both deserialize as well as serialize,
so a Rust consumer reads a bundle back into them instead of redeclaring it.

Every variable id and literal is 1-based DIMACS. Fields with nothing to report
are omitted rather than written empty.

## `reduced.cnf`

The formula to compile: standard DIMACS, renumbered, and self-describing — it
carries its own `c t`, `c p show` and `c p weight` lines in its own numbering.
No `c t` line under `--mode compile`, which is no competition track.

## `preprocess.json`

How to lift a result on `reduced.cnf` back to the original.

| field | what the name does not say |
| --- | --- |
| `count_lift_pow2` | cardinality half of the lift; `0` under a weighted mode |
| `weight_lift` | weighted half, exact `"num/den"` in lowest terms; `"1/1"` under an unweighted mode |
| `reduced_to_original_dimacs` | **signed**: `+o` means reduced `r` is original `o`, `-o` means its negation, `null` a variable the reduction introduced |
| `original_to_reduced_dimacs` | the inverse, and **`compile` only** — `±r`, `true`/`false` for a variable proved constant, `null` for unconstrained; fully resolved through equivalences |
| `reduced_weights` | every literal listed explicitly, in REDUCED numbering |

Traps:

- Forced literals contribute ×1 and are not in `count_lift_pow2`; free variables
  already are (or in `weight_lift`). Applying them twice double-counts.
- The show set and the weights come out of preprocessing, not out of the input.
  A show variable may have been dropped or swapped for an equivalent, and a
  weight folded into a survivor's. Under `compile` alone they are the input's,
  renumbered.
- Sign matters for models, not counts. A variable preprocessing *determined*
  appears in no field at all, so a lifted assignment is partial until it is
  propagated.
- On `unsat`, `reduced.cnf` holds an explicit contradiction (`x` and `¬x`),
  because DIMACS cannot portably spell the empty clause.

A Rust caller also gets what this file does not carry, because it describes the
call rather than the lift: `PreprocessBundle::stages`, `::count_lift` and
`::arjun_input`.

## `vtree.vtree`

Standard SDD text format, leaf labels the variables of `reduced.cnf`. `--dot`
writes a Graphviz sibling beside every `.vtree`.

Several construction stages read a wall clock with or without `--budget-ms`
(see [`vtrees.md`](vtrees.md#reproducibility)), so the same CNF on a different
machine, under a different load, or under a different budget can give a
different vtree: the emitted file is the reliable artifact, not a recipe for
regenerating it.

## `components.json`

`reduced.cnf` splits into connected components, each with its own vtree.
`vtree.vtree` holds them whole, joined under internal nodes whose sides share no
variables, but nothing marks which nodes are the joins, so the vtree alone does
not say where the split was. This manifest does, and it is written even for a
connected formula (one entry, identity map, pointing at the top-level files), so
read it unconditionally.

| field | what the name does not say |
| --- | --- |
| `format` | `vitri-components-v1`; refuse a tag you do not know, rather than reading the fields you recognise |
| `free_vars_reduced_dimacs` | REDUCED space — a different set from `preprocess.json`'s original-space one |
| `candidate_rank_metric` | which score sorts each candidate set after the first, ascending |
| `components[]` | emission order; position `N` is the `NNN` in that component's file names |
| `components[].local_to_reduced_dimacs` | LOCAL → REDUCED, strictly increasing, injective across components |
| `components[].cnf`, `.vtree` | `components/compNNN.{cnf,vtree}` — or the top-level files when there is one component |
| `components[].selection` | which construction produced that component's vtree, and the shape of the decomposition behind it |

`selection.winning_spec` is the `--vtree` spec that rebuilds that vtree, and
under `portfolio` it is the candidate that won, not `portfolio` — the vtree file
cannot say which one did, and a component too small for the portfolio reports
`minfill-primal`. It is the spec exactly as the parser took it, every parameter
written on it included (`hypergraph-bisect:imbalance=0.40`), so feeding it back
as `--vtree` rebuilds that construction over that component alone; it does not
reproduce the whole-formula run, whose other components chose separately.
`built_by` under `vtree_candidates` spells its candidates the same way.

`selection.tree_decomposition` is a summary — bag count and width, on the graph
projection the construction ran on, so `treewidth` can reach or exceed the
component's variable count on the incidence graph. `treewidth` is the width of
the decomposition this run found, its largest bag less one, hence an upper bound
on the projected graph's treewidth rather than that treewidth itself.
Constructions that decompose nothing (`force`, `hypergraph-bisect`, the simple
vtrees) omit it, as does one that recombined several decompositions.

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

## `candidates/`

`--candidates N` writes out the runners-up the portfolio already built and
scored, as `candidates/compNNN.rankRR.vtree` in that component's local space.
Selection is untouched — `vtree.vtree` is the same either way. Choosing among
them is covered in [`vtrees.md`](vtrees.md).

Array order is the rank; there is no `rank` field. Scores are computed on the
**realized** vtree against that component's own CNF, all lower-is-better.

- Entry 0 is always the selected vtree, pinned rather than sorted into place,
  and points at the component's own `vtree` rather than a copy.
- Duplicates are collapsed: two constructions converging on the same vtree give
  one entry naming both in `built_by`, so asking for 4 can yield 3.
- The count is capped at `candidates::MAX_CANDIDATES`, and a larger value is an
  error rather than a truncation.
- Candidates exist only where a portfolio ran. A component built directly has no
  `vtree_candidates`, and `--candidates N > 1` under a single-vtree spec
  (`minfill-primal`, `balanced`, …) is rejected.

## Writing a formula back out

`CnfFormula::write_dimacs` is the inverse of `CnfFormula::from_dimacs` and
`write_dimacs_clauses` writes the clause body alone, for a caller assembling a
file with a preamble of its own.
