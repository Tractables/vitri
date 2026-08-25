use crate::cnf::CnfFormula;
use crate::decompose::td_to_vtree::*;
use crate::decompose::{GraphKind, TdBag, TreeDecomposition};
use crate::tests::common::make_formula;

/// The number of variables the in-bag affinity fixture below spans: four the
/// single bag holds, then enough beyond them to pad a hub clause past the cap.
const AFFINITY_NUM_VARS: u32 = 53;

/// The affinity fixture's clauses, optionally with one hub clause of `hub_len`
/// literals appended.
///
/// The short clauses put variable 0 with 1 three times, with 2 twice and with 3
/// once, and leave 1, 2 and 3 sharing no clause with each other. That makes the
/// greedy in-bag walk start at 0 (its co-occurrence total is the largest), step
/// to 1, and then choose between 2 and 3 on a tie it breaks toward the later
/// one. The hub clause names 1 and 2 — so if it reaches the co-occurrence graph
/// it breaks that tie the other way — and pads itself out with variables the
/// bag does not hold, which is what a hub clause looks like.
fn affinity_formula(hub_len: Option<usize>) -> CnfFormula {
    let mut clauses: Vec<Vec<i32>> = vec![
        vec![1, 2],
        vec![1, 2],
        vec![1, 2],
        vec![1, 3],
        vec![1, 3],
        vec![1, 4],
    ];
    if let Some(len) = hub_len {
        let mut hub = vec![2, 3];
        hub.extend(5..(3 + len as i32));
        assert_eq!(hub.len(), len, "hub clause built to the wrong length");
        clauses.push(hub);
    }
    make_formula(AFFINITY_NUM_VARS, clauses)
}

/// The variable at each leaf, bottom-up, of the vtree `formula` converts to over
/// ONE bag holding variables 0..=3, read under the affinity fold.
///
/// The decomposition is written out here rather than decomposed from the
/// formula, so the clause set is the only input that differs between calls and
/// the in-bag ordering is the only thing that can act on it. The reading is
/// named in full, so the conversion builds that one tree rather than searching
/// for a cheaper one.
fn affinity_leaf_order(formula: &CnfFormula) -> Vec<u32> {
    let td = TreeDecomposition {
        kind: GraphKind::Primal,
        num_vars: AFFINITY_NUM_VARS,
        bags: vec![TdBag {
            id: 0,
            vertices: vec![0, 1, 2, 3],
        }],
        adj: vec![vec![]],
    };
    let reading = Reading {
        root: Some(Root::First),
        place: Some(Place::Deep),
        fold: Some(Fold::Affinity),
    };
    td_to_vtree_reading(&td, AFFINITY_NUM_VARS, reading, Some(formula), None)
        .leaf_bottomup()
        .map(|(_, v)| v.0)
        .collect()
}

/// A clause too long to belong in the co-occurrence graph does not order a bag.
///
/// The graph the in-bag affinity ordering ranks by is the same primal graph the
/// rest of the crate reads, which leaves out clauses over
/// `COOC_CLAUSE_LEN_CAP`: one hub clause naming half a formula contributes a
/// clique that says nothing about which variables belong together, and would
/// otherwise outvote every short clause in every bag it touches.
///
/// The second assertion is the control. Without it the first would also hold if
/// the fixture could not see a hub clause at all, and the test would pass for
/// the wrong reason.
#[test]
fn a_clause_over_the_length_cap_does_not_order_a_bag() {
    let cap = crate::decompose::td_parse::COOC_CLAUSE_LEN_CAP;

    let without_hub = affinity_leaf_order(&affinity_formula(None));
    let over_cap = affinity_leaf_order(&affinity_formula(Some(cap + 1)));
    let at_cap = affinity_leaf_order(&affinity_formula(Some(cap)));

    assert_eq!(
        over_cap, without_hub,
        "a clause longer than the cap reached the co-occurrence graph the in-bag \
         affinity ordering ranks by"
    );
    assert_ne!(
        at_cap, without_hub,
        "control: the same clause one literal shorter must still reach it"
    );
}
