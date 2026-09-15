# Changelog

## 0.2.0

- Upgrade Goatd to 0.2.1.
- Configure final refinement and bipartite lifting with `GoatdPolishing` and `GoatdLift`.
- Use bounded adaptive polishing by default; explicit legacy policies remain available.
- Configure pairwise selection weights with `PortfolioKnobs::pairwise_weighting`.
- Apply cooperative wall limits when converting adaptive refinement results.
