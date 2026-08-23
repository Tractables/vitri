use crate::cnf::CnfFormula;
use crate::cnf::{Reduced, ShowMask, ShowSet};
use crate::preprocess::count_preserve::*;
use crate::tests::common::make_formula;

/// The show set `vars` (0-based) over a formula of `num_vars` variables.
fn show(num_vars: u32, vars: &[u32]) -> ShowMask {
    ShowSet::<Reduced>::from_zero_based(vars.iter().copied()).mask(num_vars)
}

/// The unit clauses of a residual formula, as `(0-indexed var, polarity)`.
/// Re-pinning is the observable of this module, so every test reads it here.
fn units(f: &CnfFormula) -> Vec<(usize, bool)> {
    let mut u: Vec<(usize, bool)> = f
        .clauses
        .iter()
        .filter(|c| c.literals.len() == 1)
        .map(|c| (c.literals[0].var.0 as usize, c.literals[0].positive))
        .collect();
    u.sort_unstable();
    u
}

#[test]
fn no_units_leaves_formula_untouched() {
    // (x1 ∨ x2) ∧ (¬x2 ∨ x3): no unit clauses, so nothing propagates and
    // nothing is re-pinned.
    let f = make_formula(3, vec![vec![1, 2], vec![-2, 3]]);
    let r = bcp_simplify(&f, &show(3, &[0, 1, 2]));
    assert!(!r.unsat);
    assert_eq!(r.formula.clauses.len(), 2);
    assert!(units(&r.formula).is_empty(), "got {:?}", units(&r.formula));
}

#[test]
fn forced_show_var_is_re_pinned_with_its_polarity() {
    // Unit (x1) forces x1=true; the cascade shortens (¬x1∨x2) to (x2), forcing
    // x2=true. Both are show vars, so both come back as units — that is what
    // keeps them at factor 1 instead of being counted free.
    let f = make_formula(3, vec![vec![1], vec![-1, 2], vec![2, 3]]);
    let r = bcp_simplify(&f, &show(3, &[0, 1, 2]));
    assert!(!r.unsat);
    assert_eq!(units(&r.formula), vec![(0, true), (1, true)]);
    // (x2 ∨ x3) was satisfied by the cascade, so the two re-pins are the whole
    // residual — x3 is left genuinely free, as it should be.
    assert_eq!(r.formula.clauses.len(), 2);
}

#[test]
fn forced_projected_var_is_not_re_pinned() {
    // Same cascade, but x1 and x2 are projected (hidden). A forced projected
    // var is ∃-absorbed — weight 1, contributes nothing — so it stays removed.
    let f = make_formula(3, vec![vec![1], vec![-1, 2], vec![2, 3]]);
    let r = bcp_simplify(&f, &show(3, &[2])); // only x3 is show
    assert!(!r.unsat);
    assert!(r.formula.clauses.is_empty(), "got {:?}", r.formula.clauses);
}

#[test]
fn empty_show_set_re_pins_nothing() {
    // The show set IS the free-show universe: an empty one owes no re-pin,
    // however many variables the cascade forces.
    let f = make_formula(2, vec![vec![1], vec![-1, 2]]);
    let r = bcp_simplify(&f, &show(2, &[]));
    assert!(!r.unsat);
    // ...and the propagation itself still happened: both clauses are gone.
    assert!(r.formula.clauses.is_empty(), "got {:?}", r.formula.clauses);
}

#[test]
fn conflict_marks_unsat() {
    let f = make_formula(1, vec![vec![1], vec![-1]]);
    let r = bcp_simplify(&f, &show(1, &[0]));
    assert!(r.unsat);
}

#[test]
fn unsat_result_carries_no_re_pins() {
    // The re-pin loop is skipped on UNSAT: the residual is the single empty
    // clause the caller's degenerate-residual check looks for, nothing else.
    let f = make_formula(2, vec![vec![1], vec![-1]]);
    let r = bcp_simplify(&f, &show(2, &[0, 1]));
    assert!(r.unsat);
    assert!(units(&r.formula).is_empty(), "got {:?}", units(&r.formula));
}
