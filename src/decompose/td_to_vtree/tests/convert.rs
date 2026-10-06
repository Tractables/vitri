use crate::cnf::CnfFormula;
use crate::decompose::TreeDecomposition;
use crate::decompose::td_to_vtree::*;
use crate::tests::common::{make_formula, make_td};
use crate::vtree::{VarId, Vtree, VtreeIdx};

/// The number of variables the fixture below spans: five the bags hold, then
/// enough beyond them to pad a hub clause past the cap.
const CAP_NUM_VARS: u32 = 60;

/// The cap fixture's decomposition: a root bag and two children of equal depth,
/// both holding vertex 0, which is variable 1.
///
/// Variable 1 is therefore at the same depth in two bags, which is the tie
/// `Place::Deep` breaks by clause co-occurrence — bag 1 if 1 shares clauses
/// with 4 and 5 (vertices 3 and 4), bag 2 otherwise, since an exact tie keeps
/// the bag the walk reached last.
fn cap_td() -> TreeDecomposition {
    make_td(
        vec![vec![0, 1, 2], vec![0, 3, 4], vec![0, 1, 2]],
        vec![(0, 1), (0, 2)],
        5,
    )
}

/// One clause of `len` literals naming variables 1, 4 and 5, padded out with
/// variables no bag holds — which is what a hub clause looks like.
///
/// It is the ONLY clause putting 1 with 4 and 5, so whether the tie-break sees
/// it is the whole difference between the two bags.
fn hub_formula(len: usize) -> CnfFormula {
    let mut hub = vec![1, 4, 5];
    hub.extend(6..(3 + len as i32));
    assert_eq!(hub.len(), len, "hub clause built to the wrong length");
    make_formula(CAP_NUM_VARS, vec![hub])
}

/// Whether variable 1 was placed in the bag holding 4 and 5 — which is the bag
/// the tie-break picks when it can see the hub clause.
///
/// Read off the tree rather than off the assignment, which is internal: a bag's
/// variables are the leaves of one subtree, so 1 landing in bag 1 means the
/// join of 1 and 4 stays inside a subtree bag 2's variables are absent from.
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
    let vtree = td_to_vtree_reading(&cap_td(), CAP_NUM_VARS, reading, Some(formula), None)
        .expect("the fixture decomposition covers the fixture formula");
    let join = vtree.lca(
        vtree.leaf_of(VarId::from_dimacs(1)),
        vtree.leaf_of(VarId::from_dimacs(4)),
    );
    !leaves_under(&vtree, join).contains(&2)
}

/// The variables at the leaves under `idx`.
fn leaves_under(vtree: &Vtree, idx: VtreeIdx) -> Vec<u32> {
    let mut out = Vec::new();
    let mut stack = vec![idx];
    while let Some(node) = stack.pop() {
        if vtree.node(node).is_leaf() {
            out.push(vtree.leaf_var(node).get());
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

#[test]
fn a_component_centroid_ignores_other_components() {
    let td = make_td(
        vec![vec![0], vec![1], vec![2], vec![3], vec![4]],
        vec![(0, 1), (1, 2), (3, 4)],
        5,
    );

    assert_eq!(
        crate::decompose::td_to_vtree::algo::find_centroid(&td, 0),
        1,
    );
}

/// The hypergraph combiner reads a bag's clauses through the occurrence lists
/// of its items' variables, or through the whole formula once those are a
/// large share of it. Either way its hyperedges are the definition's: one per
/// clause, in clause order, holding the sorted positions of the items the
/// clause's variables sit in, kept when it holds two or more.
#[test]
fn hyperedges_read_through_occurrence_lists_are_the_ones_the_whole_formula_gives() {
    use super::super::combiners::{HyperedgeSource, clause_occurrences};
    use crate::tests::common::Lcg;
    let mut rng = Lcg::new(11);
    for round in 0..40 {
        let num_vars = 4 + rng.below(40) as u32;
        let clauses: Vec<Vec<i32>> = (0..1 + rng.below(60))
            .map(|_| {
                let mut chosen: Vec<i32> = Vec::new();
                for _ in 0..1 + rng.below(5) {
                    let v = 1 + rng.below(u64::from(num_vars)) as i32;
                    if !chosen.contains(&v) && !chosen.contains(&-v) {
                        chosen.push(if rng.below(2) == 0 { v } else { -v });
                    }
                }
                chosen
            })
            .collect();
        let formula = make_formula(num_vars, clauses);
        let occurrences = clause_occurrences(&formula);
        let mut source = HyperedgeSource::new(&formula, &occurrences, None);
        // Disjoint items over a random share of the variables, from a few
        // variables (read through the lists) to most of them (the whole pass).
        let mut item_vars: Vec<Vec<u32>> = Vec::new();
        let share = 1 + rng.below(8);
        for v in 0..num_vars {
            if rng.below(8) < share {
                if item_vars.is_empty() || rng.below(3) == 0 {
                    item_vars.push(Vec::new());
                }
                item_vars.last_mut().unwrap().push(v);
            }
        }
        let members: Vec<usize> = (0..item_vars.len()).filter(|_| rng.below(4) != 0).collect();
        let mut expected: Vec<Vec<u32>> = Vec::new();
        for clause in formula.clauses() {
            let mut pins: Vec<u32> = Vec::new();
            for lit in &clause.literals {
                let v = lit.var.idx() as u32;
                if let Some(position) = members.iter().position(|&m| item_vars[m].contains(&v))
                    && !pins.contains(&(position as u32))
                {
                    pins.push(position as u32);
                }
            }
            if pins.len() >= 2 {
                pins.sort_unstable();
                expected.push(pins);
            }
        }
        assert_eq!(
            source.hyperedges(&members, &item_vars),
            expected,
            "round {round}"
        );
        assert_eq!(
            source.hyperedges(&members, &item_vars),
            expected,
            "round {round}, read again through the cleared table"
        );
    }
}
