//! Independent components of a formula.
//!
//! Two clauses share a component when a chain of clauses links them through
//! shared variables. A formula that splits is one a consumer can compile a
//! piece at a time, so the split — and the extraction of one piece as a
//! formula in its own right — lives together here.

use super::{Clause, CnfFormula, Literal, VarId, union_find};

impl CnfFormula {
    /// Detect independent components via union-find over variables.
    ///
    /// Returns `None` if there is only 1 component (no benefit from decomposition).
    /// Otherwise returns clause index groups (into `self.clauses`) sorted
    /// smallest-first.
    pub fn detect_components(&self) -> Option<Vec<Vec<usize>>> {
        detect_components_in(&self.clauses, self.num_vars)
    }

    /// Extract a sub-formula for a component with contiguous variable IDs.
    ///
    /// Returns `(sub_formula, local_to_global)` where `local_to_global[local_id]`
    /// gives the original `VarId`.
    pub fn extract_component(&self, clause_indices: &[usize]) -> (CnfFormula, Vec<VarId>) {
        let mut var_set = std::collections::BTreeSet::new();
        for &ci in clause_indices {
            for lit in &self.clauses[ci].literals {
                var_set.insert(lit.var);
            }
        }

        let local_to_global: Vec<VarId> = var_set.iter().copied().collect();
        let global_to_local: std::collections::HashMap<VarId, u32> = local_to_global
            .iter()
            .enumerate()
            .map(|(i, &v)| (v, i as u32))
            .collect();

        let clauses = clause_indices
            .iter()
            .map(|&ci| {
                let lits = self.clauses[ci]
                    .literals
                    .iter()
                    .map(|lit| Literal::new(VarId(global_to_local[&lit.var]), lit.positive))
                    .collect();
                Clause::new(lits)
            })
            .collect();

        let sub = CnfFormula {
            num_vars: local_to_global.len() as u32,
            clauses,
        };
        (sub, local_to_global)
    }
}

/// The independent components of `clauses` over a universe of `num_vars`
/// variables, as groups of indices into `clauses`, smallest group first.
/// `None` when the clauses form a single component — the common case, and the
/// answer that lets a caller skip a partition it would not use.
///
/// The clause-slice form of [`CnfFormula::detect_components`], for a caller
/// holding clauses it has not wrapped in a formula: a residual part-way through
/// a search, a cofactor still being built. `num_vars` is the universe the
/// clauses are numbered over, not a count of the variables they mention.
///
/// The order is stable: groups are sorted by size and then by their smallest
/// variable, so the same clauses always split the same way. A consumer that
/// composes its per-component results in this order gets the same composition
/// every run.
pub fn detect_components_in(clauses: &[Clause], num_vars: u32) -> Option<Vec<Vec<usize>>> {
    if clauses.len() <= 1 {
        return None;
    }

    // Union-find over variables: every clause unions all its variables, so each
    // connected component (in the variable-incidence graph) ends up sharing a
    // single representative.
    let mut uf = union_find::UnionFind::new(num_vars as usize + 1);
    for clause in clauses {
        if let [first, rest @ ..] = clause.literals.as_slice() {
            for lit in rest {
                uf.union(first.var.0 as usize, lit.var.0 as usize);
            }
        }
    }

    // Bucket each clause by its first variable's representative. Empty clauses
    // (no variables to consult) all collapse into a single bucket keyed by 0.
    let mut by_rep: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    for (i, clause) in clauses.iter().enumerate() {
        let rep = clause
            .literals
            .first()
            .map_or(0, |lit| uf.find(lit.var.0 as usize));
        by_rep.entry(rep).or_default().push(i);
    }

    if by_rep.len() <= 1 {
        return None;
    }
    let mut components: Vec<Vec<usize>> = by_rep.into_values().collect();
    // Sort by (component size, then smallest variable) — smallest-variable
    // rather than smallest-clause-index keeps the order invariant under clause
    // reordering, so the same formula always splits the same way. Needed
    // because `by_rep` is a randomly-seeded `std::collections::HashMap` whose
    // iteration order varies across calls, and a consumer's graft path depends
    // on this order: it shapes the right-linear graft chain and therefore the
    // compiled diagram.
    components.sort_by_cached_key(|c| {
        let min_var = c
            .iter()
            .flat_map(|&ci| clauses[ci].literals.iter().map(|l| l.var.0))
            .min()
            .unwrap_or(u32::MAX);
        (c.len(), min_var)
    });
    Some(components)
}
