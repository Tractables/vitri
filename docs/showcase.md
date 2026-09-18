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

All lower-is-better, measured on the emitted vtree against its formula. What
each estimates is on `score::VtreeScores`, the type the selector ranks by. The
tables below abbreviate them to stddev, max load, peak ctx and cost, and add
`tw`: the width of the tree decomposition the vtree was built from, its widest
bag less one, where there is one. That is not a score.

## Raw formula

Preprocessing off: `--no-simplify --no-arjun --components whole`.

| `--vtree` spec | stddev | max load | peak ctx | cost | tw | wall |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| **Baselines** | | | | | | |
| `balanced` | 824.219 | 20,781 | 3,428 | 52257.91 | — | 100 ms |
| `linear` | 23.461 | 436 | 5,771 | 4458.46 | — | 2.1 s |
| `reverse-linear` | **1.998** | **7** | 2,421 | 2411.87 | — | 1.6 s |
| `random` | 547.958 | 8,820 | 4,069 | 16972.59 | — | 110 ms |
| **Default** | | | | | | |
| `portfolio` | 22.734 | 1,053 | 542 | 96.48 | **44** | 20.7 s |
| **Decomposition on a graph view** | | | | | | |
| `flowcutter-primal` | 30.148 | 1,284 | 781 | 126.40 | 112 | 4.7 s |
| `flowcutter-incidence` | 181.483 | 1,865 | 890 | 123.33 | 3,164 | 10.5 s |
| `goatd-primal` | 26.710 | 982 | 615 | 103.12 | 52 | 5.7 s |
| `goatd-primal:refine=off` | 25.214 | 937 | 611 | 97.12 | 52 | 27.7 s |
| `goatd-incidence` | 20.896 | 787 | **331** | 99.06 | **44** | 3.7 s |
| `guided-bisect` | 84.814 | 1,520 | 671 | 106.45 | — | 98.5 s |
| `flowcutter-primal:budget=2000ms` | 25.873 | 773 | 552 | 94.83 | 91 | 4.8 s |
| `flowcutter-primal:budget=100000steps,iters=900` | 26.009 | 791 | 504 | 95.52 | 60 | 104.4 s |
| `guided-bisect:budget=2000ms,patience=500` | 27.564 | 531 | 442 | 101.12 | — | 33.2 s |
| `flowcutter-primal:root=centroid,place=deep,binarize=balanced` | 69.950 | 1,661 | 1,024 | 165.14 | 112 | 317 ms |
| `flowcutter-primal:root=first,place=deep,binarize=edge` | 71.505 | 1,748 | 1,054 | 185.78 | 112 | 312 ms |
| `goatd-incidence:seed=7` | 24.197 | 1,123 | 843 | 126.67 | **44** | 3.7 s |
| **Elimination orders** | | | | | | |
| `minfill-primal` | 27.030 | 965 | 603 | 103.12 | 58 | 1.8 s |
| `minfill-incidence` | 20.551 | 527 | 371 | **87.80** | 55 | 2.4 s |
| `mindegree-primal` | 30.822 | 1,248 | 752 | 118.91 | 67 | 1.9 s |
| `mindegree-incidence` | 31.021 | 1,475 | 954 | 133.07 | 54 | 2.5 s |
| `nested-dissection-primal` | 25.392 | 801 | 504 | 108.78 | 93 | 2.1 s |
| `nested-dissection-incidence` | 25.772 | 1,053 | 742 | 125.31 | 67 | 2.8 s |
| `minfill-primal:ties=jw-sample,seed=7` | 26.748 | 973 | 604 | 106.58 | 54 | 1.7 s |
| **Other constructions** | | | | | | |
| `hypergraph-bisect` | 28.667 | 1,088 | 771 | 297.32 | — | 3.9 s |
| `hypergraph-bisect:imbalance=0.40` | 21.400 | 446 | 499 | 196.11 | — | 8.5 s |
| `primal-bisect` | 32.317 | 1,561 | 999 | 308.27 | — | 475 ms |
| `force` | 44.244 | 775 | 772 | 211.61 | — | 728 ms |
| `force:treeify=cut` | 77.860 | 1,594 | 926 | 319.02 | — | 571 ms |
| `force:root=balance` | 68.229 | 1,414 | 803 | 140.48 | — | 718 ms |
| `force:orient=small` | 44.244 | 775 | 772 | 211.61 | — | 724 ms |
| `force:weights=co` | 39.615 | 956 | 473 | 112.26 | — | 2.5 s |
| `force:feedback=2` | 44.244 | 775 | 772 | 211.61 | — | 2.0 s |
| `force:clause-weight=short` | 59.702 | 1,247 | 1,447 | 240.88 | — | 710 ms |
| `force:dim=3` | 37.067 | 465 | 528 | 141.50 | — | 796 ms |
| `force:restarts=8` | 37.747 | 469 | 658 | 172.36 | — | 5.2 s |
| `force:init=force1d` | 57.941 | 1,574 | 1,565 | 274.51 | — | 738 ms |

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
| `balanced` | 27.076 | 84 | 29 | 57.00 | — | 12 ms |
| `linear` | 6.654 | 37 | 48 | 76.28 | — | 12 ms |
| `reverse-linear` | 4.088 | **15** | 32 | 67.59 | — | 7 ms |
| `random` | 26.582 | 83 | 38 | 67.44 | — | 10 ms |
| **Default** | | | | | | |
| `portfolio` | 4.481 | **15** | 13 | 40.20 | **13** | 272 ms |
| **Decomposition on a graph view** | | | | | | |
| `flowcutter-primal` | 4.481 | **15** | 13 | 40.20 | **13** | 121 ms |
| `flowcutter-incidence` | 7.455 | 31 | 30 | 40.18 | 14 | 124 ms |
| `goatd-primal` | 6.599 | 31 | 23 | 38.42 | **13** | 147 ms |
| `goatd-primal:refine=off` | 7.906 | 31 | 28 | **38.27** | **13** | 485 ms |
| `goatd-incidence` | 6.682 | 31 | 23 | 38.49 | **13** | 180 ms |
| `guided-bisect` | 11.696 | 40 | 41 | 57.99 | — | 110 ms |
| `flowcutter-primal:budget=2000ms` | 4.481 | **15** | 13 | 40.20 | **13** | 171 ms |
| `flowcutter-primal:budget=100000steps,iters=900` | 4.481 | **15** | 13 | 40.20 | **13** | 3.5 s |
| `guided-bisect:budget=2000ms,patience=500` | 11.696 | 40 | 41 | 57.99 | — | 514 ms |
| `flowcutter-primal:root=centroid,place=deep,binarize=balanced` | 7.486 | 27 | **12** | 42.22 | **13** | 111 ms |
| `flowcutter-primal:root=first,place=deep,binarize=edge` | 7.558 | 27 | **12** | 42.26 | **13** | 111 ms |
| `goatd-incidence:seed=7` | 6.682 | 31 | 23 | 38.49 | **13** | 184 ms |
| **Elimination orders** | | | | | | |
| `minfill-primal` | 9.350 | 36 | 28 | 41.42 | 14 | 28 ms |
| `minfill-incidence` | 9.478 | 36 | 28 | 41.43 | 14 | 46 ms |
| `mindegree-primal` | 6.464 | 31 | 30 | 40.99 | 14 | 28 ms |
| `mindegree-incidence` | 6.503 | 31 | 31 | 41.38 | 14 | 45 ms |
| `nested-dissection-primal` | 9.350 | 36 | 28 | 41.42 | 14 | 43 ms |
| `nested-dissection-incidence` | 4.641 | 18 | 14 | 40.45 | 17 | 38 ms |
| `minfill-primal:ties=jw-sample,seed=7` | 9.210 | 31 | 28 | 39.46 | 14 | 42 ms |
| **Other constructions** | | | | | | |
| `hypergraph-bisect` | 7.702 | 30 | 22 | 44.78 | — | 16 ms |
| `hypergraph-bisect:imbalance=0.40` | 7.702 | 30 | 22 | 44.78 | — | 17 ms |
| `primal-bisect` | 7.702 | 30 | 22 | 44.78 | — | 12 ms |
| `force` | 4.627 | 16 | 17 | 39.74 | — | 15 ms |
| `force:treeify=cut` | 12.008 | 42 | 22 | 49.02 | — | 15 ms |
| `force:root=balance` | 8.429 | 35 | 15 | 44.34 | — | 16 ms |
| `force:orient=small` | 4.627 | 16 | 17 | 39.74 | — | 15 ms |
| `force:weights=co` | **4.086** | 16 | 16 | 38.96 | — | 15 ms |
| `force:feedback=2` | 4.627 | 16 | 17 | 39.74 | — | 18 ms |
| `force:clause-weight=short` | 5.164 | 18 | 16 | 40.22 | — | 12 ms |
| `force:dim=3` | 4.456 | 16 | 17 | 39.84 | — | 15 ms |
| `force:restarts=8` | 4.627 | 16 | 17 | 39.74 | — | 53 ms |
| `force:init=force1d` | 4.785 | 16 | 16 | 39.73 | — | 16 ms |

The individual load scores still favour `reverse-linear`: it shares the
smallest maximum load and has nearly the smallest standard deviation. The
composite `cost` ranks `goatd-primal:refine=off` first, `reverse-linear` 36th
and `linear` last. It also ranks the default portfolio above both chains on the
raw formula.

Every decomposition spec searches the readings its keys leave open and keeps
the cheapest, which is why several rows here share one tree.

Pictures: leaves are variables; internal nodes show `c=` clause load and
`w=` context width, coloured by clause load relative to that tree's maximum.
One picture per distinct tree; rows with identical trees are listed in the
caption.

### `balanced`

![balanced](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/balanced.png)

Depth 6. The root carries 84 of the 145 clauses; 48 of 57 internal nodes carry
none.

### `linear`

![linear](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/linear.png)

A chain over `1..n`. Depth 57, root load 8, maximum load 37, context width 48.

### `reverse-linear`

![reverse-linear](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/reverse-linear.png)

The same chain over the reversed order. Depth 57, root load 12, maximum load 15.

### `random`

![random](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/random.png)

A random tree over a randomly permuted order, both from a fixed seed. Depth 12,
root load 83.

### `portfolio`

![portfolio](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/portfolio.png)

The default construction. Depth 24, root load 3, maximum load 15. Same tree:
`flowcutter-primal`, `flowcutter-primal:budget=2000ms`,
`flowcutter-primal:budget=100000steps,iters=900`.

`flowcutter-primal`, adopted. With `VITRI_PORTFOLIO_TRACE=1`:

```text
[portfolio] cand flowcutter-incidence stddev=    7.46 peak_ctx=   30 peak_context_width_show=    - cost=40.18
[portfolio] cand flowcutter-primal  stddev=    4.48 peak_ctx=   13 peak_context_width_show=    - cost=40.20
[portfolio] cand goatd-incidence    stddev=    6.68 peak_ctx=   23 peak_context_width_show=    - cost=38.49
[portfolio] cand goatd-incidence:candidate=1 stddev=    8.51 peak_ctx=   30 peak_context_width_show=    - cost=42.06
[portfolio] cand goatd-incidence:candidate=2 stddev=    8.51 peak_ctx=   28 peak_context_width_show=    - cost=41.78
[portfolio] cand goatd-incidence:candidate=3 stddev=    9.67 peak_ctx=   30 peak_context_width_show=    - cost=40.51
[portfolio] wall_ms=261 vars=58 budget_ms=- skip=-
[portfolio] selected: flowcutter-primal (metric=agg, stddev=4.48, cost=40.20)
```

The pick is made by the ranker shipped in the crate ([`vtrees.md`](vtrees.md),
*The portfolio*), not by `cost` alone: `goatd-incidence` has the lowest cost
here and is not chosen.

### `flowcutter-incidence`

![flowcutter-incidence](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/flowcutter-incidence.png)

Flow-based separators on the incidence graph. Depth 31, root load 2, maximum
load 31.

### `goatd-primal`

![goatd-primal](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/goatd-primal.png)

The goatd decomposer on the primal graph. Depth 18, root load 1, maximum load
31.

### `goatd-primal:refine=off`

![goatd-primal:refine=off](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/goatd-primal-refine-off.png)

The primal decomposition without refinement. Depth 20, root load 6, maximum load
31.

### `goatd-incidence`

![goatd-incidence](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/goatd-incidence.png)

The goatd decomposer on the incidence graph. Depth 18, root load 9, maximum load
31. Same tree: `goatd-incidence:seed=7`.

### `guided-bisect`

![guided-bisect](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/guided-bisect.png)

Recursive primal bisection with the incidence decomposition offered at every
level. Depth 22, root load 1, maximum load 40. Same tree:
`guided-bisect:budget=2000ms,patience=500`.

### `flowcutter-primal:root=centroid,place=deep,binarize=balanced`

![flowcutter-primal:root=centroid,place=deep,binarize=balanced](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/flowcutter-primal-root-centroid-place-deep-binarize-balanced.png)

The decomposition rooted at its centroid instead of its first bag. Depth 16,
root load 21.

### `flowcutter-primal:root=first,place=deep,binarize=edge`

![flowcutter-primal:root=first,place=deep,binarize=edge](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/flowcutter-primal-root-first-place-deep-binarize-edge.png)

Each bag binarized along the decomposition's own edges. Depth 15, root load 1.

### `minfill-primal`

![minfill-primal](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/minfill-primal.png)

Greedy min-fill elimination order. Depth 18, root load 6, maximum load 36. Same
tree: `nested-dissection-primal`.

### `minfill-incidence`

![minfill-incidence](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/minfill-incidence.png)

Min-fill on the incidence graph. Depth 20, root load 2, maximum load 36.

### `mindegree-primal`

![mindegree-primal](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/mindegree-primal.png)

Greedy min-degree elimination order. Depth 28, root load 10, maximum load 31.

### `mindegree-incidence`

![mindegree-incidence](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/mindegree-incidence.png)

Min-degree on the incidence graph. Depth 33, root load 2, maximum load 31.

### `nested-dissection-incidence`

![nested-dissection-incidence](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/nested-dissection-incidence.png)

Separator-based elimination on the incidence graph. Depth 22, root load 2,
maximum load 18.

### `minfill-primal:ties=jw-sample,seed=7`

![minfill-primal:ties=jw-sample,seed=7](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/minfill-primal-ties-jw-sample-seed-7.png)

Min-fill with weighted sampled tie-breaking, seed 7. Depth 18, root load 6,
maximum load 31.

### `hypergraph-bisect`

![hypergraph-bisect](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/hypergraph-bisect.png)

Recursive multilevel hypergraph bisection. Depth 17, root load 8, maximum load
30. Same tree: `hypergraph-bisect:imbalance=0.40`, `primal-bisect`.

### `force`

![force](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force.png)

No decomposition; the tree is read off a geometric embedding of the variables.
Depth 16, root load 1, maximum load 16. Same tree: `force:feedback=2`,
`force:restarts=8`.

### `force:treeify=cut`

![force:treeify=cut](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-treeify-cut.png)

A median cut instead of a spanning tree. Depth 6, root load 37, maximum load 42.

### `force:root=balance`

![force:root=balance](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-root-balance.png)

The spanning tree rooted for balance rather than by merge order. Depth 7, root
load 8, maximum load 35.

### `force:orient=small`

![force:orient=small](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-orient-small.png)

The smaller child is placed on the left. Depth 16, root load 1, maximum load 16.

### `force:weights=co`

![force:weights=co](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-weights-co.png)

Co-occurrence weights drive the embedding. Depth 18, root load 1, maximum load
16.

### `force:clause-weight=short`

![force:clause-weight=short](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-clause-weight-short.png)

Short clauses receive more weight. Depth 17, root load 1, maximum load 18.

### `force:dim=3`

![force:dim=3](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-dim-3.png)

The embedding in three dimensions. Depth 17, root load 1, maximum load 16.

### `force:init=force1d`

![force:init=force1d](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-init-force1d.png)

A one-dimensional force layout supplies the initial order. Depth 15, root load
1, maximum load 16.

## End to end

Preprocess, then the default construction (no flags): 1.1 s, `cost` 40.20.
The default construction on the raw formula: 20.7 s, `cost` 96.48.

## Beyond `--vtree`

- A tree decomposition from another solver, as a PACE-format `.td` file, goes through the same conversion to a vtree ([`vtrees.md`](vtrees.md), *Your own decomposition*).
- `vitri::vtree::rotate::rotate_left` and `rotate_right` rotate an existing vtree one edge at a time, for local search over its neighbours ([`vtrees.md`](vtrees.md), *Local search from a vtree*).
- `--candidates N` keeps the portfolio's runners-up, with their scores, in the bundle.
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
each run's `components.json`; the scores are `score::VtreeScores::compute`
over the emitted vtree and its formula, and a portfolio run with
`--candidates N` records its candidates' scores in `components.json`.

Walls are single runs. Preprocessing and several constructions are
time-budgeted, so host load can change a reduced formula or tree.

## Source

Model Counting Competition 2023, Track 1, instance 008 (CC BY 4.0). The
competition renumbers submissions; the original benchmark name is not known.

- Fichte, Hecher, Hamiti. "The Model Counting Competition 2020." *ACM JEA* 26 (2021). [doi:10.1145/3459080](https://doi.org/10.1145/3459080)
- Fichte, Hecher. "The Model Counting Competitions 2021–2023." [arXiv:2504.13842](https://arxiv.org/abs/2504.13842)
