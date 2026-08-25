use crate::cnf::CnfFormula;
use crate::decompose::td_to_vtree::*;
use crate::decompose::{GraphKind, TdBag, TreeDecomposition};
use crate::tests::common::make_formula;
use crate::vtree::{VarId, Vtree, VtreeIdx};

/// The number of variables the fixture below spans: five the bags hold, then
/// enough beyond them to pad a hub clause past the cap.
const CAP_NUM_VARS: u32 = 60;

/// The cap fixture's decomposition: a root bag and two children of equal depth,
/// both holding variable 0.
///
/// Variable 0 is therefore at the same depth in two bags, which is the tie
/// `Place::Deep` breaks by clause co-occurrence — bag 1 if 0 shares clauses
/// with 3 and 4, bag 2 otherwise, since an exact tie keeps the bag the walk
/// reached last.
fn cap_td() -> TreeDecomposition {
    TreeDecomposition {
        kind: GraphKind::Primal,
        num_vars: CAP_NUM_VARS,
        bags: vec![
            TdBag {
                id: 0,
                vertices: vec![0, 1, 2],
            },
            TdBag {
                id: 1,
                vertices: vec![0, 3, 4],
            },
            TdBag {
                id: 2,
                vertices: vec![0, 1, 2],
            },
        ],
        adj: vec![vec![1, 2], vec![0], vec![0]],
    }
}

/// One clause of `len` literals naming variables 0, 3 and 4, padded out with
/// variables no bag holds — which is what a hub clause looks like.
///
/// It is the ONLY clause putting 0 with 3 and 4, so whether the tie-break sees
/// it is the whole difference between the two bags.
fn hub_formula(len: usize) -> CnfFormula {
    let mut hub = vec![1, 4, 5];
    hub.extend(6..(3 + len as i32));
    assert_eq!(hub.len(), len, "hub clause built to the wrong length");
    make_formula(CAP_NUM_VARS, vec![hub])
}

/// Whether variable 0 was placed in the bag holding 3 and 4 — which is the bag
/// the tie-break picks when it can see the hub clause.
///
/// Read off the tree rather than off the assignment, which is internal: a bag's
/// variables are the leaves of one subtree, so 0 landing in bag 1 means the
/// join of 0 and 3 stays inside a subtree bag 2's variables are absent from.
///
/// The decomposition is written out here rather than decomposed from the
/// formula, so the clause set is the only input that differs between calls. The
/// reading is named in full, so the conversion builds that one tree rather than
/// searching for a cheaper one.
fn placed_with_its_partners(formula: &CnfFormula) -> bool {
    let reading = Reading {
        root: Some(Root::First),
        place: Some(Place::Deep),
        binarize: Some(Binarization::Balanced),
    };
    let vtree = td_to_vtree_reading(&cap_td(), CAP_NUM_VARS, reading, Some(formula), None);
    let join = vtree.lca(vtree.leaf_of(VarId(0)), vtree.leaf_of(VarId(3)));
    !leaves_under(&vtree, join).contains(&1)
}

/// The variables at the leaves under `idx`.
fn leaves_under(vtree: &Vtree, idx: VtreeIdx) -> Vec<u32> {
    let mut out = Vec::new();
    let mut stack = vec![idx];
    while let Some(node) = stack.pop() {
        if vtree.node(node).is_leaf() {
            out.push(vtree.leaf_var(node).0);
        } else {
            let (l, r) = vtree.children(node);
            stack.push(l);
            stack.push(r);
        }
    }
    out
}

/// A clause too long to belong in the co-occurrence graph does not place a
/// variable.
///
/// The graph the deep-placement tie-break ranks bags by leaves out clauses over
/// `COOC_CLAUSE_LEN_CAP`: one hub clause naming half a formula contributes a
/// clique that says nothing about which variables belong together, and would
/// otherwise outvote every short clause in every bag it touches.
///
/// The second assertion is the control. Without it the first would also hold if
/// the fixture could not see a hub clause at all, and the test would pass for
/// the wrong reason.
#[test]
fn a_clause_over_the_length_cap_does_not_place_a_variable() {
    let cap = crate::decompose::td_parse::COOC_CLAUSE_LEN_CAP;

    assert!(
        !placed_with_its_partners(&hub_formula(cap + 1)),
        "a clause longer than the cap reached the co-occurrence graph the \
         deep-placement tie-break ranks by"
    );
    assert!(
        placed_with_its_partners(&hub_formula(cap)),
        "control: the same clause one literal shorter must still reach it"
    );
}
