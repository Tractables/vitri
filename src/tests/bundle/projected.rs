//! Projected cases: the show set has to reach the written file and the
//! record in REDUCED numbering, and the projected count has to survive the
//! lift.

use super::*;

/// Arjun REWRITES the show set (dropping variables that are free or determined
/// given the rest) and renumbers the formula under it, so emitting the input's
/// own show ids beside a renumbered formula would not be a weaker answer — it
/// would be a silently wrong count. That composition is what this case pins.
#[test]
fn round_trip_projected_show_set() {
    let rt = round_trip(
        "show",
        "c t pmc\n\
         p cnf 5 5\n\
         c p show 2 4 5 0\n\
         1 2 0\n\
         -1 3 0\n\
         -2 -3 4 0\n\
         2 3 -4 0\n\
         4 5 0\n",
    );
    rt.assert_sound();

    let orig_show = rt.original_show.clone().expect("input declares a show set");
    assert_ne!(
        brute_force_pmc(&rt.original, &orig_show),
        brute_force_mc(&rt.original),
        "the test instance must have a projected count distinct from its plain count",
    );
}

/// A projected instance whose show set is the WHOLE variable set: the projected
/// count then equals the plain one, and the chain must still hold the identity
/// without accidentally taking the count-preserving shortcut.
#[test]
fn round_trip_projected_full_show_set() {
    let rt = round_trip(
        "show-full",
        "c t pmc\n\
         p cnf 4 4\n\
         c p show 1 2 3 4 0\n\
         1 2 0\n\
         -1 3 0\n\
         -2 -3 4 0\n\
         2 3 -4 0\n",
    );
    rt.assert_sound();
    let orig_show = rt.original_show.clone().unwrap();
    assert_eq!(
        brute_force_pmc(&rt.original, &orig_show),
        brute_force_mc(&rt.original),
        "a full show set means the projected count IS the plain count",
    );
}

/// The projected chain over a formula whose hidden half is a Tseitin definition:
/// exactly the structure the show-frozen strengthening pass exists to eliminate.
#[test]
fn round_trip_projected_definitions() {
    let rt = round_trip(
        "show-defs",
        "c t pmc\n\
         p cnf 5 8\n\
         c p show 1 2 3 0\n\
         -4 1 0\n\
         -4 2 0\n\
         4 -1 -2 0\n\
         5 -3 0\n\
         5 -4 0\n\
         -5 3 4 0\n\
         1 3 0\n\
         -1 -3 5 0\n",
    );
    rt.assert_sound();
}

/// The Arjun-only policy is an export checkpoint, not a parallel bundle path:
/// the post-Arjun formula, rewritten show set, lift and map must still satisfy
/// the same round-trip contract as the complete projected chain.
#[test]
fn round_trip_projected_arjun_only_checkpoint() {
    let rt = round_trip_with(
        "show-arjun-only",
        "c t pmc\n\
         p cnf 5 5\n\
         c p show 2 4 5 0\n\
         1 2 0\n\
         -1 3 0\n\
         -2 -3 4 0\n\
         2 3 -4 0\n\
         4 5 0\n",
        &RunConfig {
            mode: Some(Mode::Pmc),
            projection_policy: crate::config::ProjectionPolicy::ArjunOnly(
                crate::config::ProjectionNoGain::KeepSound,
            ),
            ..RunConfig::default()
        },
    );
    rt.assert_sound();
}

/// With the Arjun stage off, nothing renumbers: the projected reduction that follows it
/// preserves ids, so the input's own show ids are already the reduced formula's.
/// That is an ASSUMPTION the chain makes rather than a map it consults, which is
/// exactly the kind of claim that stops being true quietly — a stage gaining a
/// renumbering step would leave this path emitting the input's ids beside a
/// formula they no longer index.
#[test]
fn projected_without_arjun_keeps_the_original_show_set() {
    let rt = round_trip_with(
        "show-no-arjun",
        "c t pmc\n\
         p cnf 5 5\n\
         c p show 2 4 5 0\n\
         1 2 0\n\
         -1 3 0\n\
         -2 -3 4 0\n\
         2 3 -4 0\n\
         4 5 0\n",
        &RunConfig {
            mode: Some(Mode::Pmc),
            stages: crate::config::PreprocessStages {
                arjun: false,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    rt.assert_sound();
    assert_eq!(
        rt.reduced_show(),
        rt.original_show,
        "with no renumbering stage the emitted show set is the declared one; record = {}",
        rt.record.to_json_string(),
    );
}

/// Preprocessing can REFUTE a projected instance, and the bundle it writes then
/// describes a contradiction over the ORIGINAL variables — but the field it
/// writes the show set into is a reduced-space field. The two coincide only
/// because that bundle renumbers nothing, so the set must still be there, still
/// inside the emitted formula's range, and still identical to the `c p show`
/// line beside it: a refuted bundle that dropped the projection would leave a
/// consumer unable to tell a projected refutation from a plain one.
#[test]
fn a_refuted_projected_instance_still_records_a_show_set() {
    let rt = round_trip(
        "show-unsat",
        "c t pmc\n\
         p cnf 4 5\n\
         c p show 1 3 0\n\
         1 2 0\n\
         -2 3 0\n\
         -3 4 0\n\
         2 0\n\
         -2 0\n",
    );
    assert_eq!(
        brute_force_pmc(&rt.original, &[0, 2]),
        num_bigint::BigUint::ZERO,
        "the test instance must really be refuted",
    );
    rt.assert_sound();
    assert!(
        rt.record.unsat,
        "a proved-UNSAT run must be recorded as such"
    );

    let recorded = rt
        .record
        .show_vars_reduced_dimacs
        .as_ref()
        .expect("a refuted projected bundle must still record its show set")
        .to_dimacs();
    for v in &recorded {
        assert!(
            *v >= 1 && *v <= rt.reparsed.num_vars,
            "show var {v} is outside the emitted space 1..={}",
            rt.reparsed.num_vars,
        );
    }
    assert_eq!(
        recorded,
        rt.reduced_show()
            .expect("the emitted contradiction must carry a `c p show` line")
            .iter()
            .map(|v| v + 1)
            .collect::<Vec<u32>>(),
        "the record and the emitted `c p show` line must agree on a refutation too",
    );
}
