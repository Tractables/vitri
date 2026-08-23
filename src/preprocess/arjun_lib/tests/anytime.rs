use super::*;
use crate::tests::common::{clause_dimacs, lit};
use crate::tests::pmc_oracle::brute_force_mc;

/// The knob set [`reduce_anytime`] settles before it forks, resolved the same
/// way here so an inline call is the same reduction as the forked one. No
/// `VITRI_*` variable is set in these tests, so every knob takes its default
/// and SBVA stays on.
fn default_knobs() -> FullCountKnobs {
    FullCountKnobs::resolve(/*force_no_sbva=*/ false).expect("no VITRI_* knob is set in this test")
}

#[test]
fn anytime_reduces_toy_cnf() {
    let mut f = CnfFormula {
        num_vars: 10,
        clauses: Vec::new(),
    };
    for c in [
        &[1, 2][..],
        &[1, 2, 9],
        &[3, -4],
        &[3, -4, 5],
        &[-1, 6],
        &[6, -2],
    ] {
        f.clauses.push(clause_dimacs(c));
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    let r = reduce_anytime(&f, deadline, ArjunEffort::Full, ArjunOptions::default())
        .expect("no VITRI_* knob is set in this test")
        .expect("reduce");
    assert!(r.formula.num_vars <= 10);
    assert!(r.multiplier_exp > 0, "exp={}", r.multiplier_exp);
}

/// A formula that gives Arjun real work to do, so a tight deadline can
/// actually land inside a stage rather than before stage 1. Small enough
/// to brute-force count (n <= 20), structured enough (chains + a parity
/// ladder + free vars) that BVE/SBVA/oracle all have something to chew on.
fn deadline_probe_formula() -> CnfFormula {
    let mut clauses = Vec::new();
    // Implication chain 0→1→…→11 (BVE bait).
    for v in 0..11u32 {
        clauses.push(Clause::new(vec![lit(v, false), lit(v + 1, true)]));
    }
    // A ladder of ternary clauses over the same vars (oracle/vivify bait).
    for v in 0..10u32 {
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
    // vars 12..16 appear in no clause at all ⇒ folded into the multiplier.
    CnfFormula {
        num_vars: 16,
        clauses,
    }
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
        ArjunEffort::Full,
        false,
        default_knobs(),
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
        ArjunEffort::Full,
        false,
        default_knobs(),
    )
    .expect("reduce with 600s budget");
    let b = reduce_anytime_inner(
        &formula,
        Instant::now() + Duration::from_secs(4200),
        ArjunEffort::Full,
        false,
        default_knobs(),
    )
    .expect("reduce with 4200s budget");
    assert_eq!(
        a.formula, b.formula,
        "far-future deadline changed the reduction"
    );
    assert_eq!(a.multiplier_exp, b.multiplier_exp);
    assert_eq!(a.backbone, b.backbone);
    assert_eq!(a.equiv, b.equiv);
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
    let formula = CnfFormula {
        num_vars: 6,
        clauses: vec![
            Clause::new(vec![lit(0, true)]),
            Clause::new(vec![lit(0, false), lit(1, true)]),
            Clause::new(vec![lit(1, false), lit(2, true)]),
            Clause::new(vec![lit(3, true), lit(4, false)]),
            Clause::new(vec![lit(3, false), lit(4, true)]),
        ],
    };
    let budget = Duration::from_secs(30);
    let forked = reduce_anytime(
        &formula,
        Instant::now() + budget,
        ArjunEffort::Full,
        ArjunOptions::default(),
    )
    .expect("no VITRI_* knob is set in this test")
    .expect("forked reduce");
    let direct = reduce_anytime_inner(
        &formula,
        Instant::now() + budget,
        ArjunEffort::Full,
        false,
        default_knobs(),
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
        formula.num_vars as usize,
        "the map must be indexed by input variable",
    );
    let mut claimed = vec![false; forked.formula.num_vars as usize];
    for e in forked.input_to_reduced_lit.iter().flatten() {
        let r = e.unsigned_abs() as usize;
        assert!(
            r >= 1 && r <= forked.formula.num_vars as usize,
            "map names reduced var {r}, outside 1..={}",
            forked.formula.num_vars,
        );
        assert!(!claimed[r - 1], "two input vars map onto reduced var {r}");
        claimed[r - 1] = true;
    }
}
