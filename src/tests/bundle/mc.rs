use super::*;

#[test]
fn round_trip_backbone() {
    let rt = round_trip(
        "backbone",
        "p cnf 4 5\n\
         1 0\n\
         -2 0\n\
         -1 3 4 0\n\
         2 -3 4 0\n\
         3 -4 0\n",
    );
    rt.assert_sound();
    {
        rt.assert_reduced_below(4);
        assert!(
            !rt.record.forced_literals_original_dimacs.is_empty(),
            "the backbone case must record forced literals",
        );
    }
}

#[test]
fn round_trip_equivalences() {
    let rt = round_trip(
        "equiv",
        "p cnf 5 8\n\
         -1 2 0\n\
         1 -2 0\n\
         -3 -4 0\n\
         3 4 0\n\
         1 3 5 0\n\
         -1 -3 5 0\n\
         2 4 -5 0\n\
         -2 -4 -5 0\n",
    );
    rt.assert_sound();
    rt.assert_reduced_below(5);
}

#[test]
fn round_trip_free_vars() {
    let rt = round_trip(
        "free",
        "p cnf 5 3\n\
         1 2 0\n\
         -1 3 0\n\
         -2 -3 0\n",
    );
    rt.assert_sound();
    {
        assert!(
            rt.record.count_lift_pow2 >= 2,
            "two free vars must contribute at least 2^2, got 2^{}",
            rt.record.count_lift_pow2,
        );
        for v in [4u32, 5] {
            assert!(
                rt.record.free_vars_original_dimacs.contains(&v),
                "original var {v} occurs in no clause and must be listed as free, got {:?}",
                rt.record.free_vars_original_dimacs,
            );
        }
        rt.assert_reduced_below(5);
    }
}

#[test]
fn round_trip_mixed_reductions() {
    let rt = round_trip(
        "mixed",
        "p cnf 7 7\n\
         1 0\n\
         -2 3 0\n\
         2 -3 0\n\
         -1 2 4 0\n\
         -4 5 0\n\
         4 -5 0\n\
         -3 -5 0\n",
    );
    rt.assert_sound();
    rt.assert_reduced_below(7);
}

#[test]
fn round_trip_irreducible() {
    let rt = round_trip(
        "irreducible",
        "p cnf 3 4\n\
         1 2 3 0\n\
         -1 -2 3 0\n\
         1 -2 -3 0\n\
         -1 2 -3 0\n",
    );
    rt.assert_sound();
}

/// UNSAT, and specifically the empty-clause hazard: preprocessing derives the
/// empty clause, but DIMACS cannot portably spell one — a lone `0` line reads
/// back as a stray terminator by most parsers, which would turn an UNSAT
/// instance into a nonzero count on the write/re-parse path. The export must
/// therefore emit an explicit contradiction. This test failed on the first
/// implementation, reporting 8 models for an UNSAT instance; keep it as the
/// guard.
#[test]
fn round_trip_unsat() {
    let rt = round_trip(
        "unsat",
        "p cnf 3 4\n\
         1 0\n\
         -1 0\n\
         2 3 0\n\
         -2 -3 0\n",
    );
    assert_eq!(
        brute_force_mc(&rt.original),
        BigUint::ZERO,
        "the test instance must really be UNSAT",
    );
    rt.assert_sound();
    assert_eq!(
        brute_force_mc(&rt.reparsed),
        BigUint::ZERO,
        "the emitted reduced.cnf must still be UNSAT after a write/re-parse round trip",
    );
    {
        assert!(
            rt.record.unsat,
            "a proved-UNSAT run must be recorded as such"
        );
        assert!(
            !rt.reparsed.clauses.iter().any(|c| c.literals.is_empty()),
            "the empty clause must never be written — it does not survive DIMACS",
        );
    }
}

/// The same hazard on the way IN: the empty clause is the input file's own
/// refutation, written as a bare `0`, rather than something preprocessing
/// derived. Preprocessing must carry it through to `unsat` — stepping over the
/// `0` at parse reported models for an instance that has none.
#[test]
fn round_trip_unsat_from_a_bare_zero_clause() {
    let rt = round_trip(
        "unsat-bare-zero",
        "p cnf 3 3\n\
         1 2 0\n\
         0\n\
         -1 3 0\n",
    );
    assert_eq!(
        brute_force_mc(&rt.original),
        BigUint::ZERO,
        "a formula carrying the empty clause has no models",
    );
    rt.assert_sound();
    assert!(
        rt.record.unsat,
        "the input's own empty clause must be recorded as UNSAT",
    );
    assert_eq!(
        brute_force_mc(&rt.reparsed),
        BigUint::ZERO,
        "the emitted reduced.cnf must still be UNSAT after a write/re-parse round trip",
    );
}
