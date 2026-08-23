//! Count-preserving BCP simplification for the `pmc` and `pwmc` chains.
//!
//! Runs unit propagation (BCP) on the formula to fixpoint, id-preservingly:
//! variable ids are never renumbered, so the projected chain's `show_set` and weight
//! tables stay keyed to original DIMACS ids.
//!
//! ## Why forced show vars are the crux
//!
//! After BCP a forced variable appears in no remaining clause, so it is absent
//! from the vtree — exactly like a variable that never occurred. But the two
//! contribute differently to a *projected* count:
//!
//!   * a genuinely-free show var (never constrained) → factor 2 (PMC) /
//!     (w⁺+w⁻) (PWMC): both polarities extend to a model;
//!   * a *forced* show var (pinned by a unit cascade) → factor 1 (PMC) /
//!     w_forced-polarity (PWMC): only one polarity extends.
//!
//! A downstream free-show correction counts every absent show var as free, so
//! this module RE-PINS each forced show var with a unit clause: the var
//! occurs again, the compile weights it at exactly factor 1 / its forced
//! polarity, and no downstream correction is owed. Forced *hidden* vars stay
//! removed (correctly ∃-absorbed) — only show vars need the re-pin.
//!
//! A show var absent because its clauses were *satisfied* rather than
//! unit-forced is genuinely free and stays uncounted here: `propagate` only
//! reports unit-forced literals, so no unit is re-pinned for it.

use crate::cnf::{CnfFormula, ShowMask};

/// Result of count-preserving BCP over the projected chain.
pub(super) struct BcpResult {
    /// Reduced formula, variable ids preserved, with a unit clause re-pinning
    /// every forced show var. If UNSAT, contains a single empty clause and
    /// `unsat` is set.
    pub(super) formula: CnfFormula,
    /// True iff BCP derived the empty clause (formula is UNSAT).
    pub(super) unsat: bool,
}

/// Run BCP to fixpoint, re-pinning forced counted-vars.
///
/// A projected instance always declares a show set before reaching this module,
/// so there is no "no show set" case to handle here — `show` is the free-show
/// universe outright.
pub(super) fn bcp_simplify(formula: &CnfFormula, show: &ShowMask) -> BcpResult {
    use crate::cnf::Clause;

    let (mut clauses, forced) =
        crate::preprocess::unit_propagation::propagate(&formula.clauses, formula.num_vars);
    let unsat = crate::cnf::contains_empty_clause(&clauses);

    if !unsat {
        for &l in forced.iter().filter(|l| show.is_show(l.var)) {
            clauses.push(Clause::new(vec![l]));
        }
    }

    BcpResult {
        formula: CnfFormula {
            num_vars: formula.num_vars,
            clauses,
        },
        unsat,
    }
}
