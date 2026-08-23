//! The builder itself: valid output, the same output twice, and no panics.
//!
//! These are the properties that must hold for any input at all, including
//! the degenerate ones — an empty formula, isolated variables, and an
//! instance large enough to take the neighbour-graph branch.

use super::*;

#[test]
fn one_leaf_per_var_including_isolated() {
    // 5 variables; variable index 4 appears in no clause.
    let formula = CnfFormula {
        num_vars: 5,
        clauses: vec![
            clause_dimacs(&[1, 2, 3]),
            clause_dimacs(&[2, 3]),
            clause_dimacs(&[1, 3]),
        ],
    };
    for mode in [ForceMode::Mst, ForceMode::Cut] {
        let vt = vtree_from_force(&formula, ForceConfig::new(mode)).unwrap();
        assert_covers_all_vars(&vt, 5, &format!("mode {mode:?}"));
    }
}

#[test]
fn two_builds_of_one_formula_are_identical() {
    let formula = CnfFormula {
        num_vars: 12,
        clauses: vec![
            clause_dimacs(&[1, 2, 3]),
            clause_dimacs(&[3, 4, 5]),
            clause_dimacs(&[5, 6, 7]),
            clause_dimacs(&[7, 8, 9]),
            clause_dimacs(&[9, 10, 11]),
            clause_dimacs(&[11, 12, 1]),
            clause_dimacs(&[2, 6, 10]),
        ],
    };
    for mode in [ForceMode::Mst, ForceMode::Cut] {
        let a = vtree_from_force(&formula, ForceConfig::new(mode)).unwrap();
        let b = vtree_from_force(&formula, ForceConfig::new(mode)).unwrap();
        assert_eq!(
            a.to_vtree_text(),
            b.to_vtree_text(),
            "force must be deterministic (mode {mode:?})"
        );
    }
}

#[test]
fn both_modes_valid_on_medium_formula() {
    let n = 50u32;
    // Deterministic pseudo-random 3-clauses, no dependency on the `rand` crate.
    let mut state = 0x1234_5678u64;
    let mut rand_var = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n as u64) as i32 + 1
    };
    let mut clauses = Vec::new();
    for _ in 0..120 {
        let a = rand_var();
        let mut b = rand_var();
        while b == a {
            b = rand_var();
        }
        let mut c = rand_var();
        while c == a || c == b {
            c = rand_var();
        }
        clauses.push(clause_dimacs(&[a, -b, c]));
    }
    let formula = CnfFormula {
        num_vars: n,
        clauses,
    };
    for mode in [ForceMode::Mst, ForceMode::Cut] {
        let vt = vtree_from_force(&formula, ForceConfig::new(mode)).unwrap();
        assert_covers_all_vars(&vt, n, &format!("mode {mode:?}"));
    }
}

#[test]
fn degenerate_inputs_dont_panic() {
    let mst = ForceConfig::new(ForceMode::Mst);
    let cut = ForceConfig::new(ForceMode::Cut);
    let f1 = CnfFormula {
        num_vars: 1,
        clauses: vec![],
    };
    assert_eq!(vtree_from_force(&f1, mst).unwrap().num_leaves(), 1);
    assert_eq!(vtree_from_force(&f1, cut).unwrap().num_leaves(), 1);

    let f2 = CnfFormula {
        num_vars: 8,
        clauses: vec![],
    };
    assert_covers_all_vars(&vtree_from_force(&f2, mst).unwrap(), 8, "isolated mst");
    assert_covers_all_vars(&vtree_from_force(&f2, cut).unwrap(), 8, "isolated cut");

    // One clause over every variable: all points collapse identically each round,
    // which exercises the whitening jitter guard.
    let f3 = CnfFormula {
        num_vars: 6,
        clauses: vec![clause_dimacs(&[1, 2, 3, 4, 5, 6])],
    };
    assert_covers_all_vars(&vtree_from_force(&f3, mst).unwrap(), 6, "collapsed mst");
    assert_covers_all_vars(&vtree_from_force(&f3, cut).unwrap(), 6, "collapsed cut");

    // No variables at all: an error naming the cause.
    let f0 = CnfFormula {
        num_vars: 0,
        clauses: vec![],
    };
    let err = vtree_from_force(&f0, mst).expect_err("an empty formula cannot build");
    assert!(err.contains("no variables"), "unexpected error: {err}");
}

/// Above [`PRIM_LIMIT`] points the MST switches from exact Prim to the grid-kNN
/// candidate graph, which need not span — a union-find fold closes the leftover
/// forest into one tree.
#[test]
#[ignore = "slow: a 20k-variable formula exercising the grid-kNN MST path, several \
            times the rest of the suite. Run with --include-ignored."]
fn grid_knn_branch_large_n() {
    let n = (PRIM_LIMIT + 50) as u32;
    let mut clauses = Vec::with_capacity(n as usize);
    for a in 0..(n - 1) {
        clauses.push(clause_dimacs(&[(a + 1) as i32, -((a + 2) as i32)]));
    }
    let formula = CnfFormula {
        num_vars: n,
        clauses,
    };
    let vt = vtree_from_force(&formula, ForceConfig::new(ForceMode::Mst)).unwrap();
    assert_eq!(
        vt.num_leaves(),
        n,
        "the grid-kNN path must stay leaf-complete"
    );
}
