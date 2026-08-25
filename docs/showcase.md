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

The rows whose specs leave a conversion key unwritten were measured before those
keys named a search over readings. They denote a different construction now, and
are regenerated with the next run of the commands below.

## Raw formula

Preprocessing off: `--no-simplify --no-arjun --components whole`.

| `--vtree` spec | stddev | max load | peak ctx | cost | tw | wall |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| **Baselines** | | | | | | |
| `balanced` | 824.219 | 20,781 | 3,428 | 8.97e12 | — | 13 ms |
| `linear` | 23.461 | 436 | 5,771 | 8.73e7 | — | 14 ms |
| `reverse-linear` | **1.998** | **7** | 2,421 | **457,860** | — | 14 ms |
| `random` | 547.958 | 8,820 | 4,069 | 6.86e11 | — | 13 ms |
| **Default** | | | | | | |
| `portfolio` | 24.131 | 934 | 624 | 1.11e9 | 57 | 7.6 s |
| **Decomposition on a graph view** | | | | | | |
| `flowcutter-primal` | 87.139 | 2,729 | 1,182 | 2.06e10 | 91 | 224 ms |
| `flowcutter-incidence` | 44.045 | 1,250 | 892 | 2.27e9 | 90 | 553 ms |
| `goatd-primal` | 28.176 | 980 | 554 | 1.21e9 | **49** | 9.7 s |
| `goatd-primal:refine=off` | 26.699 | 858 | 618 | 9.14e8 | **49** | 3.1 s |
| `goatd-incidence` | 36.366 | 2,223 | 1,578 | 1.13e10 | 78 | 3.3 s |
| `guided-bisect` | 26.687 | 471 | 436 | 4.11e8 | — | 5.3 s |
| `flowcutter-primal:budget=2000ms` | 66.526 | 1,678 | 1,142 | 5.02e9 | 76 | 625 ms |
| `flowcutter-primal:budget=100000steps,iters=900` | 25.638 | 744 | 447 | 7.09e8 | 60 | 76.5 s |
| `flowcutter-primal:root=centroid,place=deep,binarize=balanced` | 87.139 | 2,729 | 1,182 | 2.06e10 | 91 | 263 ms |
| `flowcutter-primal:root=first,place=deep,binarize=edge` | 86.996 | 2,729 | 1,182 | 2.06e10 | 91 | 267 ms |
| `goatd-incidence:seed=7` | 30.783 | 1,453 | 847 | 3.35e9 | 79 | 3.3 s |
| **Elimination orders** | | | | | | |
| `minfill-primal` | 30.158 | 787 | 376 | 7.23e8 | 154 | 585 ms |
| `minfill-incidence` | 25.340 | 662 | 451 | 5.89e8 | 55 | 703 ms |
| `mindegree-primal` | 44.766 | 1,957 | 1,076 | 7.75e9 | 97 | 280 ms |
| `mindegree-incidence` | 31.955 | 1,350 | 794 | 2.74e9 | 81 | 432 ms |
| `nested-dissection-primal` | 23.954 | 724 | 415 | 6.65e8 | 93 | 389 ms |
| `nested-dissection-incidence` | 26.047 | 930 | 613 | 1.10e9 | 73 | 626 ms |
| `minfill-primal:ties=jw-sample,seed=7` | 27.125 | 718 | 654 | 6.49e8 | 58 | 326 ms |
| **Other constructions** | | | | | | |
| `hypergraph-bisect` | 26.818 | 1,088 | 771 | 1.61e9 | — | 2.4 s |
| `hypergraph-bisect:imbalance=0.40` | 20.642 | 358 | 425 | 3.54e8 | — | 5.9 s |
| `force` | 44.244 | 775 | 772 | 7.58e8 | — | 370 ms |
| `force:treeify=cut` | 77.860 | 1,594 | 926 | 4.34e9 | — | 260 ms |
| `force:dim=3` | 37.067 | 465 | 528 | 4.02e8 | — | 441 ms |
| `force:restarts=8` | 37.747 | 469 | 658 | 4.04e8 | — | 2.9 s |
| `force:root=balance` | 68.229 | 1,414 | 803 | 3.13e9 | — | 362 ms |

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
| `balanced` | 27.076 | 84 | 29 | 594,084 | — | <1 ms |
| `linear` | 6.654 | 37 | 48 | 51,298 | — | <1 ms |
| `reverse-linear` | **4.088** | **15** | 32 | **4,080** | — | <1 ms |
| `random` | 26.582 | 83 | 38 | 572,466 | — | <1 ms |
| **Default** | | | | | | |
| `portfolio` | 4.812 | 17 | 12 | 6,493 | **13** | 45 ms |
| **Decomposition on a graph view** | | | | | | |
| `flowcutter-primal` | 4.812 | 17 | 12 | 6,493 | **13** | 102 ms |
| `flowcutter-incidence` | 5.539 | 19 | **11** | 9,192 | 14 | 104 ms |
| `goatd-primal` | 6.950 | 27 | 14 | 22,782 | **13** | 30 ms |
| `goatd-primal:refine=off` | 4.812 | 17 | 12 | 6,493 | **13** | 251 ms |
| `goatd-incidence` | 5.213 | 21 | 13 | 11,977 | **13** | 30 ms |
| `guided-bisect` | 7.860 | 33 | 20 | 39,485 | — | 101 ms |
| `flowcutter-primal:budget=2000ms` | 4.812 | 17 | 12 | 6,493 | **13** | 152 ms |
| `flowcutter-primal:budget=100000steps,iters=900` | 7.678 | 27 | 12 | 21,611 | **13** | 2.9 s |
| `flowcutter-primal:root=centroid,place=deep,binarize=balanced` | 7.486 | 27 | 12 | 21,735 | **13** | 101 ms |
| `flowcutter-primal:root=first,place=deep,binarize=edge` | 7.558 | 27 | 12 | 21,794 | **13** | 100 ms |
| `goatd-incidence:seed=7` | 5.146 | 21 | 13 | 11,977 | **13** | 30 ms |
| **Elimination orders** | | | | | | |
| `minfill-primal` | 6.950 | 27 | 14 | 22,782 | **13** | 3 ms |
| `minfill-incidence` | 5.154 | 26 | 17 | 20,326 | 14 | 5 ms |
| `mindegree-primal` | 7.286 | 28 | 14 | 23,253 | 14 | 3 ms |
| `mindegree-incidence` | 5.153 | 22 | 14 | 11,995 | 14 | 3 ms |
| `nested-dissection-primal` | 6.950 | 27 | 14 | 22,782 | **13** | 3 ms |
| `nested-dissection-incidence` | 6.854 | 27 | 13 | 21,842 | 14 | 5 ms |
| `minfill-primal:ties=jw-sample,seed=7` | 7.975 | 32 | 13 | 34,446 | **13** | 3 ms |
| **Other constructions** | | | | | | |
| `hypergraph-bisect` | 6.613 | 32 | 16 | 37,918 | — | 1 ms |
| `hypergraph-bisect:imbalance=0.40` | 6.613 | 32 | 16 | 37,918 | — | 1 ms |
| `force` | 4.627 | 16 | 17 | 8,756 | — | 1 ms |
| `force:treeify=cut` | 12.008 | 42 | 22 | 77,556 | — | 1 ms |
| `force:dim=3` | 4.456 | 16 | 17 | 8,846 | — | 1 ms |
| `force:restarts=8` | 4.627 | 16 | 17 | 8,756 | — | 3 ms |
| `force:root=balance` | 8.429 | 35 | 15 | 48,473 | — | 1 ms |

`reverse-linear` takes `clause_load_stddev`, `max_clause_load` and `cost`;
`flowcutter-incidence` takes `peak_context_width_all`. The four baselines hold
the last four places on `peak_context_width_all`, and `balanced` and `random`
the last two on each of the other three columns. The portfolio's tree is 5th,
5th, 2nd and 2nd of the rows.

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

The default construction. Depth 22, root load 4, maximum load 17. Same tree: `flowcutter-primal`, `goatd-primal:refine=off`, `flowcutter-primal:budget=2000ms`.

`flowcutter-primal`, adopted. With `VITRI_PORTFOLIO_TRACE=1`:

```text
[portfolio] cand flowcutter-incidence stddev=  5.54 peak_ctx= 11 cost=9192
[portfolio] cand flowcutter-primal    stddev=  4.81 peak_ctx= 12 cost=6493
[portfolio] cand goatd-incidence      stddev=  5.21 peak_ctx= 13 cost=11977
[portfolio] selected: flowcutter-primal (metric=stddev, stddev=4.81, cost=6493)
```

Candidates are ranked on `clause_load_stddev`.

### `flowcutter-incidence`

![flowcutter-incidence](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/flowcutter-incidence.png)

Flow-based separators on the incidence graph. Depth 19, root load 15, context width 11.

### `goatd-incidence`

![goatd](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/goatd.png)

This crate's decomposer on the incidence graph. Depth 21, root load 1, maximum load 21.

### `guided-bisect`

![guided-bisect](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/hybrid-flowcutter-incidence.png)

Recursive primal bisection with the incidence decomposition offered at every level. Depth 14, root load 1, maximum load 33.

### `flowcutter-primal:budget=100000steps,iters=900`

![flowcutter-primal-100000-900steps](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/flowcutter-primal-100000-900steps.png)

A step budget in place of the 200 ms default. Depth 18, root load 1, maximum load 27.

### `flowcutter-primal:root=centroid,place=deep,binarize=balanced`

![flowcutter-primal-centroid](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/flowcutter-primal-centroid.png)

The decomposition rooted at its centroid instead of its first bag. Depth 16, root load 21.

### `flowcutter-primal:root=first,place=deep,binarize=edge`

![flowcutter-primal-td-edge](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/flowcutter-primal-td-edge.png)

Each bag binarized along the decomposition's own edges. Depth 15, root load 1.

### `goatd-incidence:seed=7`

![goatd-7](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/goatd-7.png)

The incidence decomposer at seed 7. Depth 21, root load 1.

### `minfill-primal`

![minfill](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/minfill.png)

Greedy min-fill elimination order. Depth 15, root load 4, bags of at most 14. Same tree: `nested-dissection-primal`, `goatd-primal`.

### `minfill-incidence`

![minfill-inc](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/minfill-inc.png)

Min-fill on the incidence graph. Depth 18, root load 4, maximum load 26.

### `mindegree-primal`

![mindegree](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/mindegree.png)

Greedy min-degree elimination order. Depth 22, root load 1, maximum load 28.

### `mindegree-incidence`

![mindegree-inc](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/mindegree-inc.png)

Min-degree on the incidence graph. Depth 29, root load 1, maximum load 22.

### `nested-dissection-incidence`

![nested-dissection-inc](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/nested-dissection-inc.png)

Separator-based elimination on the incidence graph. Depth 24, root load 26.

### `minfill-primal:ties=jw-sample,seed=7`

![minfill-sample-jw-7](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/minfill-sample-jw-7.png)

Min-fill with weighted sampled tie-breaking, seed 7. Depth 17, root load 9, maximum load 32.

### `hypergraph-bisect`

![hypergraph-bisect](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/hypergraph-bisect.png)

Recursive multilevel hypergraph bisection. Depth 21, root load 8, maximum load 32. Same tree: `hypergraph-bisect:imbalance=0.40`.

### `force`

![force](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force.png)

No decomposition; the tree is read off a geometric embedding of the variables. Depth 16, root load 1, maximum load 16. Same tree: `force:restarts=8`.

### `force:treeify=cut`

![force-cut](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-cut.png)

A median cut instead of a spanning tree. Depth 6, root load 37, maximum load 42.

### `force:dim=3`

![force-d-3](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-d-3.png)

The embedding in three dimensions. Depth 17, root load 1, maximum load 16.

### `force:root=balance`

![force-root-balance](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-root-balance.png)

The spanning tree rooted for balance rather than by merge order. Depth 7, root load 8, maximum load 35.

## End to end

Preprocess, then the default construction (no flags): 0.9 s, `cost` 6,493.
The default construction on the raw formula: 7.6 s, `cost` 1,106,991,469.

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
  'flowcutter-primal:root=centroid,place=deep,binarize=balanced' \
  'flowcutter-primal:root=first,place=deep,binarize=edge' \
  'goatd-incidence:seed=7' minfill-primal minfill-incidence \
  mindegree-primal mindegree-incidence nested-dissection-primal \
  nested-dissection-incidence 'minfill-primal:ties=jw-sample,seed=7' \
  hypergraph-bisect 'hypergraph-bisect:imbalance=0.40' force \
  'force:treeify=cut' 'force:dim=3' 'force:restarts=8' 'force:root=balance'
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

Walls are single runs taken with several runs in parallel. Preprocessing and several constructions are time-budgeted, so a
loaded machine can give a different reduced formula or tree.

## Source

Model Counting Competition 2023, Track 1, instance 008 (CC BY 4.0). The
competition renumbers submissions; the original benchmark name is not known.

- Fichte, Hecher, Hamiti. "The Model Counting Competition 2020." *ACM JEA* 26 (2021). [doi:10.1145/3459080](https://doi.org/10.1145/3459080)
- Fichte, Hecher. "The Model Counting Competitions 2021–2023." [arXiv:2504.13842](https://arxiv.org/abs/2504.13842)
