# Acknowledgements

`vitri` preprocesses a CNF and builds a vtree for it. Both rest on prior work:
SAT preprocessors, the papers that introduced the preprocessing techniques we
reimplemented, and the work that introduced vtrees.

## Code we build and link

`build.rs` compiles the vendored Arjun stack. Licences and copyright are in
[`THIRD-PARTY.md`](THIRD-PARTY.md); pinned commits for the Arjun stack are in
[`vendor/arjun/upstream/PROVENANCE.md`](vendor/arjun/upstream/PROVENANCE.md).

Tree-decomposition construction and graph partitioning come from
[goatd](https://github.com/Tractables/goatd); its
[acknowledgements](https://github.com/Tractables/goatd/blob/main/docs/ACKNOWLEDGEMENTS.md)
credit those implementations and algorithms.

Vendored under `vendor/arjun/upstream/`:

| Project | Authors | Role here |
|---|---|---|
| **Arjun** | Mate Soos, Kuldeep S. Meel | Independent-support computation and formula reduction — the single biggest contributor to preprocessing quality |
| **CryptoMiniSat** | Mate Soos et al. | SAT backend underneath Arjun; its `oracle`/`puura` machinery drives several preprocessing steps |
| **CaDiCaL** | Armin Biere et al. | The SAT solver behind our own preprocessing — backbone stripping, dead-variable removal, DVE — and Arjun's |
| **CadiBack** | Armin Biere et al. | Backbone extraction |
| **SBVA** | Andrew Haberlandt, Harrison Green, Marijn Heule | Structured bounded variable addition |
| **Eigen** | Eigen contributors | Linear algebra used internally by SBVA, which bundles it |

- Mate Soos, Kuldeep S. Meel. *Arjun: An Efficient Independent Support Computation Technique and its Applications to Counting and Sampling.* ICCAD 2022.
- Armin Biere, Tobias Faller, Katalin Fazekas, Mathias Fleury, Nils Froleyks, Florian Pollitt. *CaDiCaL 2.0.* CAV 2024.
- Andrew Haberlandt, Harrison Green, Marijn J. H. Heule. *Effective Auxiliary Variables via Structured Reencoding.* SAT 2023 (LIPIcs 271:11).
- Mate Soos, Karsten Nohl, Claude Castelluccia. *Extending SAT Solvers to Cryptographic Problems.* SAT 2009. (CryptoMiniSat.)
- Armin Biere, Nils Froleyks, Wenxi Wang. *CadiBack: Extracting Backbones with CaDiCaL.* SAT 2023 (LIPIcs 271:3).

## Preprocessing algorithms

The distinction that organises our whole preprocessing pipeline — that a
variable may be removed without changing the model count **only** when it is
*defined* by the others, whereas plain resolution-based elimination
existentially forgets it — comes from this line of work:

- Jean-Marie Lagniez, Pierre Marquis. *Preprocessing for Propositional Model Counting.* AAAI 2014. The `pmc` preprocessor: gate detection, literal equivalence, and size-gated elimination that preserves the count. Extended as *On Preprocessing Techniques and Their Impact on Propositional Model Counting*, Journal of Automated Reasoning 58:413–481, 2017.
- Jean-Marie Lagniez, Emmanuel Lonca, Pierre Marquis. *Improving Model Counting by Leveraging Definability.* IJCAI 2016. The `B+E` preprocessor: input/output bipartition, then elimination of the *defined* side. Journal account: *Definability for Model Counting*, Artificial Intelligence 281:103229, 2020.
- Jérôme Lang, Pierre Marquis. *On Propositional Definability.* Artificial Intelligence 172(8–9):991–1017, 2008. The definability notion the two above rest on.
- Jean-Marie Lagniez, Pierre Marquis, Armin Biere. *Dynamic Blocked Clause Elimination for Projected Model Counting.* SAT 2024 (LIPIcs 305:21). Elimination that is sound for a *projected* count but not for a plain one — the reason our projected path may do things the plain path must not.
- Mate Soos, Kuldeep S. Meel. *Arjun* (ICCAD 2022, cited in full above). Independent-support computation built on that definability line. Vendored as our strongest preprocessing stage, and the biggest single influence on the stages we wrote ourselves.

The definability test itself rests on **Padoa's method** (Alessandro Padoa,
1901), reached through the SAT encoding used by the works above.

**Definite variable elimination (DVE)** — our SAT-based DVE follows the technique
GPMC made its signature, exploiting functional dependence to remove a very large
fraction of variables outright.

- Ryosuke Suzuki, Kenji Hashimoto, Masahiko Sakai. *Improvement of Projected Model-Counting Solver with Component Decomposition Using SAT Solving in Components.* JSAI Technical Report SIG-FPAI-103-07, 2017 (in Japanese). (GPMC.)

General SAT preprocessing we build on:

- Niklas Eén, Armin Biere. *Effective Preprocessing in SAT Through Variable and Clause Elimination.* SAT 2005. (SatELite; bounded variable elimination.)
- Marijn J. H. Heule, Matti Järvisalo, Armin Biere. *Efficient CNF Simplification Based on Binary Implication Graphs.* SAT 2011. Equivalent-literal substitution.
- Norbert Manthey, Marijn J. H. Heule, Armin Biere. *Automated Reencoding of Boolean Formulas.* HVC 2012. Bounded variable addition, which SBVA later generalised.
- Philip Kilby, John Slaney, Sylvie Thiébaux, Toby Walsh. *Backbones and Backdoors in Satisfiability.* AAAI 2005. The iterative assume/solve approach our backbone stripping follows.
- Yong Lai, Kuldeep S. Meel, Roland H. C. Yap. *The Power of Literal Equivalence in Model Counting.* AAAI 2021. (PreLite.)

## Vtree construction

Vtrees — the object this library produces — were introduced by Pipatsrisawat
and Darwiche as the structure underlying structured decomposability, and later
became the variable-ordering backbone of Sentential Decision Diagrams:

- Knot Pipatsrisawat, Adnan Darwiche. *New Compilation Languages Based on Structured Decomposability.* AAAI 2008.
- Adnan Darwiche. *SDD: A New Canonical Representation of Propositional Knowledge Bases.* IJCAI 2011.

---

If we have misattributed a technique or missed a debt, please open an issue.
Corrections are welcome and will be applied.
