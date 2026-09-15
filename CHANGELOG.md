# Changelog

## Unreleased

- Add `vitri::request`: run a JSON-describable request and get the bundle files in memory with a summary.
- Run the budgeted Arjun stage inline unless the process is known to have one thread and keeps its children waitable.
- Publish a Linux x86_64 archive of the command-line tool with each release, bundling GMP built from its attached source.
- Build for `wasm32-unknown-emscripten` with the Arjun stage, linking GMP as side modules from the prefix `VITRI_EMSCRIPTEN_PREFIX` names.
- Add `bindings/wasm`, which builds vitri as a WebAssembly module for the browser, and publish that module with GMP's side modules and source.
- Publish a page that runs vitri in the browser at the root of the GitHub Pages site, beside the rustdoc.

## 0.2.0

- Upgrade Goatd to 0.2.1.
- Configure final refinement and bipartite lifting with `GoatdPolishing` and `GoatdLift`.
- Use bounded adaptive polishing by default; explicit legacy policies remain available.
- Configure pairwise selection weights with `PortfolioKnobs::pairwise_weighting`.
- Apply cooperative wall limits when converting adaptive refinement results.
