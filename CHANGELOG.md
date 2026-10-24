# Changelog

## 0.3.0

### Migrating from 0.2

- Variables are one-based DIMACS IDs in `1..=i32::MAX`. Replace tuple
  construction with `VarId::new`, use `get` for the ID and `idx` for an array
  index. `VarId::from_idx` converts an array index back to an ID.
- Build formulas with `CnfFormula::new`; use `num_vars()`, `clauses()` and
  `into_clauses()` in place of direct field access.
- Replace `ShowSet::from_zero_based` with `from_vars` after converting IDs,
  and `as_zero_based` / `to_dimacs` with `as_dimacs`. `from_vars` and `insert`
  no longer return `Result`.
- Replace `GoatdPolishing::with_wall_limit` with `with_work_limit`; the limit
  measures construction work rather than elapsed wall time.
- Use `Vtree::try_linear_from_order` for caller-supplied orders. The existing
  `linear_from_order` requires a nonempty order without repeated variables.
- `GoatdPolishing::with_separator`, `GoatdSeparatorConfig` and
  `score::check_score_env` were removed. Read environment defaults through
  `SelectionCtx::with_env_defaults`.
- `FrontendRetryConfig` has a new field, `arjun_skip_monotone`. End a
  struct literal with `..FrontendRetryConfig::default()`.

### Added

- Python bindings, `pyvitri` on PyPI: `vitri.prepare` returns the bundle and
  summary of `vitri::request::prepare`.
- A C ABI in `bindings/c`: shared and static libraries over `vitri::request`,
  with a generated header.
- `bindings/wasm`, and a page at the root of the GitHub Pages site that runs
  vitri in the browser and shows the vtree and its scores.
- `vitri::request`: one run as plain values or JSON, with the bundle files
  returned in memory and a summary that includes the emitted vtree's scores.
- A Linux x86-64 archive of the command-line tool on each release, bundling
  GMP, with GMP's source and GNU's signature attached.
- `--version`, which prints the version string the C, Python and wasm
  surfaces report.
- `ArjunOptions::skip_monotone` (`VITRI_ARJUN_SKIP_MONOTONE`) skips the Arjun
  stage under `mc` on a formula that is monotone up to renaming;
  `FrontendRetryConfig::arjun_skip_monotone` lets a retry run it.
- `ArjunBudget::derived_duration`, `spec::is_structural_spec` and a public
  `CnfFormula::component_vars`.
- The published crate carries the score-vtree example and the showcase
  instance.

### Changed

- Variables are numbered from 1, `VarId` cannot hold 0 and `CnfFormula`'s
  fields are private; see *Migrating from 0.2*.
- Components that are the same up to a renaming of their variables share one
  vtree build.
- A portfolio build of a large, clause-dense `--mode compile` formula that
  splits into components, none of them dominant, builds every component with
  `force` (`SelectionCtx::preserves_function`).
- Under a deadline, a portfolio build bounds the second FlowCutter view and
  goatd by the time the FlowCutter views already took, and on a formula with
  many clauses per variable it runs `flowcutter-primal` first.
- goatd's default polishing is bounded by work instead of wall time, so its
  tree no longer depends on machine load.
- `ConstructionBudget::Deterministic` charges a tree-decomposition reading
  about what it costs, so a deterministic budget lasts about as long as the
  wall it was sized for.
- Arjun's independent-support minimization leaves the last quarter of Arjun's
  budget to the elimination after it, and ends the stage with no reduction when
  it has made no progress partway through.
- `pwmc` skips Arjun's oracle by default (`OracleCaps::weighted_projected`).
- A weight given on one literal only is read as the Model Counting Competition
  format defines it: the other literal weighs `1 - w` when `0 < w < 1`, and any
  other lone weight is refused.
- The budgeted Arjun stage runs in a child process only when the process has
  one thread and keeps its children waitable; otherwise it runs inline.
- `root=centroid` roots at the centroid alone.

### Faster

- Backbone probing asks again about a candidate whose capped probe stopped
  undecided, and equivalence probing stops each probe at a conflict limit and
  ends at the second undecided one.
- Probing skips variables that no clause mentions.
- Equivalent-literal detection, unit propagation and backbone stripping cost
  in proportion to the clauses, not the declared variable count.
- The second round of CaDiCaL clause simplification runs only when the first
  exposed a new equivalence.
- Arjun's search for defined variables reads the clock once every 1024
  decisions.
- Component-cache keys are computed under a work bound.

### Fixed

- Arjun's independent-support minimization and projected variable
  elimination stop at the deadline instead of running past it.
- Bisection honours the construction deadline.
- A header that declares no variables, such as `p cnf 0 1`, parses.
- `td_to_vtree` returns `VitriError::Input` for an empty decomposition or a
  formula with no variables instead of asserting.
- `.dot` node labels use the numbering of the `.vtree` file beside them.
- A `preprocess.json` or `components.json` whose `format` tag or rank metric
  this version does not know is refused, and a refused `VtreeBuild` writes no
  partial bundle.
- Variable counts outside the DIMACS range, invalid peak tolerances,
  mismatched preprocessing metadata and wrong export leaves are refused.
- `VITRI_SCORE_AGG_MARGIN=none` is read whatever its case.
- The locked random-number dependency includes its SSE2 compatibility fix.
- The browser page shows the vtree drawing.

## 0.2.0

- Upgrade Goatd to 0.2.1.
- Configure final refinement and bipartite lifting with `GoatdPolishing` and `GoatdLift`.
- Use bounded adaptive polishing by default; explicit legacy policies remain available.
- Configure pairwise selection weights with `PortfolioKnobs::pairwise_weighting`.
- Apply cooperative wall limits when converting adaptive refinement results.
