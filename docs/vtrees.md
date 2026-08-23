# Vtrees

A **vtree** (variable tree) is a rooted binary tree whose leaves are the
variables of your formula, one leaf each — a recursive partition of the variable
set, carrying no Boolean content of its own. This package builds several, scores
them against the CNF, and hands you the best by those scores. The emitted file
and the rest of the bundle are in [`bundle.md`](bundle.md).

The trees this library builds are **unordered**: at each internal node the two
children are a partition of that node's variables into two sets, and which one
ends up written or drawn as "left" and which as "right" carries no meaning this
library assigns — nothing here scores, builds toward, or chooses between the
two arrangements. A consumer that needs an ordered vtree is choosing that order
itself; this library does not choose or optimize for one.

## How the portfolio produces candidates

The default `--vtree` spec is a **portfolio**: rather than committing to one
heuristic it walks an ordered catalog, builds a vtree with each construction
that passes its gate, scores every result against the CNF, and selects a winner.

| candidate | how it builds |
|---|---|
| `flowcutter-incidence` | FlowCutter tree decomposition of the **incidence** graph (variables *and* clauses as vertices) |
| `flowcutter-primal` | the same on the **primal** graph (variables only, edges for co-occurrence) |
| `goatd` | this crate's own decomposer — min-fill / min-degree elimination with safe reductions and a refinement pass |
| `hypergraph-bisect` | multilevel **hypergraph bisection**, recursive rather than decomposition-derived |
| `hybrid-flowcutter-incidence` | a FlowCutter decomposition combined under a different vtree-assembly rule |

Every one is also a `--vtree` spec under its own name, and that spec — not the
bare family — is what a bundle publishes as the winner. The bisection candidate
runs at a relaxed imbalance, so it is published as `hypergraph-bisect:0.40`; the
bare name means the balanced default, which is a different tree.

Under a budget it is deadline-truncated: running behind schedule abandons the
rest of the catalog. It also runs per component, each independent component
decomposed on its own and the results grafted into one whole-formula vtree;
`components.json` ([`bundle.md`](bundle.md)) is the split.

## The `--vtree` specs

`portfolio` is the default: it builds several constructions and keeps the
best-scoring one. Every other spec names a single construction, for a caller
who already knows what they want.

Besides the five catalog names above: `goatd-primal`, the primal graph with one
elimination slot and no refinement pass; the single elimination orders
`minfill`, `mindegree` and `nested-dissection`, each one fixed order, unrefined
and unscheduled; `minfill-sample-jw` and `mindegree-sample-jw`, the same with
ties broken by sampling weighted by the SAT-aware Jeroslow-Wang score — **these
two are the orders the portfolio's goatd candidate runs**; `force` and the
`balanced` / `linear` / `reverse-linear` / `random` baselines, both below.

The elimination orders above mirror `elimination_spec_names()`, which is what
`--help` prints and the only place they are written down as code. `--help` also
carries the `-inc` incidence-graph variant of every one of them, and the
optional parameters each spec takes.

### The force-directed embedding

`force` is the one construction here that does not go through a tree
decomposition or a partitioner: it places the variables as points in space and
reads a tree off the geometry. It generalizes FORCE — Aloul, Markov and
Sakallah, "FORCE: a fast and easy-to-implement variable-ordering heuristic",
GLSVLSI 2003 — which embeds variables on a *line* by repeatedly moving each to
the centre of gravity of the clauses it appears in. Here the embedding runs in
several dimensions, and the dimension, the clause weighting, the restart count
and the tree-ifier are all tunable.

### The baselines

Four specs build a tree from the variable numbering alone, consulting no clause:
`balanced`, a balanced binary tree over `1..n`; `linear`, a right-leaning chain,
which is exactly an OBDD variable order; `reverse-linear`, the same chain shape
mirrored; and `random`, a randomly shaped tree over a randomly permuted variable
order — the randomness is fixed and takes no seed, so this is a reproducible
baseline, not a fresh tree per run.

`linear` places variable 1 at the leftmost leaf and variable *n* deepest on the
right, the forward variable order — matching the OBDD order 1..n.
`reverse-linear` is the mirror: the same chain shape with variable *n*
leftmost, the reversed order.

## Reproducibility

No construction here draws on entropy: every generator is seeded from a
constant or from a seed you pass, so the spec string, the CNF and the seed fix
what each stage *attempts*. They do not fix how far it gets. Several stages
read a wall clock whether or not you pass `--budget-ms`, and a machine or a
load that changes their timing can change the tree:

- the **goatd family** — `goatd`, which is the portfolio's own candidate, and
  `goatd-primal` — bounds its elimination with a soft deadline that switches to
  a cheaper fallback and a hard one that bails out to a path decomposition, and
  caps each min-fill slot of its schedule separately;
- the **single elimination orders** (`minfill`, `mindegree`,
  `nested-dissection` and their `-inc` variants) run that same elimination core
  under those same deadlines, falling back to a cheaper order if elimination
  runs long.

On a small formula none of those limits trips and the tree reproduces exactly;
on a large dense one they decide it. `force` and the four baselines above are
deterministic under all of these conditions.

`--budget-ms` pins the budget the run divides up rather than removing those
clocks, and adds one: it puts the portfolio and the timed FlowCutter modes on a
deadline too, so what they finish depends on the machine and how loaded it is.
FlowCutter's step-budgeted spelling (`:<N>steps`) reads no clock at all, but
it is not the timed search stopped early — it searches differently, so the two
spellings are not interchangeable.

None of this makes a whole run reproducible by itself: the preprocessing ahead
of construction is budgeted too, so regenerating a bundle byte for byte means
also turning off whatever preprocessing the mode has — `--no-arjun
--no-simplify` under `mc` and `wmc`, and `--no-simplify` alone under `compile`,
which has no Arjun stage and refuses the flag. A projected mode keeps steps no
flag turns off.
Otherwise treat **the emitted vtree file as the artifact, not a recipe for
regenerating it.**

## The scores

Every candidate is scored on the **realized** vtree against the component's own
CNF. None of these is an estimate read off the tree decomposition the vtree came
from — they are measured on the tree you are handed. **All five are
lower-is-better.**

| score | what it measures |
|---|---|
| `clause_load_stddev` | standard deviation of the per-node *clause load* — the number of clauses whose variables first meet at that node |
| `max_clause_load` | the largest clause load on any single node |
| `peak_context_width_all` | the largest **context width** in the tree. A node's context width is the number of variables its subtree shares with the rest of the formula — those that sit below it yet still appear in a clause reaching above it |
| `peak_context_width_show` | the same, counted over **show** (kept) variables only; `null` for a non-projected instance |
| `cost` | a composite, blending worst-node load, cross-subtree interaction and how deeply clauses are scoped |

`candidate_rank_metric` in `components.json` names which single one of these the
retained set is sorted by, ascending: `clause_load_stddev` for a plain count,
and for a projected one `peak_context_width_show` where there is a show set,
`peak_context_width_all` otherwise. The other four are emitted anyway — they are
there for you to re-rank on.

## Choosing among the candidates

`--candidates N` retains the runners-up instead of dropping them. They are free:
every one was built and scored on the way to picking the winner, and retaining
them does not change the selection. What the retained set means field by field
is in [`bundle.md`](bundle.md).

**"Best" above means best by this crate's own cost model.** If your cost profile
differs, the ranking may not be yours — entry 0 is what this crate's model
picked, so re-rank on the score that matches your bottleneck. If you are memory-bound,
`peak_context_width_all` (or `peak_context_width_show` when projected) speaks to
the widest context, and it is often *not* the metric entry 0 was chosen by. If you
are bound by the largest single node, `max_clause_load`.

## Drawing a vtree

`--dot` writes a Graphviz `.dot` beside every `.vtree` the run emits, with the
same stem. Render one with:

```sh
dot -Tsvg vtree.dot > vtree.svg
```

Leaves are boxes labelled with their 1-based DIMACS variable, internal nodes
circles labelled with their node index. Both are annotated against the CNF that
vtree serves, but not with the same thing. **Fill colour** is on every node:
its clause load normalised by the largest in the tree, light yellow for none
and dark red for the worst node. The **`c=` / `w=`** annotation — that load
written out, then the node's context width — is on the internal nodes only,
since a leaf's width is fixed by its one variable. The width is counted over
the show variables on a projected instance and over all variables otherwise.

The same rendering is available from the library — `vitri::dot` — and its
annotation table is a plain per-node `(colour, label)` map, so a caller can put
its OWN measurements on this picture instead of this crate's.

## Bringing your own decomposer

Most of the catalog above ends in the same place: a tree decomposition of one
graph view of the CNF, converted into a vtree. Both ends of that route are
public, so a decomposer this package does not bundle reaches the same
conversion — over PACE, the treewidth-competition interchange.

The rustdoc on `PaceGraph` carries the round trip in full, as a compiled
example.

Three things are yours to decide. **Which graph**: `GraphKind::Primal` gives
the variables-only view, `GraphKind::Incidence` the one that makes each clause
a vertex too — a decomposition of the latter carries vertex ids above
`num_vars`, which the conversion ignores. **How long the solver runs**:
nothing here launches it, so the budget and the stopping rule are yours, and a
decomposition is usable however early you stop it. **Whether the solution
belongs to the graph you wrote out**: a `.td` for some other graph still parses
and still converts, into a vtree that simply scores badly — compare through
`vitri::score::vtree_cost`, which is the same number this package's own
selection ranks on.

## Searching onward from the vtree you were given

This package builds a vtree, scores it under the metrics above, and stops. If
your own cost model disagrees with those metrics — you know what your compiler
pays for, and this library does not — you can keep searching from the vtree it
handed you rather than starting over.

`vitri::vtree::rotate::rotate_left` and `rotate_right` are the two moves to search
with. Each rewrites one edge in place and leaves the leaf set alone, so every
tree you reach is still a vtree over the same variables; rotating the other way
at the same node undoes the move. That makes the loop the obvious one: rotate,
rescore under your cost, keep or undo. A move returns which nodes it touched,
so per-node state you cache — a score, a width, a compiled fragment — can be
invalidated for those and kept everywhere else.
