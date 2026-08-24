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
| `goatd-incidence` | this crate's own decomposer — min-fill / min-degree elimination with safe reductions and a refinement pass |
| `hypergraph-bisect` | multilevel **hypergraph bisection**, recursive rather than decomposition-derived |
| `flowcutter-incidence:assembly=hybrid` | the same decomposition, combined under a different vtree-assembly rule |

Every one is also a `--vtree` spec under its own name, and that spec — not the
bare family — is what a bundle publishes as the winner. The bisection candidate
runs at a relaxed imbalance, so it is published as `hypergraph-bisect:imbalance=0.40`; the
bare name means the balanced default, which is a different tree.

Under a budget it is deadline-truncated: running behind schedule abandons the
rest of the catalog. It also runs per component, each independent component
decomposed on its own and the results grafted into one whole-formula vtree;
`components.json` ([`bundle.md`](bundle.md)) is the split.

## From a tree decomposition to a vtree

A tree decomposition is a tree of bags, each bag a set of the graph's vertices,
each vertex occurring in a connected set of bags. Reading a vtree off one is a
sequence of five decisions, and the `--vtree` parameters below name them one for
one.

On the incidence view a bag holds clause vertices as well as variables. Those
get no leaves — the conversion reads only the vertices below the variable count
— but they still sit in the bag tree: they count toward the depth the
placement rule measures, and a bag holding nothing else still groups its
children.

**Where the decomposition is rooted** (`td-root`). A decomposition is unrooted;
the conversion needs a root because it builds each bag's subtree out of its
children's, leaves upward. `first-bag` takes the bag the decomposition was
written with first, `centroid` the bag that minimises the largest part left
when it is removed. Rooting is per connected component — a decomposition that
is a forest gets a root each — and the component subtrees are combined at the
top of the vtree, together with a leaf for every variable no bag mentions.

**Which bag each variable is placed in** (`assign`). A variable occurs in a
connected set of bags and gets exactly one leaf, so one of those bags is its
home and the rest hold it only as a bag vertex. `deep` picks the bag furthest
from the root, `shallow` the closest. Deep placement lets each clause's
variables meet as far from the root as the decomposition allows, which is what
carries the decomposition's width over to the tree. Given the CNF, `deep`
breaks a tie between equally deep bags toward the one holding more of the
variable's clause partners.

**How the variables inside one bag are ordered** (`var-order`). Their order
decides which of them are grouped together when the bag is assembled.
`natural` is by variable number; `affinity` chains them greedily by clause
co-occurrence from the most connected one outward, so variables sharing clauses
sit adjacent.

**How a bag is assembled** (`order`). A bag arrives with its children's
subtrees already built and one leaf per variable placed there, and has to fold
that list into a single binary subtree.

| `order` | the subtree it builds |
|---|---|
| `children-first` | children then leaves, the list halved recursively into a balanced subtree |
| `vars-first` | the same, leaves before children |
| `children-by-size`, `largest-first` | children sorted by subtree leaf count, ascending and descending, then leaves |
| `left-deep` | a chain instead of a balanced split: one item per level, the first nearest the root |
| `clause-split` | the items bisected greedily so that as few clauses as possible span both halves, recursively |
| `hypergraph-bisect` | the same objective under the multilevel partitioner, clauses as hyperedges |
| `boundary-adjacent` | the leaves that also appear in the parent bag split off as one subtree, beside everything else |
| `td-edge` | children bisected to share as few of this bag's variables as possible; a leaf goes to the side that uses it, rises above the cut when both sides do, and follows its clause partners when neither does |

The clause-aware readings — `affinity` and the last four orders — need the CNF,
and a conversion handed none falls back to the balanced split. `td-edge` is
built for `assign=shallow`: under `deep` a shared variable already sits inside
one branch, so nothing rises above a cut and what is left is edge-aligned
children plus leaf routing.

**Whether several readings are scored** (`best`). One decomposition can be read
many ways, and `best` reads it repeatedly — the first bag, the centroid and the
decomposition's leaf bags as roots, each under several arrangements — scores
every finished tree by `cost` (*The scores* below) and keeps the lowest. It
varies the root and the arrangement only: placement stays `deep` and within-bag
order `natural`. Past a handful of roots it screens rather than sweeps, reading
every root one way and spending the remaining arrangements on the few that
scored best. `--budget-ms` cuts the sweep short between conversions, never
before the first, so a bounded run still returns a converted tree.

`auto` reads the whole formula's variable count — so every component of one
formula is built the same way — and is on below the size `--help` prints in its
default column, off above it. It is also off for a spec that wrote any of
`assign`, `td-root`, `var-order` or `order`: presence decides, not value, so
`assign=deep` turns the sweep off just as `assign=shallow` does. It is also off
under `assembly=hybrid` and under a FlowCutter `budget` given in steps.

Two cases the parameter table does not show. The goatd family scores several
readings whatever `best` says; the key is accepted there so that one setting
can be applied across a batch of specs. And `flowcutter-incidence`, with the sweep off and no
conversion parameter written, still reads its decomposition twice —
`children-first` and `children-by-size` — and keeps the cheaper; a spec that
named a conversion parameter gets exactly the one reading it named.

**`assembly=hybrid`** replaces the walk over bags altogether. It bisects the
formula's primal graph recursively, and at each level also projects the
decomposition onto that level's variables, converts the projection, scores both
against the clauses that stay inside the level and keeps the cheaper, so the
decomposition can override the bisection level by level instead of fixing the
whole shape. Below a small subset it stops bisecting and builds from a local
elimination order. None of the five decisions
above applies to it, and it takes none of their keys.

## The `--vtree` specs

`portfolio` is the default: it builds several constructions and keeps the
best-scoring one. Every other spec names a single construction, for a caller
who already knows what they want.

The single elimination orders build from ONE order, unrefined and unscheduled.
`minfill` and `mindegree` can break ties by sampling weighted by the SAT-aware
Jeroslow-Wang score (`ties=jw-sample`), and **those two sampled orders are what
the portfolio's goatd candidate runs**.

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
| `flowcutter-primal` | FlowCutter decomposition of the primal graph | `budget` `iters` `patience` `assign` `td-root` `var-order` `order` `best` `assembly` |
| `flowcutter-incidence` | the same on the incidence graph | as `flowcutter-primal` |
| `goatd-primal` | scheduled elimination with safe reductions and a refinement pass, primal graph | `seed` `best` `refine` |
| `goatd-incidence` | the same on the incidence graph | `seed` `best` `refine` |
| `hypergraph-bisect` | multilevel bisection of the clause hypergraph | `imbalance` |
| `primal-bisect` | the same multilevel core on the primal graph | `imbalance` |
| `minfill-primal`, `minfill-incidence` | min-fill elimination order | `seed` `ties` `assign` `td-root` `var-order` `order` `best` |
| `mindegree-primal`, `mindegree-incidence` | min-degree elimination order | `seed` `ties` `assign` `td-root` `var-order` `order` `best` |
| `nested-dissection-primal`, `nested-dissection-incidence` | nested-dissection order | `seed` `assign` `td-root` `var-order` `order` `best` |
| `force` | force-directed embedding, tree-ified | `treeify` `root` `orient` `weights` `feedback` `clause-weight` `dim` `restarts` `init` |
| `balanced`, `linear`, `reverse-linear`, `random` | the variable numbering alone | — |

Every family that decomposes a graph view of the CNF names the view it runs on;
the rest carry no view. `nested-dissection` breaks ties deterministically only,
so it takes no `ties`. An elimination order is one decomposition and the
FlowCutter families produce one too, so all of them take the same conversion
keys and the same `best`.

Every parameter, with what it changes:

| key | values | default | changes |
|---|---|---|---|
| `seed` | an integer | `0` | which random tie-break the elimination takes |
| `ties` | `fixed`, `jw-sample` | `fixed` | how the elimination breaks a tie between two candidate variables |
| `refine` | `on`, `off` | `on` | whether the goatd schedule ends in the refinement pass, or runs one unrefined elimination slot |
| `assembly` | `convert`, `hybrid` | `convert` | how the vtree is assembled from the decomposition |
| `imbalance` | a fraction in `0.0..=1.0` | `0.03` | how uneven the two sides of a partition may be |
| `budget` | `<N>ms` or `<N>steps` | `200ms` | how hard FlowCutter looks for a decomposition |
| `iters` | an integer | `100000` timed, `900` step-budgeted | how many FlowCutter iterations the search runs |
| `patience` | milliseconds | `100` with no `budget` written, `150` with one | how long the timed search waits for an improvement |
| `assign` | `deep`, `shallow` | `deep` | which bag each variable is placed in |
| `td-root` | `first-bag`, `centroid` | `first-bag` | which bag the decomposition is rooted at |
| `var-order` | `natural`, `affinity` | `natural` | how the variables inside one bag are ordered |
| `order` | `children-first`, `vars-first`, `children-by-size`, `clause-split`, `left-deep`, `largest-first`, `hypergraph-bisect`, `boundary-adjacent`, `td-edge` | `children-first` | how children and variable leaves are arranged at each bag |
| `best` | `auto`, `on`, `off` | `auto` | whether several readings of the decomposition are scored and the best kept |
| `treeify` | `mst`, `cut` | `mst` | which tree-ifier turns the embedding into a vtree |
| `root` | `merge`, `balance`, `hybrid` | `merge` | where the MST is rooted |
| `orient` | `x`, `small`, `big` | `x` | how an MST edge becomes a left/right child pair |
| `weights` | `euclid`, `co` | `euclid` | what an MST edge weighs |
| `feedback` | an integer `0..=8` | `0` | how many feedback rounds reshape the layout |
| `clause-weight` | `uniform`, `short` | `uniform` | how strongly a clause pulls its variables together |
| `dim` | an integer `2..=8` | `2` | how many dimensions the variables are embedded in |
| `restarts` | an integer `1..=16` | `1` | how many layouts are tried, keeping the best |
| `init` | `rand`, `force1d` | `rand` | how the layout starts |

`assign`, `td-root`, `var-order`, `order`, `best` and `assembly` are the
conversion decisions *From a tree decomposition to a vtree* describes: the rows
above are how they are spelled, that section is what they do. `root`, `orient`,
`weights` and `feedback` reshape the MST, so they go with `treeify=mst`.

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

## Budget semantics

Construction is one phase of a run, and it spends a share of one budget rather
than a budget of its own. Five rules, in the order they apply.

**One deadline for the run.** `RunConfig::deadline` is the instant the whole run
must stop by. `budget_ms` is the same thing counted from the first clock read.
An explicit `deadline` wins; `budget_ms` still supplies the scale that internal
sub-budgets are derived from when both are set.

**The clock is read once.** `run` anchors the deadline at the start and hands the
anchored configuration to every phase. Preprocessing and construction divide one
budget: what preprocessing spends, construction does not get.

**`construction_budget` decides how construction divides what is left.**
`Share`, the default, takes a third of the remaining wall, clamped to between
90 s and 900 s. `WholeRemaining` takes all of it. `Until(t)` takes until `t`. All
three are clamped by the run deadline, so none of them can outlive the run, and
a run with no deadline leaves construction unbounded under all three.
`RunConfig::construction_deadline` returns the instant that resolves to — the
same value construction enforces, so a caller sizing its own later phases reads
it rather than recomputing it.

**Every bound here is soft.** The portfolio consults the deadline between
candidates and FlowCutter checks it between restart iterations and before each
of its two greedy pre-passes, so whatever is in flight when the bound passes runs
to completion. A bound decides what is *started*, not what is interrupted. The
first multilevel partition of a build that holds no decomposition yet runs
unbounded either way: returning nothing is worse than returning late.

**A caller with its own slicing policy sets `WholeRemaining`.** Passing an
already-carved construction window as `deadline` while leaving `Share` on divides
it a second time — construction gets a ninth of the run rather than a third, and
nothing reports that it happened.

## Reproducibility

No construction here draws on entropy: every generator is seeded from a
constant or from a seed you pass, so the spec string, the CNF and the seed fix
what each stage *attempts*. They do not fix how far it gets. Several stages
read a wall clock whether or not you pass `--budget-ms`, and a machine or a
load that changes their timing can change the tree:

- the **goatd family** — `goatd-incidence`, which is the portfolio's own
  candidate, and `goatd-primal` — bounds its elimination with a soft deadline
  that switches to a cheaper fallback and a hard one that bails out to a path
  decomposition, and caps each min-fill slot of its schedule separately;
- the **single elimination orders** run that same elimination core under those
  same deadlines, falling back to a cheaper order if elimination runs long.

On a small formula none of those limits trips and the tree reproduces exactly;
on a large dense one they decide it. `force` and the four baselines above are
deterministic under all of these conditions.

`--budget-ms` pins the budget the run divides up rather than removing those
clocks, and adds one: it puts the portfolio and the timed FlowCutter modes on a
deadline too, so what they finish depends on the machine and how loaded it is.
Under a deadline the portfolio also remembers what its last build in the process
cost, and a build entered with less room than that runs in its capped mode — so
a tree can depend on what the same process built before it.
FlowCutter's step-budgeted spelling (`budget=<N>steps`) reads no clock at all, but
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
