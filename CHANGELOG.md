# Changelog

## Unreleased

- Add `vitri::request`: run a JSON-describable request and get the bundle files in memory with a summary.
- Run the budgeted Arjun stage inline unless the process is known to have one thread and keeps its children waitable.
- Publish a Linux x86_64 archive of the command-line tool with each release, bundling GMP built from its attached source.
- Add Python bindings in `bindings/python`: `vitri.prepare` returns the bundle and summary of `vitri::request::prepare`.

## 0.2.0

- Upgrade Goatd to 0.2.1.
- Configure final refinement and bipartite lifting with `GoatdPolishing` and `GoatdLift`.
- Use bounded adaptive polishing by default; explicit legacy policies remain available.
- Configure pairwise selection weights with `PortfolioKnobs::pairwise_weighting`.
- Apply cooperative wall limits when converting adaptive refinement results.
