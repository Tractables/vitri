# Changelog

## Unreleased

- Add `vitri::request`: run a JSON-describable request and get the bundle files in memory with a summary.
- Run the budgeted Arjun stage inline unless the process is known to have one thread and keeps its children waitable.
- Add a C ABI in `bindings/c`: shared and static libraries over `vitri::request`, with a generated header.

## 0.2.0

- Upgrade Goatd to 0.2.1.
- Configure final refinement and bipartite lifting with `GoatdPolishing` and `GoatdLift`.
- Use bounded adaptive polishing by default; explicit legacy policies remain available.
- Configure pairwise selection weights with `PortfolioKnobs::pairwise_weighting`.
- Apply cooperative wall limits when converting adaptive refinement results.
