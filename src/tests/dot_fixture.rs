//! The tree the DOT tests render, and how they read one node's declaration
//! back out of the rendering. Shared with `dot`'s own test tree, which renders
//! the same shape against constants that module keeps to itself.

use crate::cnf::CnfFormula;
use crate::tests::common::clause_dimacs;
use crate::vtree::Vtree;

/// `balanced(4)`: leaves 0..3 hold variables 1..4 in order, internal 4 joins
/// leaves 0 and 1, internal 5 joins leaves 2 and 3, root 6 joins 4 and 5.
/// Small enough that every clause LCA is checkable by eye.
pub(crate) fn fixture() -> (Vtree, CnfFormula) {
    let formula = CnfFormula {
        num_vars: 4,
        clauses: vec![
            clause_dimacs(&[1, 2]),  // meets at internal 4
            clause_dimacs(&[-1, 2]), // meets at internal 4
            clause_dimacs(&[3, 4]),  // meets at internal 5
            clause_dimacs(&[1, 4]),  // spans both halves: meets at the root
            clause_dimacs(&[2]),     // a unit clause sits on its own leaf
        ],
    };
    (Vtree::balanced(4), formula)
}

/// The line declaring node `n`, so an assertion can pin a whole attribute
/// list instead of a substring that could match anywhere.
pub(crate) fn node_line(dot: &str, n: u32) -> &str {
    let head = format!("v{n} [");
    dot.lines()
        .find(|l| l.trim_start().starts_with(&head))
        .unwrap_or_else(|| panic!("no declaration of v{n} in:\n{dot}"))
}
