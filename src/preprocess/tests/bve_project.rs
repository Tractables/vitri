use super::PatternBlocks;
use crate::cnf::CnfFormula;
use crate::cnf::{Literal, Reduced, ShowMask, ShowSet, VarId};
use crate::preprocess::bve_project::*;
use crate::tests::common::clause;
use crate::tests::pmc_oracle::{brute_force_pmc, show_indices};
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// The mask for a formula of `num_vars` whose eliminable (projected-out)
/// variables are `projected` — every other variable is shown.
fn hiding(num_vars: u32, projected: &[u32]) -> ShowMask {
    ShowSet::<Reduced>::from_vars(VarId::all(num_vars).filter(|v| !projected.contains(&v.get())))
        .mask(num_vars)
}

fn occurs(f: &CnfFormula, var: u32) -> bool {
    f.clauses()
        .iter()
        .any(|c| c.literals.iter().any(|l| l.var.get() == var))
}

/// Does the formula contain a clause exactly equal (as a set) to `lits`?
fn has_clause(f: &CnfFormula, lits: &[(u32, bool)]) -> bool {
    let want: HashSet<(u32, bool)> = lits.iter().map(|&(v, p)| (v, p)).collect();
    f.clauses().iter().any(|c| {
        let got: HashSet<(u32, bool)> = c
            .literals
            .iter()
            .map(|l| (l.var.get(), l.positive))
            .collect();
        got == want
    })
}

#[test]
fn bve_project_pure_literal() {
    // x (variable 2) occurs only positively → pure → its clauses are deleted.
    // (a ∨ x) ∧ (b ∨ x), projected = {x}.
    let f = CnfFormula::from_parts(
        3,
        vec![
            clause(&[(1, true), (2, true)]),
            clause(&[(3, true), (2, true)]),
        ],
    );
    let out = bve_project(&f, &hiding(f.num_vars(), &[2]), None);
    assert!(!occurs(&out, 2), "pure projected var must be gone");
    assert!(out.clauses().is_empty(), "all x-clauses should be deleted");
}

#[test]
fn bve_project_basic_resolution() {
    // (a ∨ x) ∧ (b ∨ ¬x), projected = {x} → resolvent (a ∨ b), x gone.
    // a=1, b=2, x=3.
    let f = CnfFormula::from_parts(
        3,
        vec![
            clause(&[(1, true), (3, true)]),
            clause(&[(2, true), (3, false)]),
        ],
    );
    let out = bve_project(&f, &hiding(f.num_vars(), &[3]), None);
    assert!(!occurs(&out, 3), "x must be eliminated");
    assert!(
        has_clause(&out, &[(1, true), (2, true)]),
        "resolvent (a ∨ b) present"
    );
    assert_eq!(out.clauses().len(), 1);
}

#[test]
fn bve_project_taut_dropped() {
    // (a ∨ x) ∧ (a ∨ ¬x), projected = {x}.
    // Single cross-resolvent on x = (a ∨ a) = (a); x gone. a=1, x=2.
    let f = CnfFormula::from_parts(
        2,
        vec![
            clause(&[(1, true), (2, true)]),
            clause(&[(1, true), (2, false)]),
        ],
    );
    let out = bve_project(&f, &hiding(f.num_vars(), &[2]), None);
    assert!(!occurs(&out, 2), "x must be eliminated");
    assert!(has_clause(&out, &[(1, true)]), "resolvent collapses to (a)");
    assert_eq!(out.clauses().len(), 1);
}

#[test]
fn bve_project_leaves_a_var_whose_resolvents_outgrow_its_clauses() {
    // x (variable 1) is projected, occurring in 3 + 2 = 5 clauses (K=5). Cross-
    // resolvents: each of (a∨x),(b∨x),(c∨x) with each of (¬d∨¬x),(¬e∨¬x) →
    // 6 distinct non-tautological resolvents (R=6 > K=5).
    // a=2,b=3,c=4,d=5,e=6,x=1.
    let f = CnfFormula::from_parts(
        6,
        vec![
            clause(&[(2, true), (1, true)]),
            clause(&[(3, true), (1, true)]),
            clause(&[(4, true), (1, true)]),
            clause(&[(5, false), (1, false)]),
            clause(&[(6, false), (1, false)]),
        ],
    );
    assert!(
        occurs(&bve_project(&f, &hiding(f.num_vars(), &[1]), None), 1),
        "x must stay: R=6 > K=5"
    );
}

// --- Oracle route-through against brute force ----------------------------

/// `show` holds DIMACS variable numbers.
fn check_pmc(f: &CnfFormula, show: &[u32]) {
    let n = f.num_vars();
    let show_set = ShowSet::<Reduced>::from_vars(show.iter().map(|&v| VarId::new(v).unwrap()));
    let indices = show_indices(&show_set);
    let expected = brute_force_pmc(f, &indices);

    // Ids survive the pass, so the same show set names the same variables on
    // both sides and one oracle call answers each.
    let reduced = bve_project(f, &show_set.mask(n), None);

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
        &CnfFormula::from_parts(
            3,
            vec![
                clause(&[(1, true), (3, true)]),
                clause(&[(2, true), (3, false)]),
            ],
        ),
        &[1, 2],
    );

    // Case 2: 4 vars, show = {1,2}; project 3,4.
    check_pmc(
        &CnfFormula::from_parts(
            4,
            vec![
                clause(&[(1, true), (3, false)]),
                clause(&[(3, true), (4, true)]),
                clause(&[(2, false), (4, false)]),
                clause(&[(1, false), (2, true)]),
            ],
        ),
        &[1, 2],
    );

    // Case 3: 5 vars, mixed polarities, show = {1,5}.
    check_pmc(
        &CnfFormula::from_parts(
            5,
            vec![
                clause(&[(1, true), (2, true), (3, false)]),
                clause(&[(2, false), (4, true)]),
                clause(&[(3, true), (4, false), (5, true)]),
                clause(&[(1, false), (5, false)]),
                clause(&[(3, true), (5, true)]),
            ],
        ),
        &[1, 5],
    );

    // Case 4: UNSAT formula — projected count must be 0 both ways.
    // (x) ∧ (¬x) with x=2 projected, show = {1}.
    check_pmc(
        &CnfFormula::from_parts(2, vec![clause(&[(2, true)]), clause(&[(2, false)])]),
        &[1],
    );

    // Case 5: another UNSAT via resolution chain; show = {1}.
    // (a ∨ x) ∧ (¬x) ∧ (¬a)  with a=1 show, x=2 projected.
    // ∃x: (a) ∧ (¬a) = UNSAT ⇒ 0 show tuples.
    check_pmc(
        &CnfFormula::from_parts(
            2,
            vec![
                clause(&[(1, true), (2, true)]),
                clause(&[(2, false)]),
                clause(&[(1, false)]),
            ],
        ),
        &[1],
    );
}

/// Projected BVE stops at its deadline, and hands back a partial elimination:
/// each block of the input is either untouched or has lost its hidden variable
/// to exactly that variable's resolvents, never anything in between.
///
/// Run to its fixpoint, this input takes several times the bound below, even
/// in an optimized build: each block's hidden variable has sixteen million
/// resolvent pairs to visit.
#[test]
fn projected_bve_stops_at_its_deadline_with_only_whole_eliminations() {
    let fixture = PatternBlocks {
        blocks: 20,
        width: 12,
        patterns: 4_000,
    };
    let formula = fixture.formula();
    let mask = fixture.show().mask(formula.num_vars());
    let budget = Duration::from_millis(1_500);
    let started = Instant::now();
    let out = bve_project(&formula, &mask, Some(started + budget));
    let elapsed = started.elapsed();

    // The margin covers what does not read the clock: normalizing the clauses
    // on the way in and rebuilding the formula on the way out.
    assert!(
        elapsed < budget + Duration::from_secs(2),
        "projected BVE returned after {elapsed:?} against a {budget:?} deadline"
    );
    assert_eq!(out.num_vars(), formula.num_vars());

    let mut by_block: Vec<Vec<Vec<Literal>>> = vec![Vec::new(); fixture.blocks as usize];
    for c in out.clauses() {
        by_block[fixture.block_of(c.literals[0].var) as usize].push(c.literals.clone());
    }
    let as_set = |clauses: &[Vec<Literal>]| clauses.iter().cloned().collect::<HashSet<_>>();
    for (b, got) in by_block.iter().enumerate() {
        let b = b as u32;
        let whole =
            |want: Vec<Vec<Literal>>| got.len() == want.len() && as_set(got) == as_set(&want);
        assert!(
            whole(fixture.block(b)) || whole(fixture.patterns_of(b)),
            "block {b} is neither untouched nor its hidden variable's resolvents"
        );
    }
}

/// A deadline that never arrives changes nothing: the clock is only read, so
/// the pass eliminates exactly what it does with no deadline at all.
#[test]
fn a_far_deadline_eliminates_what_no_deadline_does() {
    let fixture = PatternBlocks {
        blocks: 3,
        width: 4,
        patterns: 10,
    };
    let formula = fixture.formula();
    let mask = fixture.show().mask(formula.num_vars());
    let unbounded = bve_project(&formula, &mask, None);
    let far = bve_project(
        &formula,
        &mask,
        Some(Instant::now() + Duration::from_secs(3_600)),
    );
    assert_eq!(
        unbounded.clauses().len(),
        (fixture.blocks * fixture.patterns) as usize,
        "every hidden variable goes here, or the comparison would not reach the resolvents"
    );
    assert_eq!(far, unbounded);
}
