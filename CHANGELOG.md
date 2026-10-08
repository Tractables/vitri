# Changelog

## Unreleased

- Build every component with `force` when a portfolio build of a function-preserving formula (`--mode compile`) splits into components and the formula has at least 1000 variables and at least 4.5 clauses per variable. Over function-preserving reductions of the 780 solvable track-1 instances of the Model Counting Competitions, compiled into tree decision diagrams at two minutes, the 38 formulas the rule admits compiled 17 more under `force` and none fewer, in less total time on those both compiled; outside the rule the portfolio compiles far more. Counting-mode reductions are unaffected: there the same rule gained nothing. `SelectionCtx::preserves_function` carries the condition; the full pipeline sets it from the run's mode and construction-only calls default to `false`.
- Read the clock once every 1024 decisions, instead of at every decision, when Arjun's search for defined variables checks the deadline. The search makes millions of decisions, and on a machine whose clock has no fast path each read takes about a microsecond, so the check could more than double the time independent-support minimization takes, run it into its deadline and lose the rest of the reduction. Once a read finds the deadline passed, every candidate not yet tested is kept as before.
- Skip Arjun's oracle in the `pwmc` pre-pass by default: `OracleCaps::weighted_projected` and `VITRI_PWMC_ARJUN_ORACLE_MAX_VARS` default to 0. On weighted projected instances the oracle was the largest part of the pre-pass; skipping it left every count unchanged and cut total time. The `pmc` default is unchanged.
- Stop Arjun's independent-support minimization at the deadline when it falls inside the search for defined variables. That search tests every candidate inside one solve, and the deadline was checked only between solves, so a budget that ran out during it ran on until the solve had tested every candidate, or until the process holding it was killed and its reduction lost. Past the deadline each candidate not yet tested is now kept in the support, as one that runs out of conflicts already was.
- Stop projected variable elimination at the run's deadline. Under `pmc` and `pwmc`, the bounded elimination of hidden variables after show-frozen strengthening ignored the deadline, so a large projected formula could run far past the run's wall. It now stops once the deadline passes and keeps the eliminations it finished; each is exact existential quantification, so the projected count is unchanged and only fewer hidden variables are removed. Resolvents are merged from the two sorted clauses instead of sorted from their concatenation, with identical output, so a run without a deadline reduces exactly as before. `projection::eliminate_hidden` takes no deadline and still runs to its fixpoint.
- Bound the default goatd polishing stage by work instead of by a hundred milliseconds of real time, so the vtree a construction polishes to no longer depends on how fast or how loaded the machine is. The stage runs on the construction's work clock, arming it for the stage when nothing else has, reads it between the kernel's scheduling steps as well as between proposals, and stops after 110 milliseconds of it, about what the real-time bound bought on a typical instance. `GoatdPolishing::with_wall_limit` is renamed `GoatdPolishing::with_work_limit`.
- Charge a reading of a tree decomposition 400 work units for each variable, adjacency entry and literal it covers, instead of one. That is about what a reading costs against the units the decomposition kernels charge, so a `ConstructionBudget::Deterministic` budget spent on readings lasts about as long as the wall it was sized for rather than far longer, and a deterministic construction considers fewer readings for the same budget.
- Probe for backbones and literal equivalences over the variables a clause mentions only. A declared variable that no clause mentions was still loaded into the probing solver, which decided it on every probe and read it back with every model, and one numbered above every mentioned variable was probed as a backbone candidate. Such a variable is never a backbone nor equivalent to another, so it is now counted as flippable without a probe, and the reported probe count no longer includes the probes once spent on it.
- Weigh a literal no `c p weight` line names at 1 under `pwmc` when the file names the other literal of its variable. The Arjun reduction was handed the named literal alone and weighed the other one `1 - w`, so the weight lift and the reduced weights were wrong.
- Read a missing complementary weight the way the Model Counting Competition format defines it, in every mode: when `c p weight` lines give one literal of a variable a weight `0 < w < 1` and the other none, the other weighs `1 - w`; a lone weight outside that range is refused, naming the literal; a variable with no weight line weighs 1 both ways.
- Add `spec::is_structural_spec`, which says whether a spec builds from the formula's graph, so a consumer compiling components separately makes the same call the split builder does; and make `CnfFormula::component_vars` public, the sorted variables of a clause group without the sub-formula.
- Read `root=centroid` as the centroid alone. A named root other than `leaf` also enumerated the first bag, so the conversion searched both, and one with no formula to score against rooted at the first bag.
- Read `VITRI_SCORE_AGG_MARGIN=none` whatever its case, like every other word-valued variable.
- Show the vtree drawing on the browser page. The empty-state panel has `display: flex` in the stylesheet, which beat the `hidden` attribute and left the panel over the canvas; `[hidden]` is now an author rule. A check in the module workflow reads the page against its stylesheet for that.
- Make `VarId` a type that cannot hold 0, which names no variable: the field is private over a `NonZeroU32`, `VarId::new` and `VarId::try_from_dimacs` are the checked constructors, `VarId::get` reads the number, and `VarId::all` enumerates a variable space. `ShowSet::from_dimacs_ids` still refuses a 0, since it is where file numbers become variables; `ShowSet::from_vars` and `ShowSet::insert` no longer return `Result`.
- Make `CnfFormula`'s fields private, so a formula cannot carry a clause naming a variable its
  declared count does not cover. `CnfFormula::new` is the checked constructor, `num_vars()` and
  `clauses()` read the parts, and `into_clauses` takes the clauses out.
- Parse a file whose header declares no variables, such as `p cnf 0 1` over the empty clause, instead of reporting its problem line as missing.
- Honour the construction deadline in bisection — the dials a bisection runs under carry the caller's deadline — and spend the smaller of the soft ceiling and the time left on a goatd elimination pass.
- Report an empty tree decomposition or a formula with no variables as `VitriError::Input` from `td_to_vtree`, where it asserted.
- Label `.dot` nodes by `topo_pos`, the numbering the `.vtree` file beside them uses.
- Refuse a `preprocess.json` or `components.json` whose `format` tag, or whose rank metric, this version does not understand.
- Check a `VtreeBuild` against its components before writing anything, so a refused build leaves no partial bundle behind.
- Number variables from 1: a `VarId` is the DIMACS variable of the same number, so `VarId::to_dimacs`, `VarId::from_dimacs` and `Literal`'s conversions carry no offset, and `VarId::idx` / `VarId::from_idx` are the array-index conversions. `ShowSet::from_zero_based`, `ShowSet::as_zero_based` and `ShowSet::to_dimacs` are replaced by `ShowSet::from_vars` and `ShowSet::as_dimacs`. File formats are unchanged.
- Add `vitri::request`: run a JSON-describable request and get the bundle files in memory with a summary.
- Run the budgeted Arjun stage inline unless the process is known to have one thread and keeps its children waitable.
- Add a C ABI in `bindings/c`: shared and static libraries over `vitri::request`, with a generated header.
- Publish a Linux x86_64 archive of the command-line tool with each release, bundling GMP built from its attached source.
- Add Python bindings in `bindings/python`: `vitri.prepare` returns the bundle and summary of `vitri::request::prepare`.
- Build for `wasm32-unknown-emscripten` with the Arjun stage, linking GMP as side modules from the prefix `VITRI_EMSCRIPTEN_PREFIX` names.
- Add `bindings/wasm`, which builds vitri as a WebAssembly module for the browser, and publish that module with GMP's side modules and source.
- Publish a page that runs vitri in the browser at the root of the GitHub Pages site, beside the rustdoc.
- Add `--version` to the command-line tool, which prints the same version string the C, Python and wasm surfaces hand out.
- Package `docs/showcase/*.cnf`, so the published crate carries the instance `docs/showcase.md` runs its commands on.
- Report the emitted vtree's scores on `request::VtreeSummary::scores`: the same five numbers selection ranked it on, so a caller reads them without rescoring the tree.
- Show those scores on the browser page, and link the page from the README.
- Remove `GoatdPolishing::with_separator`, the `GoatdSeparatorConfig` re-export it existed for, and `score::check_score_env`. Nothing called any of them; the argv-time environment read `check_score_env` was named for still happens in `PortfolioKnobs::with_env_defaults`.
- Publish GNU's detached signature for the GMP tarball beside it on a release, and record in `docs/THIRD-PARTY.md` where the sources of a binary distribution are and that the Rust standard library is linked into it.

## 0.2.0

- Upgrade Goatd to 0.2.1.
- Configure final refinement and bipartite lifting with `GoatdPolishing` and `GoatdLift`.
- Use bounded adaptive polishing by default; explicit legacy policies remain available.
- Configure pairwise selection weights with `PortfolioKnobs::pairwise_weighting`.
- Apply cooperative wall limits when converting adaptive refinement results.
