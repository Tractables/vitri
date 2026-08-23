//! Where a load lands on the colour scale, pinned against the scale's own
//! constant rather than the hex code it happens to produce.

use crate::cnf::CnfFormula;
use crate::dot::*;
use crate::tests::common::clause_dimacs;
use crate::tests::dot_fixture::node_line;
use crate::vtree::Vtree;
use crate::vtree::VtreeIdx;

/// A flat load — every loaded node carrying the same count — sits low on
/// the scale instead of painting the whole tree the maximum colour.
#[test]
fn a_flat_load_renders_calm_not_maximally_hot() {
    let formula = CnfFormula {
        num_vars: 4,
        // One clause each at internals 4 and 5.
        clauses: vec![clause_dimacs(&[1, 2]), clause_dimacs(&[3, 4])],
    };
    let vtree = Vtree::balanced(4);
    let ann = annotate_from_cnf(&vtree, &formula, None);
    assert_eq!(ann.heat(VtreeIdx(4)), Some(FLAT_LOAD_HEAT));
    assert_eq!(ann.heat(VtreeIdx(5)), Some(FLAT_LOAD_HEAT));
    assert_eq!(
        ann.heat(VtreeIdx(6)),
        Some(0.0),
        "the unloaded root stays at the bottom"
    );
    let dot = vtree_to_dot(&vtree, Some(&ann));
    assert!(
        !node_line(&dot, 4).contains("#800026"),
        "a uniform tree must not render at the alarm end of the scale",
    );
}
