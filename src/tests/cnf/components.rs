//! Extraction renumbers into a component-local space, which is where the
//! free variables — belonging to no clause — have to be accounted for
//! explicitly rather than falling off the end.

use super::*;

#[test]
fn test_detect_components_single() {
    let input = b"p cnf 3 2\n1 2 0\n2 3 0\n";
    let formula = CnfFormula::from_dimacs(&input[..]).unwrap().0;
    assert!(formula.detect_components().is_none());
}

#[test]
fn test_detect_components_two() {
    let formula = CnfFormula::from_dimacs(TWO_DISJOINT_CLAUSES.as_bytes())
        .unwrap()
        .0;
    let comps = formula.detect_components().unwrap();
    assert_eq!(comps.len(), 2);
    assert_eq!(comps[0].len(), 1);
    assert_eq!(comps[1].len(), 1);
}

#[test]
fn test_extract_component_renumbering() {
    let input = b"p cnf 6 2\n1 2 0\n5 6 0\n";
    let formula = CnfFormula::from_dimacs(&input[..]).unwrap().0;
    let comps = formula.detect_components().unwrap();
    assert_eq!(comps.len(), 2);

    for comp_indices in &comps {
        let (sub, local_to_global) = formula.extract_component(comp_indices);
        assert_eq!(sub.num_vars, 2);
        assert_eq!(sub.clauses.len(), 1);
        assert_eq!(local_to_global.len(), 2);
        assert_eq!(sub.clauses[0].literals[0].var, VarId(0));
        assert_eq!(sub.clauses[0].literals[1].var, VarId(1));
    }
}

/// The split is a property of the formula, not of the order its clauses happen
/// to be written in. The bucketing runs over a randomly-seeded map, so the sort
/// after it is the whole reason two orderings of one formula split alike — and a
/// consumer's graft chain, and the diagram it compiles, depend on that order.
#[test]
fn a_split_is_the_same_however_the_clauses_were_ordered() {
    let input = b"p cnf 6 4\n1 2 0\n2 3 0\n4 5 0\n6 0\n";
    let formula = CnfFormula::from_dimacs(&input[..]).unwrap().0;
    let reversed = CnfFormula {
        num_vars: formula.num_vars,
        clauses: formula.clauses.iter().rev().cloned().collect(),
    };

    // A component is a set of clause INDICES, which reordering necessarily
    // changes; what must not change is which variables each component holds.
    let vars_of = |f: &CnfFormula| {
        f.detect_components()
            .expect("three independent components")
            .iter()
            .map(|group| {
                let mut vars: Vec<u32> = group
                    .iter()
                    .flat_map(|&ci| f.clauses[ci].literals.iter().map(|l| l.var.0))
                    .collect();
                vars.sort_unstable();
                vars.dedup();
                vars
            })
            .collect::<Vec<_>>()
    };

    let forward = vars_of(&formula);
    assert_eq!(forward, vec![vec![3, 4], vec![5], vec![0, 1, 2]]);
    assert_eq!(
        vars_of(&reversed),
        forward,
        "reversing the clause list changed the split",
    );
}

#[test]
fn test_detect_components_with_free_vars() {
    // Var 3 (DIMACS 4) is free — not in any clause.
    let input = b"p cnf 4 2\n1 2 0\n3 0\n";
    let formula = CnfFormula::from_dimacs(&input[..]).unwrap().0;
    let comps = formula.detect_components().unwrap();
    assert_eq!(comps.len(), 2);
}

/// A caller holding clauses it has not wrapped in a formula asks the same
/// question of the same clauses and gets the same answer — one implementation,
/// reached two ways.
#[test]
fn the_clause_slice_split_and_the_formula_method_agree() {
    let formula = CnfFormula::from_dimacs(TWO_DISJOINT_CLAUSES.as_bytes())
        .unwrap()
        .0;
    assert_eq!(
        detect_components_in(&formula.clauses, formula.num_vars),
        formula.detect_components(),
    );
}

#[test]
fn a_connected_clause_slice_reports_no_split_to_make() {
    let formula = CnfFormula::from_dimacs(&b"p cnf 3 2\n1 2 0\n2 3 0\n"[..])
        .unwrap()
        .0;
    assert_eq!(
        detect_components_in(&formula.clauses, formula.num_vars),
        None,
        "a caller that would allocate a partition here would not use it",
    );
}

/// The universe is what the clauses are numbered over, not a count of what they
/// mention: variables in no clause belong to no component and change no split.
#[test]
fn a_universe_wider_than_the_clauses_splits_the_same_way() {
    let formula = CnfFormula::from_dimacs(TWO_DISJOINT_CLAUSES.as_bytes())
        .unwrap()
        .0;
    assert_eq!(
        detect_components_in(&formula.clauses, formula.num_vars + 100),
        formula.detect_components(),
    );
}
