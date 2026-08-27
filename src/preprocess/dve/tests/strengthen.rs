//! The strengthening pass that runs after elimination.
//!
//! It rewrites clauses in a formula that already carries elimination
//! provenance, so it must leave that provenance consistent and must not
//! touch frozen variables.

use super::*;

/// Regression: `post_dve_strengthen`'s second DVE pass must keep the per-variable
/// fates consistent with the final formula, and must NOT eliminate frozen
/// variables.
///
/// The bug (fixed): the inner pass updated `renumbering`/`formula`/`stats` but
/// left the fates reflecting only the FIRST pass, and ran with an EMPTY frozen
/// set. The weighted-DVE guard/correction walk them per var, so a dropped inner
/// elimination silently lost a weight-correction factor (and could eliminate an
/// unequal-weight var freeze was protecting) — wrong weighted counts (MCC AMC
/// instance 163 was ~3.9x too large until this fix).
fn make_strengthen_scenario() -> crate::preprocess::dve::types::DveResult {
    use crate::preprocess::dve::types::{DveFate, DveResult};
    // Simulate a first DVE pass over 5 original vars that eliminated original
    // vars 0,1 (as free) and renumbered survivors {2,3,4} -> local {0,1,2}.
    // The local formula carries an AND gate local2 <-> (local0 & local1) that the
    // first pass left in place (exposed only after renumbering).
    let formula = make_formula(
        3,
        vec![
            vec![-3, 1],     // (¬o ∨ a)
            vec![-3, 2],     // (¬o ∨ b)
            vec![3, -1, -2], // (o ∨ ¬a ∨ ¬b)   ⇒ o ↔ (a ∧ b), o = local2
            vec![1, 2],      // (a ∨ b)  keep a,b
        ],
    );
    DveResult {
        formula,
        definition_clauses: Vec::new(),
        // local -> original
        renumbering: Some(crate::preprocess::renumber::Renumber::of_kept(
            5,
            [VarId(2), VarId(3), VarId(4)],
        )),
        // original 0,1 eliminated as free
        fates: vec![
            DveFate::Free,
            DveFate::Free,
            DveFate::Kept,
            DveFate::Kept,
            DveFate::Kept,
        ],
        elapsed_ms: 0,
    }
}

#[test]
fn post_dve_strengthen_keeps_provenance_consistent() {
    let mut dve = make_strengthen_scenario();
    crate::preprocess::dve::post_dve_strengthen(&mut dve, &rustc_hash::FxHashSet::default());
    // The inner pass must have fired (gate output eliminated) — otherwise this
    // test guards nothing; fail loudly so a detect_gates change is noticed.
    assert!(
        dve.formula.num_vars < 3,
        "inner strengthen pass did not eliminate the gate output (num_vars={}); \
         test no longer exercises the bug",
        dve.formula.num_vars
    );
    // Core invariant the bug violated: every survivor accounted for.
    let elim_count = dve.total_eliminated();
    assert_eq!(
        elim_count,
        dve.original_num_vars() - dve.formula.num_vars as usize,
        "elimination provenance ({} eliminated) inconsistent with final formula \
         ({} of {} survive): post_dve_strengthen dropped the inner pass's \
         eliminations from the per-var fates",
        elim_count,
        dve.formula.num_vars,
        dve.original_num_vars(),
    );
    assert!(dve.fates[0].eliminated() && dve.fates[1].eliminated());
    assert!(
        dve.fates[4].eliminated(),
        "inner-pass elimination of original var 4 not recorded"
    );
}

#[test]
fn post_dve_strengthen_respects_frozen() {
    // Freeze the gate output (original var 4 == local 2). The inner pass must NOT
    // eliminate it, even though it is a clean gate the pass would otherwise peel.
    let mut frozen: rustc_hash::FxHashSet<VarId> = rustc_hash::FxHashSet::default();
    frozen.insert(VarId(4));
    let mut dve = make_strengthen_scenario();
    crate::preprocess::dve::post_dve_strengthen(&mut dve, &frozen);
    assert!(
        !dve.fates[4].eliminated(),
        "frozen original var 4 was eliminated by the post-DVE strengthen pass"
    );
}

/// A formula whose only reduction under a frozen CaDiCaL round is clause
/// strengthening: `(a∨b∨c)` and `(a∨b∨¬c)` self-subsuming-resolve to `(a∨b)`,
/// which then subsumes both. Every variable occurs, so freezing blocks nothing
/// the round needs, which is what makes the round observable at all.
fn strengthenable_formula() -> crate::cnf::CnfFormula {
    make_formula(
        5,
        vec![
            vec![1, 2, 3],
            vec![1, 2, -3],
            vec![-1, 4],
            vec![-2, -4, 5],
            vec![3, 4, -5],
            vec![-3, -4, 5],
        ],
    )
}

/// The vivification round is bounded by the stage wall it was handed.
///
/// `strengthen_clauses` runs a CaDiCaL round that polls no clock of its own, and
/// it used to be called with no terminator at all, on the premise that the DVE
/// budget already bounded it. That budget is checked between rounds, so it only
/// decides whether a round starts.
///
/// The invariant, stated without reference to what CaDiCaL finds: a wall that
/// has already gone leaves the clause set exactly as it was, while the same call
/// with no wall reduces it.
#[test]
fn strengthening_is_cut_by_the_stage_wall_it_was_handed() {
    use crate::preprocess::dve::strengthen::strengthen_clauses;

    let f = strengthenable_formula();

    // The unbounded round, which is what the bounded one must not do once its
    // wall has gone. Fail loudly if it stops reducing this fixture — the test
    // would otherwise pass while guarding nothing.
    let mut unbounded = f.clauses.clone();
    assert!(
        strengthen_clauses(&mut unbounded, f.num_vars as usize, None),
        "the fixture is no longer strengthened at all; this test guards nothing"
    );

    // The same round under a wall that is already gone.
    let past = std::time::Instant::now() - std::time::Duration::from_secs(1);
    let mut cut = f.clauses.clone();
    let changed = strengthen_clauses(&mut cut, f.num_vars as usize, Some(past));
    assert!(
        !changed,
        "a vivification round with no time left strengthened anyway — the stage wall did not \
         reach CaDiCaL's terminator"
    );
    // Degrades to the reduction reached so far, which with no time is the input:
    // never a partial or reordered clause set.
    assert_eq!(
        cut, f.clauses,
        "a cut round returned something other than its input"
    );
}

/// A wall the round cannot reach changes nothing. The bound is half of what is
/// left of the stage wall, so a stage wall an hour out is a round that finishes
/// long before its terminator, matching the unbounded round exactly.
#[test]
fn strengthening_under_a_generous_stage_wall_matches_the_unbounded_round() {
    use crate::preprocess::dve::strengthen::strengthen_clauses;

    let f = strengthenable_formula();

    let mut unbounded = f.clauses.clone();
    let a = strengthen_clauses(&mut unbounded, f.num_vars as usize, None);

    let far = std::time::Instant::now() + std::time::Duration::from_secs(3_600);
    let mut bounded = f.clauses.clone();
    let b = strengthen_clauses(&mut bounded, f.num_vars as usize, Some(far));

    assert_eq!(
        a, b,
        "a wall it cannot reach changed whether the round reduced"
    );
    assert_eq!(
        bounded, unbounded,
        "a wall it cannot reach changed the reduction"
    );
}
