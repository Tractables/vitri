//! The formula and the vtree the scoring tests are worked out over, and the
//! one metric still spelled out separately from the fused computation. Shared
//! with `score`'s own test tree, which holds the fused computation against the
//! same hand-checkable shape.

use crate::cnf::Clause;
use crate::cnf::CnfFormula;
use crate::score::vtree_context_width_per_node;
use crate::tests::common::lit;
use crate::vtree::Vtree;
use crate::vtree::VtreeIdx;
use crate::vtree::{VarId, VtreeNode};

/// Four variables, five clauses, laid out so that the fixture vtree below
/// puts clauses at three different nodes with three different loads. Kept
/// small enough that every metric can be worked out by hand.
///
/// ```text
/// c1: v0 ∨  v1      c2: v2 ∨ v3      c3: v0 ∨  v2
/// c4: v1 ∨ ¬v3      c5: v0 ∨ ¬v1
/// ```
pub(crate) fn fixture_formula() -> CnfFormula {
    CnfFormula {
        num_vars: 4,
        clauses: vec![
            Clause::new(vec![lit(0, true), lit(1, true)]),
            Clause::new(vec![lit(2, true), lit(3, true)]),
            Clause::new(vec![lit(0, true), lit(2, true)]),
            Clause::new(vec![lit(1, true), lit(3, false)]),
            Clause::new(vec![lit(0, true), lit(1, false)]),
        ],
    }
}

/// The balanced vtree `((v0 v1) (v2 v3))` over [`fixture_formula`], built
/// node by node so the expected metrics can be derived from a known shape
/// rather than from whatever a conversion happens to return.
pub(crate) fn fixture_vtree() -> Vtree {
    let leaf = |v: u32| VtreeNode::Leaf {
        var: VarId(v),
        parent: None,
    };
    let internal = |l: u32, r: u32| VtreeNode::Internal {
        left: VtreeIdx(l),
        right: VtreeIdx(r),
        parent: None,
    };
    let nodes = vec![
        leaf(0),        // 0
        leaf(1),        // 1
        internal(0, 1), // 2  — spans {v0, v1}
        leaf(2),        // 3
        leaf(3),        // 4
        internal(3, 4), // 5  — spans {v2, v3}
        internal(2, 5), // 6  — root
    ];
    Vtree::from_nodes(nodes, VtreeIdx(6), 4)
}

/// Peak context width: the largest separator (∃-forget frontier) over all vtree
/// nodes. `2^peak` bounds the largest intermediate diagram materialised during
/// compilation. The metric projected counting minimizes (max over cuts).
///
/// Every scoring site in the crate reads the fused `VtreeScores` instead, so
/// this separate spelling exists only as the reference that field is checked
/// against — which is why it lives here and not beside the fused computation.
pub(crate) fn vtree_peak_context_width(vtree: &Vtree, formula: &CnfFormula) -> u32 {
    vtree_context_width_per_node(vtree, formula, None)
        .into_iter()
        .max()
        .unwrap_or(0)
}
