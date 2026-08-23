use crate::cnf::CnfFormula;
use crate::cnf::{Reduced, ShowMask, ShowSet};
use crate::preprocess::bve_project::*;
use crate::tests::common::clause;
use crate::tests::pmc_oracle::brute_force_pmc;
use std::collections::HashSet;

/// The mask for a formula of `num_vars` whose eliminable (projected-out)
/// variables are `projected` — every other variable is shown.
fn hiding(num_vars: u32, projected: &[u32]) -> ShowMask {
    ShowSet::<Reduced>::from_zero_based((0..num_vars).filter(|v| !projected.contains(v)))
        .mask(num_vars)
}

fn occurs(f: &CnfFormula, var: u32) -> bool {
    f.clauses
        .iter()
        .any(|c| c.literals.iter().any(|l| l.var.0 == var))
}

/// Does the formula contain a clause exactly equal (as a set) to `lits`?
fn has_clause(f: &CnfFormula, lits: &[(u32, bool)]) -> bool {
    let want: HashSet<(u32, bool)> = lits.iter().map(|&(v, p)| (v, p)).collect();
    f.clauses.iter().any(|c| {
        let got: HashSet<(u32, bool)> = c.literals.iter().map(|l| (l.var.0, l.positive)).collect();
        got == want
    })
}

#[test]
fn bve_project_pure_literal() {
    // x (var 1) occurs only positively → pure → its clauses are deleted.
    // (a ∨ x) ∧ (b ∨ x), projected = {x}.
    let f = CnfFormula {
        num_vars: 3,
        clauses: vec![
            clause(&[(0, true), (1, true)]),
            clause(&[(2, true), (1, true)]),
        ],
    };
    let out = bve_project(&f, &hiding(f.num_vars, &[1]));
    assert!(!occurs(&out, 1), "pure projected var must be gone");
    assert!(out.clauses.is_empty(), "all x-clauses should be deleted");
}

#[test]
fn bve_project_basic_resolution() {
    // (a ∨ x) ∧ (b ∨ ¬x), projected = {x} → resolvent (a ∨ b), x gone.
    // a=0, b=1, x=2.
    let f = CnfFormula {
        num_vars: 3,
        clauses: vec![
            clause(&[(0, true), (2, true)]),
            clause(&[(1, true), (2, false)]),
        ],
    };
    let out = bve_project(&f, &hiding(f.num_vars, &[2]));
    assert!(!occurs(&out, 2), "x must be eliminated");
    assert!(
        has_clause(&out, &[(0, true), (1, true)]),
        "resolvent (a ∨ b) present"
    );
    assert_eq!(out.clauses.len(), 1);
}

#[test]
fn bve_project_taut_dropped() {
    // (a ∨ x) ∧ (a ∨ ¬x), projected = {x}.
    // Single cross-resolvent on x = (a ∨ a) = (a); x gone. a=0, x=1.
    let f = CnfFormula {
        num_vars: 2,
        clauses: vec![
            clause(&[(0, true), (1, true)]),
            clause(&[(0, true), (1, false)]),
        ],
    };
    let out = bve_project(&f, &hiding(f.num_vars, &[1]));
    assert!(!occurs(&out, 1), "x must be eliminated");
    assert!(has_clause(&out, &[(0, true)]), "resolvent collapses to (a)");
    assert_eq!(out.clauses.len(), 1);
}

#[test]
fn bve_project_growth_ratio_gates_elimination() {
    // x (var 0) is projected, occurring in 3 + 2 = 5 clauses (K=5). Cross-
    // resolvents: each of (a∨x),(b∨x),(c∨x) with each of (¬d∨¬x),(¬e∨¬x) →
    // 6 distinct non-tautological resolvents (R=6 > K=5).
    // a=1,b=2,c=3,d=4,e=5,x=0.
    let f = CnfFormula {
        num_vars: 6,
        clauses: vec![
            clause(&[(1, true), (0, true)]),
            clause(&[(2, true), (0, true)]),
            clause(&[(3, true), (0, true)]),
            clause(&[(4, false), (0, false)]),
            clause(&[(5, false), (0, false)]),
        ],
    };
    assert!(
        occurs(&bve_project_bounded(&f, &hiding(f.num_vars, &[0]), 1.0), 0),
        "grow=1.0 must skip x (R=6 > K=5)"
    );
    assert!(
        !occurs(&bve_project_bounded(&f, &hiding(f.num_vars, &[0]), 2.0), 0),
        "grow=2.0 must eliminate x (R=6 ≤ 10)"
    );
}

// --- Oracle route-through against brute force ----------------------------

fn check_pmc(f: &CnfFormula, show: &[u32]) {
    let n = f.num_vars;
    let expected = brute_force_pmc(f, show);

    // Ids survive the pass, so the same show set names the same variables on
    // both sides and one oracle call answers each.
    let reduced = bve_project(
        f,
        &ShowSet::<Reduced>::from_zero_based(show.iter().copied()).mask(n),
    );

    let got = brute_force_pmc(&reduced, show);
    assert_eq!(
        expected, got,
        "PMC mismatch: original={expected} reduced={got}; show={show:?}"
    );

    // Show vars must never be eliminated by bve_project.
    for &s in show {
        let in_orig = occurs(f, s);
        if in_orig {
            // Subsumption/UP are not part of this pass.
            assert!(occurs(&reduced, s), "show var {s} must not be eliminated");
        }
    }
}

#[test]
fn bve_project_preserves_pmc() {
    // Case 1: basic resolution actually eliminates a var (x=2 projected).
    check_pmc(
        &CnfFormula {
            num_vars: 3,
            clauses: vec![
                clause(&[(0, true), (2, true)]),
                clause(&[(1, true), (2, false)]),
            ],
        },
        &[0, 1],
    );

    // Case 2: 4 vars, show = {0,1}; project 2,3.
    check_pmc(
        &CnfFormula {
            num_vars: 4,
            clauses: vec![
                clause(&[(0, true), (2, false)]),
                clause(&[(2, true), (3, true)]),
                clause(&[(1, false), (3, false)]),
                clause(&[(0, false), (1, true)]),
            ],
        },
        &[0, 1],
    );

    // Case 3: 5 vars, mixed polarities, show = {0,4}.
    check_pmc(
        &CnfFormula {
            num_vars: 5,
            clauses: vec![
                clause(&[(0, true), (1, true), (2, false)]),
                clause(&[(1, false), (3, true)]),
                clause(&[(2, true), (3, false), (4, true)]),
                clause(&[(0, false), (4, false)]),
                clause(&[(2, true), (4, true)]),
            ],
        },
        &[0, 4],
    );

    // Case 4: UNSAT formula — projected count must be 0 both ways.
    // (x) ∧ (¬x) with x=1 projected, show = {0}.
    check_pmc(
        &CnfFormula {
            num_vars: 2,
            clauses: vec![clause(&[(1, true)]), clause(&[(1, false)])],
        },
        &[0],
    );

    // Case 5: another UNSAT via resolution chain; show = {0}.
    // (a ∨ x) ∧ (¬x) ∧ (¬a)  with a=0 show, x=1 projected.
    // ∃x: (a) ∧ (¬a) = UNSAT ⇒ 0 show tuples.
    check_pmc(
        &CnfFormula {
            num_vars: 2,
            clauses: vec![
                clause(&[(0, true), (1, true)]),
                clause(&[(1, false)]),
                clause(&[(0, false)]),
            ],
        },
        &[0],
    );
}
