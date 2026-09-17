use crate::cnf::CnfFormula;
use crate::cnf::{Reduced, ShowMask, ShowSet, VarId};
use crate::preprocess::bve_project::*;
use crate::tests::common::clause;
use crate::tests::pmc_oracle::{brute_force_pmc, show_indices};
use std::collections::HashSet;

/// The mask for a formula of `num_vars` whose eliminable (projected-out)
/// variables are `projected` — every other variable is shown.
fn hiding(num_vars: u32, projected: &[u32]) -> ShowMask {
    ShowSet::<Reduced>::from_vars((1..=num_vars).filter(|v| !projected.contains(v)).map(VarId))
        .unwrap()
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
    // x (variable 2) occurs only positively → pure → its clauses are deleted.
    // (a ∨ x) ∧ (b ∨ x), projected = {x}.
    let f = CnfFormula {
        num_vars: 3,
        clauses: vec![
            clause(&[(1, true), (2, true)]),
            clause(&[(3, true), (2, true)]),
        ],
    };
    let out = bve_project(&f, &hiding(f.num_vars, &[2]));
    assert!(!occurs(&out, 2), "pure projected var must be gone");
    assert!(out.clauses.is_empty(), "all x-clauses should be deleted");
}

#[test]
fn bve_project_basic_resolution() {
    // (a ∨ x) ∧ (b ∨ ¬x), projected = {x} → resolvent (a ∨ b), x gone.
    // a=1, b=2, x=3.
    let f = CnfFormula {
        num_vars: 3,
        clauses: vec![
            clause(&[(1, true), (3, true)]),
            clause(&[(2, true), (3, false)]),
        ],
    };
    let out = bve_project(&f, &hiding(f.num_vars, &[3]));
    assert!(!occurs(&out, 3), "x must be eliminated");
    assert!(
        has_clause(&out, &[(1, true), (2, true)]),
        "resolvent (a ∨ b) present"
    );
    assert_eq!(out.clauses.len(), 1);
}

#[test]
fn bve_project_taut_dropped() {
    // (a ∨ x) ∧ (a ∨ ¬x), projected = {x}.
    // Single cross-resolvent on x = (a ∨ a) = (a); x gone. a=1, x=2.
    let f = CnfFormula {
        num_vars: 2,
        clauses: vec![
            clause(&[(1, true), (2, true)]),
            clause(&[(1, true), (2, false)]),
        ],
    };
    let out = bve_project(&f, &hiding(f.num_vars, &[2]));
    assert!(!occurs(&out, 2), "x must be eliminated");
    assert!(has_clause(&out, &[(1, true)]), "resolvent collapses to (a)");
    assert_eq!(out.clauses.len(), 1);
}

#[test]
fn bve_project_leaves_a_var_whose_resolvents_outgrow_its_clauses() {
    // x (variable 1) is projected, occurring in 3 + 2 = 5 clauses (K=5). Cross-
    // resolvents: each of (a∨x),(b∨x),(c∨x) with each of (¬d∨¬x),(¬e∨¬x) →
    // 6 distinct non-tautological resolvents (R=6 > K=5).
    // a=2,b=3,c=4,d=5,e=6,x=1.
    let f = CnfFormula {
        num_vars: 6,
        clauses: vec![
            clause(&[(2, true), (1, true)]),
            clause(&[(3, true), (1, true)]),
            clause(&[(4, true), (1, true)]),
            clause(&[(5, false), (1, false)]),
            clause(&[(6, false), (1, false)]),
        ],
    };
    assert!(
        occurs(&bve_project(&f, &hiding(f.num_vars, &[1])), 1),
        "x must stay: R=6 > K=5"
    );
}

// --- Oracle route-through against brute force ----------------------------

/// `show` holds DIMACS variable numbers.
fn check_pmc(f: &CnfFormula, show: &[u32]) {
    let n = f.num_vars;
    let show_set = ShowSet::<Reduced>::from_vars(show.iter().map(|&v| VarId(v))).unwrap();
    let indices = show_indices(&show_set);
    let expected = brute_force_pmc(f, &indices);

    // Ids survive the pass, so the same show set names the same variables on
    // both sides and one oracle call answers each.
    let reduced = bve_project(f, &show_set.mask(n));

    let got = brute_force_pmc(&reduced, &indices);
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
    // Case 1: basic resolution actually eliminates a var (x=3 projected).
    check_pmc(
        &CnfFormula {
            num_vars: 3,
            clauses: vec![
                clause(&[(1, true), (3, true)]),
                clause(&[(2, true), (3, false)]),
            ],
        },
        &[1, 2],
    );

    // Case 2: 4 vars, show = {1,2}; project 3,4.
    check_pmc(
        &CnfFormula {
            num_vars: 4,
            clauses: vec![
                clause(&[(1, true), (3, false)]),
                clause(&[(3, true), (4, true)]),
                clause(&[(2, false), (4, false)]),
                clause(&[(1, false), (2, true)]),
            ],
        },
        &[1, 2],
    );

    // Case 3: 5 vars, mixed polarities, show = {1,5}.
    check_pmc(
        &CnfFormula {
            num_vars: 5,
            clauses: vec![
                clause(&[(1, true), (2, true), (3, false)]),
                clause(&[(2, false), (4, true)]),
                clause(&[(3, true), (4, false), (5, true)]),
                clause(&[(1, false), (5, false)]),
                clause(&[(3, true), (5, true)]),
            ],
        },
        &[1, 5],
    );

    // Case 4: UNSAT formula — projected count must be 0 both ways.
    // (x) ∧ (¬x) with x=2 projected, show = {1}.
    check_pmc(
        &CnfFormula {
            num_vars: 2,
            clauses: vec![clause(&[(2, true)]), clause(&[(2, false)])],
        },
        &[1],
    );

    // Case 5: another UNSAT via resolution chain; show = {1}.
    // (a ∨ x) ∧ (¬x) ∧ (¬a)  with a=1 show, x=2 projected.
    // ∃x: (a) ∧ (¬a) = UNSAT ⇒ 0 show tuples.
    check_pmc(
        &CnfFormula {
            num_vars: 2,
            clauses: vec![
                clause(&[(1, true), (2, true)]),
                clause(&[(2, false)]),
                clause(&[(1, false)]),
            ],
        },
        &[1],
    );
}
