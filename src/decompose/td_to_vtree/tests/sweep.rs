//! The root/ordering sweep under a wall-clock bound.

use std::time::{Duration, Instant};

use crate::cnf::CnfFormula;
use crate::decompose::TreeDecomposition;
use crate::decompose::td_to_vtree::td_to_vtree_best;
use crate::tests::common::{make_formula, make_td};

/// A star decomposition: a hub bag holding variable 0, and one leaf bag per
/// other variable. Eight leaf bags is past the point where the sweep switches
/// from converting every root under every ordering to screening the roots
/// first, so both halves of it run.
fn star_td() -> (TreeDecomposition, CnfFormula) {
    let num_vars = 9;
    let mut bags = vec![vec![0u32]];
    let mut edges = Vec::new();
    for v in 1..num_vars {
        bags.push(vec![0, v]);
        edges.push((0usize, v as usize));
    }
    let clauses: Vec<Vec<i32>> = (1..num_vars).map(|v| vec![1, v as i32 + 1]).collect();
    (
        make_td(bags, edges, num_vars),
        make_formula(num_vars, clauses),
    )
}

/// A sweep handed a deadline that has already passed still returns a vtree over
/// every variable.
///
/// The bound governs how many conversions get scored, never whether any does.
/// The caller is a construction that has just spent its whole budget building a
/// decomposition, so a refusal here would throw that decomposition away exactly
/// when the wall around it starts working.
#[test]
fn an_expired_sweep_deadline_still_returns_a_vtree_over_every_variable() {
    let (td, formula) = star_td();
    let vtree = td_to_vtree_best(
        &td,
        formula.num_vars,
        &formula,
        1.0,
        Some(Instant::now() - Duration::from_secs(1)),
    );
    assert_eq!(
        vtree.num_leaves(),
        formula.num_vars,
        "an expired sweep deadline returned a partial vtree",
    );
}

/// A sweep deadline the conversion never reaches leaves the winner unchanged.
#[test]
fn a_sweep_deadline_the_conversion_never_reaches_selects_the_unbounded_winner() {
    let (td, formula) = star_td();
    let unbounded = td_to_vtree_best(&td, formula.num_vars, &formula, 1.0, None);
    let bounded = td_to_vtree_best(
        &td,
        formula.num_vars,
        &formula,
        1.0,
        Some(Instant::now() + Duration::from_secs(3600)),
    );
    assert_eq!(
        bounded.to_vtree_text(),
        unbounded.to_vtree_text(),
        "a bound the sweep never reaches changed the vtree it selected",
    );
}
