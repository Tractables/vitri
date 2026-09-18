# Changelog

## Unreleased

- Make `VarId` a type that cannot hold 0, which names no variable: the field is private over a `NonZeroU32`, `VarId::new` and `VarId::try_from_dimacs` are the checked constructors, `VarId::get` reads the number, and `VarId::all` enumerates a variable space. `ShowSet::from_dimacs_ids` still refuses a 0, since it is where file numbers become variables; `ShowSet::from_vars` and `ShowSet::insert` no longer return `Result`.
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
