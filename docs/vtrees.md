# Vtrees

A **vtree** (variable tree) is a rooted binary tree whose leaves are the
variables of the formula, one leaf each — a recursive partition of the variable
set, carrying no Boolean content of its own. This package builds several, scores
them against the CNF, and returns the best by those scores. The emitted file and
the rest of the bundle are in [`bundle.md`](bundle.md).

The trees this library builds are **unordered**: at each internal node the two
children are a partition of that node's variables into two sets, and which one
is written or drawn as "left" and which as "right" carries no meaning. Nothing
here scores, builds toward, or chooses between the two arrangements. A consumer
that needs an ordered vtree chooses that order itself.

## The portfolio

The default `--vtree` spec is a **portfolio**. It walks an ordered catalog,
builds a vtree with each construction that passes its gate, scores every result
against the CNF, and selects a winner.

| candidate | how it builds |
|---|---|
| `flowcutter-incidence` | FlowCutter tree decomposition of the **incidence** graph (variables *and* clauses as vertices) |
| `flowcutter-primal` | the same on the **primal** graph (variables only, edges for co-occurrence) |
| `goatd-incidence` | goatd's min-fill / min-degree schedule with safe reductions and a refinement pass |
| `hypergraph-bisect` | multilevel **hypergraph bisection**, recursive rather than decomposition-derived |
| `guided-bisect` | recursive bisection of the primal graph, with the incidence decomposition offered at every level |

The decomposition-derived portfolio candidates leave `place` open by default,
so their conversion searches both placements in the order described below.
Naming `place=shallow` or `place=deep` fixes that choice for every such
candidate. Standalone decomposition specs follow the same rule.

Every candidate is also a `--vtree` spec under its own name, and that spec, not
the bare family, is what a bundle publishes as the winner. The bisection
candidate runs at a relaxed imbalance and is published as
`hypergraph-bisect:imbalance=0.40`; the bare name means the balanced default,
which is a different tree.

Under a budget the catalog is deadline-truncated: a run behind schedule abandons
the rest of it. It also runs per component, each independent component
decomposed on its own and the results grafted into one whole-formula vtree.
`components.json` ([`bundle.md`](bundle.md)) records the split. A library
caller can read which of those happened: `VtreeBuild::limits` lists the builds
that finished, the builds the budget cut short, the time they spent and the
candidates never started.

`VtreeBuild::construction_ms` reports the broader end-to-end construction wall
from the library entry through the finished whole or grafted tree. It includes
setup, simple constructors and component grafting that are deliberately outside
`limits.spent_ms`.

## From a tree decomposition to a vtree

A tree decomposition is a tree of bags, each bag a set of the graph's vertices,
each vertex occurring in a connected set of bags. It does not name a vtree by
itself: it has to be rooted, every variable has to be given one of the bags
holding it, and each bag's children and leaves have to be binarized into one
subtree. Those three choices are a **reading** of the decomposition, and one
decomposition has many.

A conversion is a **search over readings**. Every reading it reaches is built
and scored by `cost` (*The scores* below), and the cheapest tree is returned.
The three `--vtree` keys below each name one dimension of the reading: a key
that is written fixes that dimension, and a dimension left out is one the search
walks. Writing all three searches exactly one reading.

The search is ordered, so a truncated one is predictable. It screens every
candidate root under one `place`/`binarize` pair — `shallow` with `edge`, or with
`balanced` when there is no CNF to read — then gives the three cheapest-screening
roots the remaining pairs, `place` `shallow` then `deep`, each over `binarize`
`edge`, `hypergraph`, `balanced`. `--budget-ms` cuts the search short between
readings, never before the first has finished, so a bounded conversion always
returns a tree.

Every conversion reports on stderr the reading it kept, what that reading
scored, and how many readings it got through out of how many it planned. A leaf
rooting reports the bag it settled on, as `root=leaf#<bag>`, since `leaf` names
a set of them. `VITRI_CONVERSION_TRACE` ([`env.md`](env.md)) adds a line per
reading.

On the incidence view a bag holds clause vertices as well as variables. Those
get no leaves — the conversion reads only the vertices below the variable count
— but they still sit in the bag tree: they count toward the depth the placement
rule measures, and a bag holding nothing else still groups its children.

**Where the decomposition is rooted** (`root`). A decomposition is unrooted; the
conversion needs a root because it builds each bag's subtree out of its
children's, leaves upward. `first` takes the bag the decomposition was written
with first, `centroid` the bag that minimises the largest part left when it is
removed, and `leaf` the best of the decomposition's degree-1 bags — one value
naming a set of bags rather than one, so writing it still leaves the search a
choice among them. Rooting is per connected component: a decomposition that is
a forest gets a root each, and the component subtrees are combined at the top of
the vtree, together with a leaf for every variable no bag mentions.

**Which bag each variable is placed in** (`place`). A variable occurs in a
connected set of bags and gets exactly one leaf, so one of those bags is its
home and the rest hold it only as a bag vertex. `deep` picks the bag furthest
from the root, `shallow` the closest. Deep placement lets each clause's
variables meet as far from the root as the decomposition allows, which is what
carries the decomposition's width over to the tree. Given the CNF, `deep`
breaks a tie between equally deep bags toward the one holding more of the
variable's clause partners.

**How a bag is binarized** (`binarize`). A bag arrives with its children's subtrees
already built and one leaf per variable placed there, and has to binarize that
list into a single subtree.

| `binarize` | the subtree it builds |
|---|---|
| `balanced` | children then leaves, the list halved recursively into a balanced subtree |
| `edge` | children bisected along the decomposition's own edges, to share as few of this bag's variables as possible; a leaf goes to the side that uses it, rises above the cut when both sides do, and follows its clause partners when neither does |
| `hypergraph` | the items bisected under the multilevel partitioner so that as few clauses as possible span both halves, clauses as hyperedges, recursively |

`edge` and `hypergraph` read the CNF, and a conversion handed none binarizes as
`balanced` whatever was written. `edge` is written for `place=shallow`: under
`deep` a shared variable already sits inside one branch, so nothing rises above
a cut and what is left is edge-aligned children plus leaf routing.

Without the CNF there is nothing to score a reading against, so a conversion
handed no formula builds exactly one reading whatever was left open. That is
what `td_to_vtree` does; `td_to_vtree_reading` is the same conversion with the
formula, the reading and the deadline passed in.

**`guided-bisect`** is a construction rather than a reading. It bisects the
formula's primal graph recursively, and at each level also projects the
decomposition onto that level's variables, converts the projection, scores both
against the clauses that stay inside the level and keeps the cheaper, so the
decomposition can override the bisection level by level instead of fixing the
whole shape. Below a small subset it stops bisecting and builds from a local
elimination order. Its per-level conversions are the same search, but the shape
of the whole tree is not one reading of one decomposition, so it takes none of
the three keys.

## The `--vtree` specs

`portfolio` is the default: it builds several constructions and keeps the
best-scoring one. Every other spec names a single construction.

The single elimination orders build from one order, unrefined and unscheduled.
`minfill` and `mindegree` can break ties by sampling weighted by the SAT-aware
Jeroslow-Wang score (`ties=jw-sample`), and those two sampled orders are what
the portfolio's goatd candidate runs.

### The grammar

```text
spec   := base [ ":" params ]
params := key "=" value { "," key "=" value }
```

A parameter is always written with its key, and each key at most once.

Every base, with the parameters it takes:

| base | builds | parameters |
|---|---|---|
| `portfolio` | the catalog above, best-scoring candidate wins | — |
| `flowcutter-primal` | FlowCutter decomposition of the primal graph | `budget` `iters` `patience` `root` `place` `binarize` |
| `flowcutter-incidence` | the same on the incidence graph | as `flowcutter-primal` |
| `goatd-primal` | scheduled elimination with safe reductions and a refinement pass, primal graph | `seed` `refine` `root` `place` `binarize` |
| `goatd-incidence` | the same on the incidence graph | `seed` `refine` `root` `place` `binarize` |
| `guided-bisect` | recursive primal bisection guided by an incidence decomposition | `budget` `iters` `patience` |
| `hypergraph-bisect` | multilevel bisection of the clause hypergraph | `imbalance` |
| `primal-bisect` | the same multilevel core on the primal graph | `imbalance` |
| `minfill-primal`, `minfill-incidence` | min-fill elimination order | `seed` `ties` `root` `place` `binarize` |
| `mindegree-primal`, `mindegree-incidence` | min-degree elimination order | `seed` `ties` `root` `place` `binarize` |
| `nested-dissection-primal`, `nested-dissection-incidence` | nested-dissection order | `seed` `root` `place` `binarize` |
| `force` | force-directed embedding, tree-ified | `treeify` `root` `orient` `weights` `feedback` `clause-weight` `dim` `restarts` `init` |
| `balanced`, `linear`, `reverse-linear`, `random` | the variable numbering alone | — |

Every family that decomposes a graph view of the CNF names the view it runs on;
the rest carry no view. `nested-dissection` breaks ties deterministically only,
so it takes no `ties`. An elimination order is one decomposition and the
FlowCutter and goatd families produce one too, so all of them take the same
three conversion keys.

Every parameter, with what it changes:

| key | values | default | changes |
|---|---|---|---|
| `seed` | an integer | `0` | which random tie-break the elimination takes |
| `ties` | `fixed`, `jw-sample` | `fixed` | how the elimination breaks a tie between two candidate variables |
| `refine` | `on`, `off` | `on` | whether the goatd schedule ends in the refinement pass, or runs one unrefined elimination slot |
| `imbalance` | a fraction in `0.0..=0.5` | `0.03` | how far either side may deviate from an even split |
| `budget` | `<N>ms` or `<N>steps` | `200ms` | how hard FlowCutter looks for a decomposition |
| `iters` | an integer | `100000` timed, `900` step-budgeted | how many FlowCutter iterations the search runs |
| `patience` | milliseconds | `100` with no `budget` written, `150` with one | how long the timed search waits for an improvement |
| `root` | `first`, `centroid`, `leaf` | `searched` | which bag the decomposition is rooted at |
| `place` | `shallow`, `deep` | `searched` | which bag of the decomposition each variable is placed in |
| `binarize` | `edge`, `hypergraph`, `balanced` | `searched` | how each bag's children and variable leaves are binarized |
| `treeify` | `mst`, `cut` | `mst` | which tree-ifier turns the embedding into a vtree |
| `root` | `merge`, `balance`, `hybrid` | `merge` | where the MST is rooted |
| `orient` | `x`, `small`, `big` | `x` | how an MST edge becomes a left/right child pair |
| `weights` | `euclid`, `co` | `euclid` | what an MST edge weighs |
| `feedback` | an integer `0..=8` | `0` | how many feedback rounds reshape the layout |
| `clause-weight` | `uniform`, `short` | `uniform` | how strongly a clause pulls its variables together |
| `dim` | an integer `2..=8` | `2` | how many dimensions the variables are embedded in |
| `restarts` | an integer `1..=16` | `1` | how many layouts are tried, keeping the best |
| `init` | `rand`, `force1d` | `rand` | how the layout starts |

`root`, `place` and `binarize` are the three dimensions of a reading, described
under *From a tree decomposition to a vtree*: the rows above give the spelling,
that section gives the behaviour. `force` has a `root` of its own, and `orient`,
`weights` and `feedback` beside it, which reshape the MST; those four go with
`treeify=mst`.

`--help` prints this same table, and both are rendered from the one table in the
source that the parser matches against.

### The force-directed embedding

`force` is the one construction here that does not go through a tree
decomposition or a partitioner: it places the variables as points in space and
reads a tree off the geometry. It generalizes FORCE — Aloul, Markov and
Sakallah, "FORCE: a fast and easy-to-implement variable-ordering heuristic",
GLSVLSI 2003 — which embeds variables on a *line* by repeatedly moving each to
the centre of gravity of the clauses it appears in. Here the embedding runs in
several dimensions, and the parameters above tune the layout and the tree-ifier.
A caller that wants the coordinates and not a tree — to cluster on them, or to
branch on them — asks `decompose::embed` for the same layout this construction
starts from.

### The baselines

Four specs build a tree from the variable numbering alone, consulting no clause:
`balanced`, a balanced binary tree over `1..n`; `linear`, a right-leaning chain,
which is exactly an OBDD variable order; `reverse-linear`, the same chain shape
mirrored; and `random`, a randomly shaped tree over a randomly permuted variable
order. The randomness is fixed and takes no seed, so `random` is a reproducible
baseline, not a fresh tree per run.

`linear` places variable 1 at the leftmost leaf and variable *n* deepest on the
right, the forward variable order, matching the OBDD order 1..n.
`reverse-linear` is the mirror: the same chain shape with variable *n*
leftmost, the reversed order.

## Budget semantics

Construction spends a share of the run's one budget rather than a budget of its
own. `RunConfig::construction_budget` says which share — a third of what is left
by default, all of it, or up to a named instant — and its variants document what
each is for, including the double division a caller that has already carved its
own construction window has to avoid.

## Reproducibility

No construction here draws on entropy: every generator is seeded from a
constant or from a seed passed in, so the spec string, the CNF and the seed fix
what each stage *attempts*. They do not fix how far it gets. Several stages
read a wall clock with or without `--budget-ms`, and a machine or a load that
changes their timing can change the tree:

- the unrefined **goatd family** uses a one-second soft portfolio deadline and
  a two-second hard deadline; the refined schedule uses the construction budget
  or `VITRI_GOATD_REFINE_BUDGET_MS`, and is unbounded when neither exists;
- the **single elimination orders** use a ten-second soft deadline and a
  twenty-second hard deadline, switching to a cheaper order and then completing
  the residual as a path when those limits are reached.

On a small formula none of those limits trips and the tree reproduces exactly;
on a large dense one they decide it. `force` and the four baselines above are
deterministic under all of these conditions.

`--budget-ms` pins the budget the run divides up rather than removing those
clocks, and adds one: it puts the portfolio and the timed FlowCutter modes on a
deadline too, so what they finish depends on the machine and how loaded it is.
Under a wall-clock deadline the portfolio also remembers what its last build in
the process cost, and a build entered with less room than that runs in its capped
mode, so a tree can depend on what the same process built before it.
FlowCutter's step-budgeted spelling (`budget=<N>steps`) reads no clock at all,
but it is not the timed search stopped early: it searches differently, so the
two spellings are not interchangeable.

A conversion adds no clock of its own beyond `--budget-ms`. Naming all three
conversion keys therefore pins the tree a given decomposition is read into, up
to the choice `root=leaf` leaves open, and that inner search over the leaf bags
is itself deterministic when it is given the time to finish.

None of this makes a whole run reproducible by itself: the preprocessing ahead
of construction is budgeted too, so regenerating a bundle byte for byte means
also turning off whatever preprocessing the mode has — `--no-arjun
--no-simplify` under `mc` and `wmc`, and `--no-simplify` alone under `compile`,
which has no Arjun stage and refuses the flag. A projected mode keeps steps no
flag turns off. Otherwise the emitted vtree file is the artifact, not a recipe
for regenerating it, unless construction runs under the budget below.

### Deterministic construction

`ConstructionBudget::Deterministic` bounds construction by the work it does
rather than by the clock, so two runs over the same formula at the same budget
select the same vtree on any machine and under any load. The budget is in work
units — `ConstructionBudget::for_wall_ms` converts one from a wall in
milliseconds at a calibrated rate — and it costs a few percent more construction
wall than the same build under a wall-clock budget of the same size, because
charges are deliberately pessimistic. The rustdoc on `ConstructionBudget` has
the rest: what a unit is, and what the mode does and does not bound.

## The scores

Every candidate is scored on the **realized** vtree against the component's own
CNF. None of these is an estimate read off the tree decomposition the vtree came
from; they are measured on the tree that is returned. All five are
lower-is-better.

| score | what it measures |
|---|---|
| `clause_load_stddev` | standard deviation of the per-node *clause load* — the number of clauses whose variables first meet at that node |
| `max_clause_load` | the largest clause load on any single node |
| `peak_context_width_all` | the largest **context width** in the tree. A node's context width is the number of variables its subtree shares with the rest of the formula — those that sit below it yet still appear in a clause reaching above it |
| `peak_context_width_show` | the same, counted over **show** (kept) variables only; `null` for a non-projected instance |
| `cost` | the combined structural cost returned by [`vitri::score::vtree_cost`](../src/score/mod.rs) |

`candidate_rank_metric` in `components.json` names which single one of these the
retained set is sorted by, ascending: `cost` for a plain count,
and for a projected one `peak_context_width_show` where there is a show set,
`peak_context_width_all` otherwise. The other four are emitted anyway, for
re-ranking.

## Choosing among the candidates

`--candidates N` retains the runners-up instead of dropping them. They are free:
every one was built and scored on the way to picking the winner, and retaining
them does not change the selection. What the retained set means field by field
is in [`bundle.md`](bundle.md).

"Best" above means best by this crate's own cost model, and entry 0 is what that
model picked. A caller whose cost profile differs re-ranks on the score that
matches its bottleneck: `peak_context_width_all` (or `peak_context_width_show`
when projected) for the widest context, which is often *not* the metric entry 0
was chosen by, and `max_clause_load` for the largest single node.

**Steering it.** A caller retrying a piece it compiled badly wants a different
tree from the same portfolio rather than a different construction:
`PortfolioKnobs::prefer` names a candidate — softly, or as a requirement that
fails the build — and changes nothing else about how the portfolio runs.

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

The same rendering is available from the library, `vitri::dot`. Its annotation
table is a plain per-node `(colour, label)` map, so a caller can put its own
measurements on this picture instead of this crate's.

## Structure measurements

Two measurements this crate takes for its own decisions are public and
documented on the items themselves: `decompose::conditioned_primal_width_ub`
bounds the width left in the primal graph once a set of variables is
conditioned away, and `score::StructureProfile::measure` reports the clause-width
and occurrence dispersion that structure-sensitive policies consult. An
embedding that selects a vtree for a transformed formula may set
`SelectionCtx::source_profile` from `StructureProfile::from_coefficients` (or
from `measure` on the source formula). Portfolio selection then keeps the
transformed formula's occurrence dispersion authoritative while accepting the
source formula's clause-width dispersion as an additional width signal. Leaving
the field unset preserves selection from the formula being built alone. This
field applies to the construction-only API. The full pipeline has the raw input
in hand, so `vitri::frontend` measures that formula when its `FrontendSession`
is created, returns the measurement from `FrontendSession::prepare` as
`VitriRun::source_profile`, and replaces any caller-supplied profile before
vtree selection. `vitri::run` creates and immediately prepares the same session;
it is not a separate pipeline.

## Your own decomposition

`PaceGraph::to_gr` and `PaceGraph::parse_td` connect any PACE-format solver to
`td_to_vtree` and validate the returned decomposition against the exported
graph; the `PaceGraph` rustdoc contains the complete round trip.

## Local search from a vtree

This package builds a vtree, scores it under the metrics above, and stops. A
caller whose own cost model disagrees with those metrics can keep searching from
the vtree it was handed rather than starting over.

`vitri::vtree::rotate::rotate_left` and `rotate_right` are the two moves. Each
rewrites one edge in place and leaves the leaf set alone, so every tree reached
is still a vtree over the same variables; rotating the other way at the same
node undoes the move. The loop is rotate, rescore under the caller's cost, keep
or undo. A move returns which nodes it touched, so per-node state — a score, a
width, a compiled fragment — can be invalidated for those and kept everywhere
else.
