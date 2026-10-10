use super::*;
use crate::preprocess::arjun_lib::stages::stage_one_deadline;
use crate::tests::common::{Lcg, clause_dimacs, lit};
use crate::tests::pmc_oracle::brute_force_mc;

#[test]
fn anytime_reduces_toy_cnf() {
    let mut f = CnfFormula::from_parts(10, Vec::new());
    for c in [
        &[1, 2][..],
        &[1, 2, 9],
        &[3, -4],
        &[3, -4, 5],
        &[-1, 6],
        &[6, -2],
    ] {
        f.clauses_mut().push(clause_dimacs(c));
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    let r = reduce_anytime(
        &f,
        deadline,
        ArjunOptions::default(),
        /*force_no_sbva=*/ false,
    )
    .expect("no VITRI_* knob is set in this test")
    .expect("reduce");
    assert!(r.formula.num_vars() <= 10);
    assert!(r.multiplier_exp > 0, "exp={}", r.multiplier_exp);
}

/// A formula that gives Arjun real work to do, so a tight deadline can
/// actually land inside a stage rather than before stage 1. Small enough
/// to brute-force count (n <= 20), structured enough (chains + a parity
/// ladder + free vars) that BVE/SBVA/oracle all have something to chew on.
fn deadline_probe_formula() -> CnfFormula {
    let mut clauses = Vec::new();
    // Implication chain 1→2→…→12 (BVE bait).
    for v in 1..=11u32 {
        clauses.push(Clause::new(vec![lit(v, false), lit(v + 1, true)]));
    }
    // A ladder of ternary clauses over the same vars (oracle/vivify bait).
    for v in 1..=10u32 {
        clauses.push(Clause::new(vec![
            lit(v, true),
            lit(v + 1, false),
            lit(v + 2, true),
        ]));
        clauses.push(Clause::new(vec![
            lit(v, false),
            lit(v + 1, true),
            lit(v + 2, false),
        ]));
    }
    // vars 13..=16 appear in no clause at all ⇒ folded into the multiplier.
    CnfFormula::from_parts(16, clauses)
}

/// The in-process deadline must (a) be HONORED — the reduction returns
/// within the budget plus a small margin, without the fork's `SIGKILL`
/// having to do it — and (b) still hand back a SOUND checkpoint, i.e. the
/// reduced count scaled by `2^multiplier_exp` equals the original count.
///
/// Run inline (`reduce_anytime_inner`) so the assertion is about Arjun
/// stopping itself, NOT about the fork's kill.
#[test]
fn anytime_deadline_honored_and_sound() {
    let formula = deadline_probe_formula();
    let expected = brute_force_mc(&formula);
    let budget = Duration::from_millis(300);
    let started = Instant::now();
    let r = reduce_anytime_inner(
        &formula,
        started + budget,
        ArjunOptions::default(),
        /*no_sbva_call=*/ false,
    );
    let elapsed = started.elapsed();

    // (a) Honored. The margin covers the coarse polling granularity (the
    // oracle polls once per 1024 propagations, CaDiCaL every
    // `terminateint` conflicts) plus the un-gated finalization epilogue,
    // which must always run — this asserts a BOUND, not exact timing.
    assert!(
        elapsed < budget + Duration::from_secs(10),
        "deadline not honored in-process: returned after {:?} against a {:?} budget",
        elapsed,
        budget
    );

    // (b) Sound. A partial reduction is an exact checkpoint or nothing.
    if let Some(r) = r {
        let reduced = brute_force_mc(&r.formula);
        let got = reduced.clone() << r.multiplier_exp;
        assert_eq!(
            got, expected,
            "deadline-cut reduction is not count-preserving: {} << {} = {} != {}",
            reduced, r.multiplier_exp, got, expected
        );
    }
}

/// A far-future deadline must be indistinguishable from no deadline: every
/// deadline check is `now > deadline`, so with the deadline out of reach
/// they are all false and the reduction is unaffected. Arjun is seeded
/// (RNG seed 42 by default), so "same" is exact equality, not a size
/// heuristic.
///
/// This guards the property that deadline-unset is bit-identical, at the
/// only place we can observe it from Rust: two far deadlines that differ
/// by an hour must reduce identically.
#[test]
fn far_deadline_reduction_is_deterministic() {
    let formula = deadline_probe_formula();
    let a = reduce_anytime_inner(
        &formula,
        Instant::now() + Duration::from_secs(600),
        ArjunOptions::default(),
        /*no_sbva_call=*/ false,
    )
    .expect("reduce with 600s budget");
    let b = reduce_anytime_inner(
        &formula,
        Instant::now() + Duration::from_secs(4200),
        ArjunOptions::default(),
        /*no_sbva_call=*/ false,
    )
    .expect("reduce with 4200s budget");
    assert_eq!(
        a.formula, b.formula,
        "far-future deadline changed the reduction"
    );
    assert_eq!(a.multiplier_exp, b.multiplier_exp);
    assert_eq!(a.backbone, b.backbone);
    assert_eq!(a.equiv, b.equiv);
    assert_eq!(a.independent_support, b.independent_support);
}

/// Fork parity: what the caller gets back through the hard-deadline fork
/// must be exactly what the same reduction produces inline. Arjun is
/// seeded/deterministic by default, so with an ample deadline (neither run
/// can hit the budget gates) every field must match except `budget`, which is
/// a measured duration. This is the guard that the payload codec carries the
/// WHOLE result — including the input-space backbone/equiv harvest the raw
/// fallback lane is seeded from, which is easy to drop silently.
#[test]
fn reduce_anytime_fork_matches_direct() {
    // Forced unit + implication chain + an equivalence pair, so backbone and
    // equiv are both non-empty and actually have to cross the pipe.
    let formula = CnfFormula::from_parts(
        6,
        vec![
            Clause::new(vec![lit(1, true)]),
            Clause::new(vec![lit(1, false), lit(2, true)]),
            Clause::new(vec![lit(2, false), lit(3, true)]),
            Clause::new(vec![lit(4, true), lit(5, false)]),
            Clause::new(vec![lit(4, false), lit(5, true)]),
        ],
    );
    let budget = Duration::from_secs(30);
    let forked = reduce_anytime(
        &formula,
        Instant::now() + budget,
        ArjunOptions::default(),
        /*force_no_sbva=*/ false,
    )
    .expect("no VITRI_* knob is set in this test")
    .expect("forked reduce");
    let direct = reduce_anytime_inner(
        &formula,
        Instant::now() + budget,
        ArjunOptions::default(),
        /*no_sbva_call=*/ false,
    )
    .expect("direct reduce");

    assert_eq!(
        forked.formula, direct.formula,
        "reduced formula differs across the fork"
    );
    assert_eq!(forked.multiplier_exp, direct.multiplier_exp);
    assert_eq!(
        forked.backbone, direct.backbone,
        "backbone harvest lost/altered by the fork"
    );
    assert_eq!(
        forked.equiv, direct.equiv,
        "equiv harvest lost/altered by the fork"
    );
    assert_eq!(forked.learnt_clauses, direct.learnt_clauses);
    assert_eq!(
        forked.independent_support, direct.independent_support,
        "independent support lost/altered by the fork",
    );
    for var in forked.independent_support.iter_vars() {
        assert!(
            var.get() <= forked.formula.num_vars(),
            "support variable {} is outside the final checkpoint's {} variables",
            var.get(),
            forked.formula.num_vars(),
        );
    }
    // The variable map crosses the fork as a nullable signed vector; a codec
    // that dropped a `None` or a sign would mislift every model downstream.
    assert_eq!(
        forked.input_to_reduced_lit, direct.input_to_reduced_lit,
        "input->reduced variable map lost/altered by the fork",
    );
    // The harvest must be non-empty here, else the assertions above are vacuous.
    assert!(
        !forked.backbone.is_empty(),
        "expected a non-empty backbone harvest"
    );
    // The map's own invariants. NOT "at least one surviving variable": a
    // full-count reduction of a formula this small is solved outright by Arjun
    // (0v/0c), and an all-`None` map is then the correct answer, not a broken
    // getter. What must hold regardless is that whatever it does name is in
    // range and injective — the property a consumer's model lift relies on.
    assert_eq!(
        forked.input_to_reduced_lit.len(),
        formula.num_vars() as usize,
        "the map must be indexed by input variable",
    );
    let mut claimed = vec![false; forked.formula.num_vars() as usize];
    for e in forked.input_to_reduced_lit.iter().flatten() {
        let r = e.unsigned_abs() as usize;
        assert!(
            r >= 1 && r <= forked.formula.num_vars() as usize,
            "map names reduced var {r}, outside 1..={}",
            forked.formula.num_vars(),
        );
        assert!(!claimed[r - 1], "two input vars map onto reduced var {r}");
        claimed[r - 1] = true;
    }
}

/// The projection of [`backward_search_fixture`]: its variables `1..=SHOWN`.
const SHOWN: u32 = 200;

/// A projected formula whose stage-1 backward search runs far past a budget of
/// a few seconds, and whose only sound independent support is the whole
/// projection.
///
/// Variables above [`SHOWN`] are a random 3-CNF at the clause density where
/// random 3-CNF is hardest for a CDCL solver, drawn so that every clause has a
/// positive literal: the all-true assignment is a model. Each shown variable
/// `s` meets that ballast only in `(s ∨ b)` and `(¬s ∨ b')`, so with the
/// ballast all true it takes either value whatever the others are: no shown
/// variable is defined by the rest, and the projected count is `2^SHOWN`.
///
/// Arjun cannot see that cheaply. Its backward search tests the candidates
/// inside one solve over two copies of the formula, and each test here runs
/// until its conflict allowance is spent, so the solve outlasts the budget
/// and the deadline lands inside it.
fn backward_search_fixture() -> (u32, Vec<Vec<i32>>) {
    const BALLAST: u32 = 1000;
    // Clauses per ballast variable, in tenths: 4.2, near the 3-SAT threshold.
    const DENSITY_TENTHS: u32 = 42;
    let mut rng = Lcg::new(0xA5_1E);
    let ballast = |rng: &mut Lcg| (SHOWN + 1 + rng.below(u64::from(BALLAST)) as u32) as i32;
    let mut clauses = Vec::new();
    for _ in 0..BALLAST * DENSITY_TENTHS / 10 {
        let mut c: Vec<i32> = Vec::with_capacity(3);
        while c.len() < 3 {
            let v = ballast(&mut rng);
            if !c.iter().any(|l| l.abs() == v) {
                c.push(if rng.below(2) == 0 { v } else { -v });
            }
        }
        // A clause with no positive literal gets its first literal flipped,
        // which keeps the all-true assignment a model.
        if c.iter().all(|&l| l < 0) {
            c[0] = -c[0];
        }
        clauses.push(c);
    }
    for s in 1..=SHOWN as i32 {
        clauses.push(vec![s, ballast(&mut rng)]);
        clauses.push(vec![-s, ballast(&mut rng)]);
    }
    (SHOWN + BALLAST, clauses)
}

/// A deadline that passes inside stage 1's backward search stops the search
/// there, and every candidate not yet shown to be defined stays in the
/// support — here all of them, since none is defined. Without the deadline
/// check inside the search, the stage runs on until the solve has walked
/// every candidate, long past the budget.
#[test]
fn a_deadline_inside_the_backward_search_stops_it_with_every_untested_candidate_kept() {
    let (num_vars, clauses) = backward_search_fixture();
    let shown: Vec<VarId> = VarId::all(SHOWN).collect();
    let mut a = ArjunLib::new(ArjunOptions::default().seed).expect("shim ctor");
    a.new_vars(num_vars);
    for c in &clauses {
        a.add_clause_dimacs(c);
    }
    a.set_sampl(&shown);
    // Long enough for the simplification before the search to finish, so the
    // deadline passes inside the search and not before it.
    let budget = Duration::from_secs(5);
    let started = Instant::now();
    a.set_deadline(started + budget);
    assert!(a.stage_minimize_indep(false), "minimize stage failed");
    let elapsed = started.elapsed();

    // The margin is for the work after the stop: the candidates still queued
    // are each kept with one more step of the search, then the stage reads
    // its result back.
    assert!(
        elapsed < budget + Duration::from_secs(10),
        "stage 1 returned after {elapsed:?} against a {budget:?} budget"
    );
    let mut kept = a.cur_sampl();
    kept.sort();
    assert_eq!(
        kept, shown,
        "stage 1 dropped a variable no other shown variable defines"
    );
}

#[test]
fn stage_one_leaves_a_quarter_of_the_time_left_to_stage_two() {
    let now = Instant::now();
    assert_eq!(
        stage_one_deadline(now, now + Duration::from_secs(20)),
        now + Duration::from_secs(15)
    );
    assert_eq!(
        stage_one_deadline(now, now),
        now,
        "no time left, none to share"
    );
}

/// What stage 1 shows determined leaves the formula only in stage 2, which is
/// why `run_stages` stops stage 1 early enough for stage 2 to run: a run whose
/// stage 1 met the deadline used to skip stage 2 and return its input
/// unchanged. Three AND gates (7 = 1 and 2, 8 = 3 and 4, 9 = 5 and 6) under two
/// clauses over their outputs: stage 1 proves the outputs determined but keeps
/// every variable, and stage 2 eliminates them.
#[test]
fn stage_two_is_what_removes_the_variables_stage_one_proved_determined() {
    const GATES: &[&[i32]] = &[
        &[-7, 1],
        &[-7, 2],
        &[7, -1, -2],
        &[-8, 3],
        &[-8, 4],
        &[8, -3, -4],
        &[-9, 5],
        &[-9, 6],
        &[9, -5, -6],
        &[7, 8, 9],
        &[-7, -8],
    ];
    let mut a = ArjunLib::new(ArjunOptions::default().seed).expect("shim ctor");
    a.new_vars(9);
    for c in GATES {
        a.add_clause_dimacs(c);
    }
    a.set_sampl(&VarId::all(9).collect::<Vec<_>>());
    assert!(a.stage_minimize_indep(true), "minimize stage failed");
    assert!(a.cur_sampl().len() < 9, "stage 1 should shrink the support");
    assert_eq!(a.cur_formula().num_vars(), 9, "stage 1 removes no variable");
    assert!(
        a.stage_simplify(true, false, true, false),
        "simplify stage failed"
    );
    assert!(
        a.cur_formula().num_vars() < 9,
        "stage 2 should eliminate the outputs"
    );
}
