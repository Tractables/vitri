# Showcase

Every construction family and every parameter axis of `--vtree`, one spec
each, on one CNF before and after [`--mode mc`](preprocessing.md)
preprocessing. The full value sets are in [`vtrees.md`](vtrees.md).

## The instance

`mc2023_track1_008`, Track 1 of the 2023 Model Counting Competition.

| | |
| --- | ---: |
| variables | 6,856 |
| clauses | 27,626 |
| models | 171,798,691,840 |

The primal graph is one component of 6,683 variables plus 173 variables that
occur only in unit clauses.

## Scores

All lower-is-better, measured on the emitted vtree against its formula.
The tables abbreviate them to stddev, max load, peak ctx, cost and tw.

| score | definition |
| --- | --- |
| `clause_load_stddev` | standard deviation over internal nodes of the *clause load*: the number of clauses whose variables first meet at that node |
| `max_clause_load` | the largest clause load on any node |
| `peak_context_width_all` | the largest *context width* on any node: variables below the node that also occur in a clause reaching above it |
| `cost` | a composite of worst-node load, cross-subtree interaction and clause scoping depth |
| `treewidth` | not a score: the width of the tree decomposition the vtree was built from — its widest bag less one — where there is one |

## Raw formula

Preprocessing off: `--no-simplify --no-arjun --components whole`.

| `--vtree` spec | stddev | max load | peak ctx | cost | tw | wall |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| **Baselines** | | | | | | |
| `balanced` | 824.219 | 20,781 | 3,428 | 51588.59 | — | 53 ms |
| `linear` | 23.461 | 436 | 5,771 | 2960.08 | — | 756 ms |
| `reverse-linear` | **1.998** | **7** | 2,421 | 2675.92 | — | 632 ms |
| `random` | 547.958 | 8,820 | 4,069 | 18127.53 | — | 44 ms |
| **Default** | | | | | | |
| `portfolio` | 23.161 | 417 | 2,544 | 126.30 | 60 | 18.5 s |
| **Decomposition on a graph view** | | | | | | |
| `flowcutter-primal` | 29.109 | 361 | 2,621 | 162.50 | 91 | 5.2 s |
| `flowcutter-incidence` | 25.035 | 483 | 2,468 | 200.19 | 90 | 4.6 s |
| `goatd-primal` | 23.515 | 431 | 2,229 | 114.62 | **49** | 12.9 s |
| `goatd-primal:refine=off` | 21.598 | 354 | 2,003 | **114.17** | **49** | 14.8 s |
| `goatd-incidence` | 20.673 | 395 | 2,614 | 175.80 | 78 | 7.4 s |
| `guided-bisect` | 25.035 | 483 | 2,468 | 200.20 | — | 41.5 s |
| `flowcutter-primal:budget=2000ms` | 29.109 | 361 | 2,621 | 162.50 | 91 | 5.4 s |
| `flowcutter-primal:budget=100000steps,iters=900` | 23.161 | 417 | 2,544 | 126.30 | 60 | 129.6 s |
| `guided-bisect:budget=2000ms,patience=500` | 25.035 | 483 | 2,468 | 200.20 | — | 42.0 s |
| `flowcutter-primal:root=centroid,place=deep,binarize=balanced` | 87.139 | 2,729 | 1,182 | 3436.19 | 91 | 403 ms |
| `flowcutter-primal:root=first,place=deep,binarize=edge` | 86.996 | 2,729 | 1,182 | 3249.60 | 91 | 399 ms |
| `goatd-incidence:seed=7` | 20.669 | 395 | 2,937 | 179.41 | 79 | 7.3 s |
| **Elimination orders** | | | | | | |
| `minfill-primal` | 20.939 | 325 | 4,383 | 234.46 | 154 | 3.5 s |
| `minfill-incidence` | 20.810 | 422 | 2,956 | 168.50 | 55 | 2.8 s |
| `mindegree-primal` | 21.152 | 395 | 2,677 | 166.87 | 97 | 3.0 s |
| `mindegree-incidence` | 20.693 | 395 | 3,194 | 183.30 | 81 | 3.6 s |
| `nested-dissection-primal` | 19.025 | 438 | 2,679 | 163.23 | 93 | 3.1 s |
| `nested-dissection-incidence` | 18.692 | 421 | 2,699 | 155.93 | 67 | 3.2 s |
| `minfill-primal:ties=jw-sample,seed=7` | 21.364 | 347 | 2,477 | 123.96 | 58 | 2.9 s |
| **Other constructions** | | | | | | |
| `hypergraph-bisect` | 28.445 | 1,088 | 771 | 493.93 | — | 4.6 s |
| `hypergraph-bisect:imbalance=0.40` | 21.305 | 470 | 486 | 643.13 | — | 10.2 s |
| `primal-bisect` | 32.293 | 1,561 | 999 | 556.34 | — | 582 ms |
| `force` | 44.244 | 775 | 772 | 888.52 | — | 712 ms |
| `force:treeify=cut` | 77.860 | 1,594 | 926 | 1112.88 | — | 551 ms |
| `force:root=balance` | 68.229 | 1,414 | 803 | 849.39 | — | 684 ms |
| `force:orient=small` | 44.244 | 775 | 772 | 888.52 | — | 725 ms |
| `force:weights=co` | 39.615 | 956 | **473** | 642.51 | — | 2.8 s |
| `force:feedback=2` | 44.244 | 775 | 772 | 888.52 | — | 2.1 s |
| `force:clause-weight=short` | 59.702 | 1,247 | 1,447 | 831.88 | — | 710 ms |
| `force:dim=3` | 37.067 | 465 | 528 | 742.63 | — | 883 ms |
| `force:restarts=8` | 37.747 | 469 | 658 | 914.85 | — | 5.5 s |
| `force:init=force1d` | 57.941 | 1,574 | 1,565 | 951.47 | — | 719 ms |

![raw-portfolio](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/raw-portfolio.png)

The portfolio's vtree for the raw formula, 6,856 leaves, force-directed layout.

## Preprocessing

`vitri mc2023_track1_008.cnf --out-dir bundle/ --mode mc`:

| step | variables |
| --- | ---: |
| input | 6,856 |
| backbone and dead-variable stripping | 2,853 |
| equivalence reduction | 442 |
| definability elimination | 283 |
| independent-support minimization | 58 |

Result: 58 variables, 145 clauses, one component, with
`count(original) = count(reduced) × 2⁵` recorded in `preprocess.json`. The
reduced formula has 5,368,709,120 models.

## Reduced formula

Same flags, on `bundle/reduced.cnf`.

| `--vtree` spec | stddev | max load | peak ctx | cost | tw | wall |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| **Baselines** | | | | | | |
| `balanced` | 27.076 | 84 | 29 | 62.26 | — | 5 ms |
| `linear` | 6.654 | 37 | 48 | 75.01 | — | 6 ms |
| `reverse-linear` | 4.088 | **15** | 32 | 73.87 | — | 5 ms |
| `random` | 26.582 | 83 | 38 | 72.10 | — | 6 ms |
| **Default** | | | | | | |
| `portfolio` | 9.928 | 32 | 26 | 43.82 | 14 | 155 ms |
| **Decomposition on a graph view** | | | | | | |
| `flowcutter-primal` | 7.511 | 36 | 42 | 46.09 | **13** | 120 ms |
| `flowcutter-incidence` | 9.928 | 32 | 26 | 43.82 | 14 | 125 ms |
| `goatd-primal` | 6.574 | 31 | 30 | 42.31 | **13** | 39 ms |
| `goatd-primal:refine=off` | 4.371 | 16 | 23 | 41.38 | **13** | 1.6 s |
| `goatd-incidence` | 6.777 | 31 | 30 | 44.96 | **13** | 89 ms |
| `guided-bisect` | 11.039 | 40 | 41 | 44.78 | — | 108 ms |
| `flowcutter-primal:budget=2000ms` | 7.511 | 36 | 42 | 46.09 | **13** | 171 ms |
| `flowcutter-primal:budget=100000steps,iters=900` | 7.511 | 36 | 42 | 46.09 | **13** | 4.2 s |
| `guided-bisect:budget=2000ms,patience=500` | 11.039 | 40 | 41 | 44.78 | — | 508 ms |
| `flowcutter-primal:root=centroid,place=deep,binarize=balanced` | 7.486 | 27 | **12** | 47.03 | **13** | 106 ms |
| `flowcutter-primal:root=first,place=deep,binarize=edge` | 7.558 | 27 | **12** | 47.03 | **13** | 107 ms |
| `goatd-incidence:seed=7` | 6.716 | 31 | 30 | 44.96 | **13** | 66 ms |
| **Elimination orders** | | | | | | |
| `minfill-primal` | 6.574 | 31 | 30 | 42.31 | **13** | 29 ms |
| `minfill-incidence` | 7.493 | 38 | 33 | 45.02 | 14 | 30 ms |
| `mindegree-primal` | 7.323 | 34 | 42 | 46.23 | 14 | 28 ms |
| `mindegree-incidence` | 7.136 | 33 | 39 | 46.03 | 14 | 30 ms |
| `nested-dissection-primal` | 6.574 | 31 | 30 | 42.31 | **13** | 24 ms |
| `nested-dissection-incidence` | 6.305 | 31 | 30 | 44.67 | 17 | 26 ms |
| `minfill-primal:ties=jw-sample,seed=7` | 10.016 | 38 | 41 | 44.52 | **13** | 26 ms |
| **Other constructions** | | | | | | |
| `hypergraph-bisect` | 7.969 | 30 | 22 | 47.68 | — | 7 ms |
| `hypergraph-bisect:imbalance=0.40` | 7.344 | 26 | 21 | 46.88 | — | 7 ms |
| `primal-bisect` | 7.969 | 30 | 22 | 47.68 | — | 6 ms |
| `force` | 4.627 | 16 | 17 | 43.89 | — | 6 ms |
| `force:treeify=cut` | 12.008 | 42 | 22 | 54.49 | — | 6 ms |
| `force:root=balance` | 8.429 | 35 | 15 | 47.89 | — | 6 ms |
| `force:orient=small` | 4.627 | 16 | 17 | 43.89 | — | 6 ms |
| `force:weights=co` | **4.086** | 16 | 16 | **40.48** | — | 6 ms |
| `force:feedback=2` | 4.627 | 16 | 17 | 43.89 | — | 7 ms |
| `force:clause-weight=short` | 5.164 | 18 | 16 | 44.22 | — | 8 ms |
| `force:dim=3` | 4.456 | 16 | 17 | 44.00 | — | 7 ms |
| `force:restarts=8` | 4.627 | 16 | 17 | 43.89 | — | 12 ms |
| `force:init=force1d` | 4.785 | 16 | 16 | 43.70 | — | 6 ms |

The individual load scores still favour `reverse-linear`: it has the smallest
maximum load and nearly the smallest standard deviation. The composite `cost`
ranks `force:weights=co` first, `reverse-linear` 36th and `linear` last. It also
ranks the default portfolio above both chains on the raw formula.

Every decomposition spec searches the readings its keys leave open and keeps
the cheapest, which is why several rows here share one tree.

Pictures: leaves are variables; internal nodes show `c=` clause load and
`w=` context width, coloured by clause load relative to that tree's maximum.
One picture per distinct tree; rows with identical trees are listed in the
caption.

### `balanced`

![balanced](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/balanced.png)

Depth 6. The root carries 84 of the 145 clauses; 48 of 57 internal nodes carry none.

### `linear`

![linear](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/linear.png)

A chain over `1..n`. Depth 57, root load 8, maximum load 37, context width 48.

### `reverse-linear`

![reverse-linear](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/reverse-linear.png)

The same chain over the reversed order. Depth 57, root load 12, maximum load 15.

### `random`

![random](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/random.png)

A random tree over a randomly permuted order, both from a fixed seed. Depth 12, root load 83.

### `portfolio`

![portfolio](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/portfolio.png)

The default construction. Depth 22, root load 32, maximum load 32. Same tree:
`flowcutter-incidence`.

`flowcutter-incidence`, adopted. With `VITRI_PORTFOLIO_TRACE=1`:

```text
[portfolio] cand flowcutter-incidence stddev=    9.93 peak_ctx=   26 peak_context_width_show=    - cost=43.82
[portfolio] cand flowcutter-primal  stddev=    7.51 peak_ctx=   42 peak_context_width_show=    - cost=46.09
[portfolio] cand goatd-incidence    stddev=    6.78 peak_ctx=   30 peak_context_width_show=    - cost=44.96
[portfolio] selected: flowcutter-incidence (metric=cost, stddev=9.93, cost=43.82)
```

Candidates are ranked on `cost`.

### `flowcutter-primal`

![flowcutter-primal](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/flowcutter-primal.png)

Flow-based separators on the primal graph. Depth 29, root load 16, maximum load
36. Same tree: `flowcutter-primal:budget=2000ms`,
`flowcutter-primal:budget=100000steps,iters=900`.

### `goatd-primal`

![goatd-primal](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/goatd-primal.png)

The goatd decomposer on the primal graph. Depth 23, root load 10, maximum load
31. Same tree: `minfill-primal`, `nested-dissection-primal`.

### `goatd-primal:refine=off`

![goatd-primal-refine-off](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/goatd-primal-refine-off.png)

The primal decomposition without refinement. Depth 22, root load 11, maximum
load 16.

### `goatd-incidence`

![goatd-incidence](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/goatd-incidence.png)

The goatd decomposer on the incidence graph. Depth 34, root load 2, maximum load
31.

### `guided-bisect`

![guided-bisect](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/guided-bisect.png)

Recursive primal bisection with the incidence decomposition offered at every
level. Depth 22, root load 1, maximum load 40. Same tree:
`guided-bisect:budget=2000ms,patience=500`.

### `flowcutter-primal:root=centroid,place=deep,binarize=balanced`

![flowcutter-primal-centroid](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/flowcutter-primal-root-centroid-place-deep-binarize-balanced.png)

The decomposition rooted at its centroid instead of its first bag. Depth 16, root load 21.

### `flowcutter-primal:root=first,place=deep,binarize=edge`

![flowcutter-primal-td-edge](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/flowcutter-primal-root-first-place-deep-binarize-edge.png)

Each bag binarized along the decomposition's own edges. Depth 15, root load 1.

### `goatd-incidence:seed=7`

![goatd-7](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/goatd-incidence-seed-7.png)

The incidence decomposer at seed 7. Depth 35, root load 2, maximum load 31.

### `minfill-incidence`

![minfill-inc](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/minfill-incidence.png)

Min-fill on the incidence graph. Depth 24, root load 1, maximum load 38.

### `mindegree-primal`

![mindegree](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/mindegree-primal.png)

Greedy min-degree elimination order. Depth 28, root load 17, maximum load 34.

### `mindegree-incidence`

![mindegree-inc](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/mindegree-incidence.png)

Min-degree on the incidence graph. Depth 27, root load 16, maximum load 33.

### `nested-dissection-incidence`

![nested-dissection-inc](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/nested-dissection-incidence.png)

Separator-based elimination on the incidence graph. Depth 29, root load 2,
maximum load 31.

### `minfill-primal:ties=jw-sample,seed=7`

![minfill-sample-jw-7](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/minfill-primal-ties-jw-sample-seed-7.png)

Min-fill with weighted sampled tie-breaking, seed 7. Depth 23, root load 22,
maximum load 38.

### `hypergraph-bisect`

![hypergraph-bisect](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/hypergraph-bisect.png)

Recursive multilevel hypergraph bisection. Depth 16, root load 8, maximum load
30.

### `hypergraph-bisect:imbalance=0.40`

![hypergraph-bisect-imbalance](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/hypergraph-bisect-imbalance-0-40.png)

The same construction with a 0.40 imbalance limit. Depth 17, root load 4,
maximum load 26.

### `primal-bisect`

![primal-bisect](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/primal-bisect.png)

Recursive bisection of the primal graph. Depth 16, root load 8, maximum load
30.

### `force`

![force](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force.png)

No decomposition; the tree is read off a geometric embedding of the variables.
Depth 16, root load 1, maximum load 16. Same tree: `force:feedback=2`,
`force:restarts=8`.

### `force:treeify=cut`

![force-cut](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-treeify-cut.png)

A median cut instead of a spanning tree. Depth 6, root load 37, maximum load 42.

### `force:root=balance`

![force-root-balance](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-root-balance.png)

The spanning tree rooted for balance rather than by merge order. Depth 7, root load 8, maximum load 35.

### `force:orient=small`

![force-orient-small](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-orient-small.png)

The smaller child is placed on the left. Depth 16, root load 1, maximum load
16.

### `force:weights=co`

![force-weights-co](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-weights-co.png)

Co-occurrence weights drive the embedding. Depth 18, root load 1, maximum load
16.

### `force:clause-weight=short`

![force-clause-weight-short](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-clause-weight-short.png)

Short clauses receive more weight. Depth 17, root load 1, maximum load 18.

### `force:dim=3`

![force-dim-3](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-dim-3.png)

The embedding in three dimensions. Depth 17, root load 1, maximum load 16.

### `force:init=force1d`

![force-init-force1d](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-init-force1d.png)

A one-dimensional force layout supplies the initial order. Depth 15, root load
1, maximum load 16.

## End to end

Preprocess, then the default construction (no flags): 1.1 s, `cost` 43.82.
The default construction on the raw formula: 18.5 s, `cost` 126.30.

## Beyond `--vtree`

- A tree decomposition from another solver, as a PACE-format `.td` file, goes through the same conversion to a vtree ([`vtrees.md`](vtrees.md), *Your own decomposition*).
- `vitri::vtree::rotate::rotate_left` and `rotate_right` rotate an existing vtree one edge at a time, for local search over its neighbours ([`vtrees.md`](vtrees.md), *Local search from a vtree*).
- `--candidates N` (N ≤ 8) keeps the portfolio's runners-up, with their scores, in the bundle.
- `--components split`, the default, builds one vtree per independent component; this page uses `--components whole`.

## Reproduce

```sh
cargo build --release
curl -L https://raw.githubusercontent.com/Tractables/cnfs/main/mcc/2023/track1/mc2023_track1_008.cnf.xz \
  | xz -dc > raw.cnf

vitri raw.cnf --out-dir bundle/ --mode mc     # preprocess, then portfolio

for s in \
  balanced linear reverse-linear random portfolio flowcutter-primal \
  flowcutter-incidence goatd-primal \
  'goatd-primal:refine=off' goatd-incidence \
  guided-bisect 'flowcutter-primal:budget=2000ms' \
  'flowcutter-primal:budget=100000steps,iters=900' \
  'guided-bisect:budget=2000ms,patience=500' \
  'flowcutter-primal:root=centroid,place=deep,binarize=balanced' \
  'flowcutter-primal:root=first,place=deep,binarize=edge' \
  'goatd-incidence:seed=7' minfill-primal minfill-incidence \
  mindegree-primal mindegree-incidence nested-dissection-primal \
  nested-dissection-incidence 'minfill-primal:ties=jw-sample,seed=7' \
  hypergraph-bisect 'hypergraph-bisect:imbalance=0.40' primal-bisect force \
  'force:treeify=cut' 'force:root=balance' 'force:orient=small' \
  'force:weights=co' 'force:feedback=2' 'force:clause-weight=short' \
  'force:dim=3' 'force:restarts=8' 'force:init=force1d'
do
  d=$(printf %s "$s" | tr -c 'a-zA-Z0-9' -)
  vitri docs/showcase/mc2023_track1_008.reduced.cnf \
        --out-dir runs/reduced/"$d" --mode mc \
        --no-simplify --no-arjun --components whole --vtree "$s" --dot
  vitri raw.cnf            --out-dir runs/raw/"$d"     --mode mc \
        --no-simplify --no-arjun --components whole --vtree "$s"
done
```

`docs/showcase/mc2023_track1_008.reduced.cnf` is committed here: it is the
`bundle/reduced.cnf` that preprocess line produces, so the reduced runs need
no preprocessing step.

`--dot` writes a Graphviz file beside each `.vtree`. `treewidth` is in
each run's `components.json`; the four scores are `score::VtreeScores::compute`
over the emitted vtree and its formula, and a portfolio run with
`--candidates N` records its candidates' scores in `components.json`.

Walls are single runs. Preprocessing and several constructions are
time-budgeted, so host load can change a reduced formula or tree.

## Source

Model Counting Competition 2023, Track 1, instance 008 (CC BY 4.0). The
competition renumbers submissions; the original benchmark name is not known.

- Fichte, Hecher, Hamiti. "The Model Counting Competition 2020." *ACM JEA* 26 (2021). [doi:10.1145/3459080](https://doi.org/10.1145/3459080)
- Fichte, Hecher. "The Model Counting Competitions 2021–2023." [arXiv:2504.13842](https://arxiv.org/abs/2504.13842)
