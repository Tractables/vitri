# Showcase

Every construction family and several `--vtree` parameters, run on one
competition formula before and after [`--mode mc`](preprocessing.md)
preprocessing. `vitri --help` lists every parameter's values and defaults.

`vitri --mode mc` preprocesses this formula and builds its vtree in 738 ms; the vtree's `cost` is 39.07.
The same construction without preprocessing takes 11.8 s, and its vtree's `cost` is 96.48.

![The balanced vtree over the reduced formula: wide and shallow, with a dark root](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/balanced.png)

<img src="https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/portfolio.png"
     alt="The portfolio's vtree over the reduced formula" width="600">

Two vtrees over the reduced formula: `balanced` above, the default portfolio's
below. Every node is filled by its clause load, and each internal node is
labelled `c=` clause load and `w=` context width
([Drawing a vtree](vtrees.md#drawing-a-vtree)). `balanced` puts 92 of the 147
clauses on its root; the portfolio's tree has at most 31 on any node.

## The instance

`mc2023_track1_008` from Track 1 of the 2023 Model Counting Competition, under
CC BY 4.0: 6,856 variables, 27,626 clauses and 171,798,691,840 models. The
competitions are described in:

- Fichte, Hecher, Hamiti. "The Model Counting Competition 2020." *ACM JEA* 26 (2021). [doi:10.1145/3459080](https://doi.org/10.1145/3459080)
- Fichte, Hecher. "The Model Counting Competitions 2021–2023." [arXiv:2504.13842](https://arxiv.org/abs/2504.13842)

## Preprocessing

`vitri mc2023_track1_008.cnf --out-dir bundle/ --mode mc`:

| step | variables |
| --- | ---: |
| input | 6,856 |
| backbone and dead-variable stripping | 2,853 |
| equivalence reduction | 442 |
| definability elimination | 293 |
| independent-support minimization | 59 |

The reduced formula has 147 clauses in one component, and `preprocess.json`
records `count(original) = count(reduced) × 2⁵`.

## Scores

All are lower-is-better and measured on the emitted vtree against its formula;
`score::VtreeScores` says what each estimates. The tables abbreviate them to
stddev, max load, peak ctx and cost. `tw` is the width of the tree
decomposition the vtree was built from (its widest bag less one), where there
is one; it is not a score. Walls are single runs.

## Reduced formula

Each spec runs on the reduced formula with
`--no-simplify --no-arjun --components whole`, so the formula is used as it
is, and links to a drawing of its tree.

| `--vtree` spec | stddev | max load | peak ctx | cost | tw | wall |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| **Baselines** | | | | | | |
| [`balanced`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/balanced.png) | 32.393 | 92 | 30 | 64.18 | — | 4 ms |
| [`linear`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/linear.png) | 6.392 | 35 | 54 | 78.06 | — | 4 ms |
| [`reverse-linear`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/reverse-linear.png) | 4.669 | 19 | 40 | 76.76 | — | 4 ms |
| [`random`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/random.png) | 31.756 | 103 | 35 | 71.05 | — | 4 ms |
| **Default** | | | | | | |
| [`portfolio`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/portfolio.png) | 8.183 | 31 | 31 | 39.07 | **13** | 134 ms |
| **Decomposition on a graph view** | | | | | | |
| [`flowcutter-primal`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/flowcutter-primal.png) | 5.519 | 20 | 25 | 40.80 | **13** | 113 ms |
| [`flowcutter-incidence`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/flowcutter-incidence.png) | 12.062 | 49 | 41 | 44.63 | **13** | 113 ms |
| [`goatd-primal`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/goatd-primal.png) | 6.500 | 28 | 34 | 39.15 | **13** | 62 ms |
| [`goatd-primal:refine=off`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/flowcutter-primal.png) | 5.519 | 20 | 25 | 40.80 | **13** | 692 ms |
| [`goatd-incidence`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/goatd-incidence.png) | 6.177 | 24 | 27 | **38.66** | 16 | 83 ms |
| [`guided-bisect`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/guided-bisect.png) | 11.802 | 41 | 48 | 66.27 | — | 106 ms |
| [`flowcutter-primal:budget=2000ms`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/flowcutter-primal.png) | 5.519 | 20 | 25 | 40.80 | **13** | 162 ms |
| [`flowcutter-primal:budget=100000steps,iters=900`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/flowcutter-primal.png) | 5.519 | 20 | 25 | 40.80 | **13** | 3.3 s |
| [`guided-bisect:budget=2000ms,patience=500`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/guided-bisect.png) | 11.802 | 41 | 48 | 66.27 | — | 508 ms |
| [`flowcutter-primal:root=centroid,place=deep,binarize=balanced`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/flowcutter-primal-root-centroid-place-deep-binarize-balanced.png) | 6.794 | 22 | **16** | 42.44 | **13** | 106 ms |
| [`flowcutter-primal:root=first,place=deep,binarize=edge`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/flowcutter-primal-root-first-place-deep-binarize-edge.png) | 6.279 | 23 | 23 | 43.55 | **13** | 106 ms |
| [`goatd-incidence:seed=7`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/goatd-incidence-seed-7.png) | 6.585 | 28 | 32 | 38.85 | 17 | 86 ms |
| **Elimination orders** | | | | | | |
| [`minfill-primal`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/minfill-primal.png) | 9.581 | 39 | 34 | 42.10 | **13** | 16 ms |
| [`minfill-incidence`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/minfill-incidence.png) | 7.102 | 28 | 31 | 41.04 | **13** | 14 ms |
| [`mindegree-primal`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/mindegree-primal.png) | 9.581 | 39 | 34 | 42.13 | **13** | 14 ms |
| [`mindegree-incidence`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/mindegree-incidence.png) | 10.181 | 39 | 40 | 42.38 | **13** | 14 ms |
| [`nested-dissection-primal`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/minfill-primal.png) | 9.581 | 39 | 34 | 42.10 | **13** | 14 ms |
| [`nested-dissection-incidence`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/nested-dissection-incidence.png) | 6.229 | 28 | 31 | 40.29 | 15 | 13 ms |
| [`minfill-primal:ties=jw-sample,seed=7`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/minfill-primal-ties-jw-sample-seed-7.png) | 8.394 | 33 | 34 | 41.09 | **13** | 14 ms |
| **Other constructions** | | | | | | |
| [`hypergraph-bisect`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/hypergraph-bisect.png) | 8.504 | 31 | 30 | 45.82 | — | 7 ms |
| [`hypergraph-bisect:imbalance=0.40`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/hypergraph-bisect-imbalance-0-40.png) | 7.936 | 31 | 25 | 45.64 | — | 7 ms |
| [`primal-bisect`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/primal-bisect.png) | 8.504 | 31 | 30 | 45.82 | — | 4 ms |
| [`force`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force.png) | 4.689 | 21 | 17 | 42.38 | — | 5 ms |
| [`force:treeify=cut`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-treeify-cut.png) | 10.983 | 50 | 27 | 50.35 | — | 4 ms |
| [`force:root=balance`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-root-balance.png) | 12.089 | 57 | **16** | 49.52 | — | 4 ms |
| [`force:orient=small`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-orient-small.png) | 4.689 | 21 | 17 | 42.38 | — | 5 ms |
| [`force:weights=co`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-weights-co.png) | 5.552 | 22 | 17 | 41.99 | — | 5 ms |
| [`force:feedback=2`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force.png) | 4.689 | 21 | 17 | 42.38 | — | 6 ms |
| [`force:clause-weight=short`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-clause-weight-short.png) | 5.602 | 26 | 17 | 43.31 | — | 6 ms |
| [`force:dim=3`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-dim-3.png) | **4.124** | **15** | 17 | 41.02 | — | 6 ms |
| [`force:restarts=8`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-restarts-8.png) | 4.671 | 19 | 17 | 41.67 | — | 11 ms |
| [`force:init=force1d`](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/force-init-force1d.png) | 4.933 | 22 | 18 | 42.87 | — | 10 ms |

No single score settles the choice. `force:dim=3` spreads the clauses most
evenly and has the smallest maximum load, with `reverse-linear` close behind on
both, but the chain's peak context width is 40 and `cost` ranks it next to
last. `cost` ranks `goatd-incidence` first and the portfolio third.

Same tree: `flowcutter-primal`, `goatd-primal:refine=off` and the two budgeted
`flowcutter-primal` specs; the two `guided-bisect` specs; `minfill-primal` and
`nested-dissection-primal`; `force` and `force:feedback=2`.

### `portfolio`

With `VITRI_PORTFOLIO_TRACE=1`:

```text
[portfolio] cand flowcutter-incidence stddev=   12.06 peak_ctx=   41 peak_context_width_show=    - cost=44.63
[portfolio] cand flowcutter-primal  stddev=    5.52 peak_ctx=   25 peak_context_width_show=    - cost=40.80
[portfolio] cand goatd-incidence    stddev=    6.18 peak_ctx=   27 peak_context_width_show=    - cost=38.66
[portfolio] cand goatd-incidence:candidate=1 stddev=    6.39 peak_ctx=   34 peak_context_width_show=    - cost=39.88
[portfolio] cand goatd-incidence:candidate=2 stddev=    6.37 peak_ctx=   34 peak_context_width_show=    - cost=39.31
[portfolio] cand goatd-incidence:candidate=3 stddev=    8.18 peak_ctx=   31 peak_context_width_show=    - cost=39.07
[portfolio] wall_ms=128 vars=59 budget_ms=- skip=-
[portfolio] selected: goatd-incidence:candidate=3 (metric=agg, stddev=8.18, cost=39.07)
```

The trace lists the decomposition candidates; `force` is built as well. The
portfolio picks with the ranker shipped in the crate
([The portfolio](vtrees.md#the-portfolio)), not with `cost` alone, so it takes
`goatd-incidence:candidate=3` over the lower-cost `goatd-incidence`.

## Raw formula

The same specs on the input formula, with preprocessing off.

| `--vtree` spec | stddev | max load | peak ctx | cost | tw | wall |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| **Baselines** | | | | | | |
| `balanced` | 824.219 | 20,781 | 3,428 | 52257.91 | — | 58 ms |
| `linear` | 23.461 | 436 | 5,771 | 4458.46 | — | 327 ms |
| `reverse-linear` | **1.998** | **7** | 2,421 | 2411.87 | — | 311 ms |
| `random` | 547.958 | 8,820 | 4,069 | 16972.59 | — | 64 ms |
| **Default** | | | | | | |
| `portfolio` | 22.734 | 1,053 | 542 | 96.48 | **44** | 11.8 s |
| **Decomposition on a graph view** | | | | | | |
| `flowcutter-primal` | 25.873 | 773 | 552 | 94.83 | 91 | 1.1 s |
| `flowcutter-incidence` | 94.965 | 1,240 | 890 | 123.76 | 1,233 | 1.7 s |
| `goatd-primal` | 26.710 | 982 | 615 | 103.12 | 52 | 3.5 s |
| `goatd-primal:refine=off` | 24.000 | 986 | 591 | 98.78 | 52 | 8.4 s |
| `goatd-incidence` | 20.896 | 787 | **331** | 99.06 | **44** | 2.5 s |
| `guided-bisect` | 36.521 | 516 | 443 | 96.74 | — | 20.5 s |
| `flowcutter-primal:budget=2000ms` | 26.734 | 791 | 537 | 101.04 | 76 | 1.5 s |
| `flowcutter-primal:budget=100000steps,iters=900` | 26.009 | 791 | 504 | 95.52 | 60 | 77.1 s |
| `guided-bisect:budget=2000ms,patience=500` | 27.564 | 531 | 442 | 101.12 | — | 12.9 s |
| `flowcutter-primal:root=centroid,place=deep,binarize=balanced` | 87.139 | 2,729 | 1,182 | 2376.17 | 91 | 269 ms |
| `flowcutter-primal:root=first,place=deep,binarize=edge` | 86.996 | 2,729 | 1,182 | 2189.66 | 91 | 271 ms |
| `goatd-incidence:seed=7` | 24.197 | 1,123 | 843 | 126.67 | **44** | 2.6 s |
| **Elimination orders** | | | | | | |
| `minfill-primal` | 27.030 | 965 | 603 | 103.12 | 58 | 746 ms |
| `minfill-incidence` | 20.551 | 527 | 371 | **87.80** | 55 | 1.5 s |
| `mindegree-primal` | 30.822 | 1,248 | 752 | 118.91 | 67 | 757 ms |
| `mindegree-incidence` | 31.021 | 1,475 | 954 | 133.07 | 54 | 1.5 s |
| `nested-dissection-primal` | 25.392 | 801 | 504 | 108.78 | 93 | 930 ms |
| `nested-dissection-incidence` | 25.772 | 1,053 | 742 | 125.31 | 67 | 1.8 s |
| `minfill-primal:ties=jw-sample,seed=7` | 26.748 | 973 | 604 | 106.58 | 54 | 726 ms |
| **Other constructions** | | | | | | |
| `hypergraph-bisect` | 28.667 | 1,088 | 771 | 297.32 | — | 3.0 s |
| `hypergraph-bisect:imbalance=0.40` | 21.400 | 446 | 499 | 196.11 | — | 6.6 s |
| `primal-bisect` | 32.317 | 1,561 | 999 | 308.27 | — | 335 ms |
| `force` | 44.244 | 775 | 772 | 211.61 | — | 403 ms |
| `force:treeify=cut` | 77.860 | 1,594 | 926 | 319.02 | — | 293 ms |
| `force:root=balance` | 68.229 | 1,414 | 803 | 140.48 | — | 396 ms |
| `force:orient=small` | 44.244 | 775 | 772 | 211.61 | — | 404 ms |
| `force:weights=co` | 39.615 | 956 | 473 | 112.26 | — | 1.8 s |
| `force:feedback=2` | 44.244 | 775 | 772 | 211.61 | — | 1.1 s |
| `force:clause-weight=short` | 59.702 | 1,247 | 1,447 | 240.88 | — | 404 ms |
| `force:dim=3` | 37.067 | 465 | 528 | 141.50 | — | 472 ms |
| `force:restarts=8` | 37.747 | 469 | 658 | 172.36 | — | 2.9 s |
| `force:init=force1d` | 57.941 | 1,574 | 1,565 | 274.51 | — | 414 ms |

`reverse-linear` has the lowest standard deviation and maximum load of any
spec, but its peak context width is 2,421 and its `cost` 25 times the
portfolio's.

![The portfolio's vtree over the raw formula](https://raw.githubusercontent.com/Tractables/vitri/assets/showcase/raw-portfolio.png)

The portfolio's vtree over the raw formula: 6,856 leaves in a force-directed
layout.

## Reproduce

```sh
cargo install --path . --locked
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
        --no-simplify --no-arjun --components whole --vtree "$s" --dot
done
```

`docs/showcase/mc2023_track1_008.reduced.cnf` is the reduced formula the tables
use. `--components whole` builds one vtree over the whole formula instead of
one per component, and `--dot` writes a Graphviz drawing beside each `.vtree`.
Each run's `components.json` holds the `treewidth`, and a portfolio run with
`--candidates N` records its candidates' scores there; the scores are
`score::VtreeScores::compute` on the emitted vtree and its formula.
Preprocessing and several constructions have time budgets, so a busier host can
produce a different reduced formula or tree.
