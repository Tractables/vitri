//! Conformance tests for [`BisectionSolver`]. Any new backend can be
//! plugged into `assert_partition_contract` to validate it against the
//! contract documented on the trait.
use crate::cnf::CnfFormula;
use crate::decompose::multilevel_hg_bisect::HypergraphBisectSolver;
use crate::decompose::*;
use crate::tests::common::{assert_covers_all_vars, clause_dimacs};
use crate::vtree::Vtree;
use std::sync::Arc;

fn small_formula() -> CnfFormula {
    // A few overlapping clauses on 4 variables: enough structure to exercise
    // recursion, partition, and the minfill fallback at the leaves.
    CnfFormula {
        num_vars: 4,
        clauses: vec![
            clause_dimacs(&[1, 2, 3]),
            clause_dimacs(&[2, 3, 4]),
            clause_dimacs(&[1, 4]),
        ],
    }
}

fn assert_partition_contract<S: BisectionSolver>(
    solver: &mut S,
    vars: &[u32],
    formula: &CnfFormula,
) {
    let Some(split) = solver.partition(vars, formula) else {
        return; // "nothing here says how these divide" — allowed
    };
    let mut placed: Vec<u32> = split.left.iter().chain(&split.right).copied().collect();
    placed.sort_unstable();
    let mut expected = vars.to_vec();
    expected.sort_unstable();
    assert_eq!(
        placed, expected,
        "a split must place every variable of the subset exactly once"
    );
}

fn assert_full_run_produces_valid_vtree<S: BisectionSolver>(mut solver: S, formula: &CnfFormula) {
    let vt: Arc<Vtree> = run_bisection(formula, &mut solver).expect("bisection failed");
    assert_covers_all_vars(&vt, formula.num_vars, "run_bisection");
}

#[test]
fn hypergraph_bisect_satisfies_contract() {
    let f = small_formula();
    let vars: Vec<u32> = (0..f.num_vars).collect();
    let dials = crate::decompose::BisectDials {
        imbalance: 0.1,
        base_seed: 0,
        effort_scale: 1.0,
    };
    let mut s = HypergraphBisectSolver { dials };
    assert_partition_contract(&mut s, &vars, &f);
    assert_full_run_produces_valid_vtree(HypergraphBisectSolver { dials }, &f);
}

/// The clauses ARE the hyperedges, so a subset no clause ties together has
/// nothing saying how it divides — reported as no split, which the recursion
/// recovers from with a split of its own. The shared contract check above
/// returns early on that answer, so the two cases are separated here.
#[test]
fn a_subset_no_clause_ties_together_reports_no_split() {
    let dials = crate::decompose::BisectDials {
        imbalance: 0.1,
        base_seed: 0,
        effort_scale: 1.0,
    };
    let mut solver = HypergraphBisectSolver { dials };

    // Variables 0..3 appear only in clauses that hold one of them each, so no
    // clause holds two and there is no hyperedge over the subset.
    let untied = CnfFormula {
        num_vars: 6,
        clauses: vec![
            clause_dimacs(&[1, 5]),
            clause_dimacs(&[2, 5]),
            clause_dimacs(&[3, 6]),
            clause_dimacs(&[4, 6]),
        ],
    };
    assert!(
        solver.partition(&[0, 1, 2, 3], &untied).is_none(),
        "no clause ties two of the subset together, so nothing says how it divides",
    );

    // One clause over two of them is enough to have an opinion, and then every
    // variable of the subset is placed exactly once.
    let tied = CnfFormula {
        num_vars: 6,
        clauses: vec![
            clause_dimacs(&[1, 2]),
            clause_dimacs(&[2, 5]),
            clause_dimacs(&[3, 6]),
            clause_dimacs(&[4, 6]),
        ],
    };
    let split = solver
        .partition(&[0, 1, 2, 3], &tied)
        .expect("one clause over the subset is an opinion about how it divides");
    let mut placed: Vec<u32> = split.left.iter().chain(&split.right).copied().collect();
    placed.sort_unstable();
    assert_eq!(placed, vec![0, 1, 2, 3]);
}
