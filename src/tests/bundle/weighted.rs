//! Weighted cases: exact rational weights through the whole chain.
//!
//! Non-dyadic weights are deliberate — a float or power-of-two shortcut
//! anywhere in the lift fails the exact comparison rather than rounding to
//! something that looks right.

use super::*;

#[test]
fn round_trip_weighted_non_dyadic() {
    let rt = round_trip(
        "wmc",
        "c t wmc\n\
         p cnf 4 4\n\
         c p weight 1 1/3 0\n\
         c p weight -1 2/3 0\n\
         c p weight 2 5/7 0\n\
         c p weight -2 3/11 0\n\
         c p weight 3 2 0\n\
         c p weight -3 1/5 0\n\
         1 2 0\n\
         -1 3 0\n\
         -2 -3 4 0\n\
         2 3 -4 0\n",
    );
    rt.assert_sound();
}

/// Weighted, with a backbone and a free variable: the two factors a cardinality
/// lift gets wrong. A forced literal contributes `w[polarity]` (not 1), an
/// free variable contributes `w⁻ + w⁺` (not 2), and here neither number
/// coincides with the integer one.
#[test]
fn round_trip_weighted_backbone_and_free() {
    let rt = round_trip(
        "wmc-backbone",
        "c t wmc\n\
         p cnf 5 4\n\
         c p weight 1 3/5 0\n\
         c p weight -1 7/5 0\n\
         c p weight 4 1/3 0\n\
         c p weight -4 1/7 0\n\
         c p weight 2 2/9 0\n\
         c p weight -2 5/9 0\n\
         1 0\n\
         -1 2 3 0\n\
         -2 3 0\n\
         2 -3 0\n",
    );
    rt.assert_sound();
    // v1 owes w⁺ = 3/5 and v4 owes w⁻ + w⁺ = 1/7 + 1/3 — neither expressible as
    // a 2^k exponent, so the lift must be a real factor, not the unweighted 1/1.
    assert_ne!(
        rt.record.weight_lift, "1/1",
        "a forced literal and a free variable both owe a weighted factor",
    );
}

/// The weighted twin of
/// [`projected_without_arjun_keeps_the_original_show_set`](super::projected):
/// with the renumbering stage off, the table the record carries is read as a
/// reduced-space one although it was declared over the input. Nothing renumbered,
/// so the two coincide — and a stage that renumbered anyway would show up here as
/// a permuted table, which the lift identity would still accept.
#[test]
fn weighted_without_arjun_keeps_the_original_weights() {
    let rt = round_trip_with(
        "pwmc-no-arjun",
        "c t pwmc\n\
         p cnf 5 5\n\
         c p show 2 4 5 0\n\
         c p weight 2 5/7 0\n\
         c p weight -2 3/11 0\n\
         c p weight 4 2 0\n\
         c p weight -4 1/5 0\n\
         1 2 0\n\
         -1 3 0\n\
         -2 -3 4 0\n\
         2 3 -4 0\n\
         4 5 0\n",
        &RunConfig {
            mode: Some(Mode::Pwmc),
            stages: crate::config::PreprocessStages {
                arjun: false,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    rt.assert_sound();
    assert_eq!(
        rt.reduced_weights(),
        rt.original_weights.clone().assume_reduced_identity(),
        "with no renumbering stage the emitted table is the declared one; record = {}",
        rt.record.to_json_string(),
    );
}

/// Projected AND weighted: only the show variables carry weights (the track's own
/// semantics — a projected-out variable contributes existence, not weight), and
/// preprocessing must preserve that.
#[test]
fn round_trip_projected_weighted() {
    let rt = round_trip(
        "pwmc",
        "c t pwmc\n\
         p cnf 5 5\n\
         c p show 1 2 3 0\n\
         c p weight 1 1/3 0\n\
         c p weight -1 2/3 0\n\
         c p weight 2 5/7 0\n\
         c p weight -2 3/11 0\n\
         c p weight 3 2 0\n\
         c p weight -3 1/5 0\n\
         1 2 0\n\
         -1 4 0\n\
         -2 -4 5 0\n\
         2 4 -5 0\n\
         3 5 0\n",
    );
    rt.assert_sound();
}

/// The weighted Arjun-only checkpoint carries the reduced weights and rational
/// lift through the same record assembly as the full projected chain.
#[test]
fn round_trip_projected_weighted_arjun_only_checkpoint() {
    let rt = round_trip_with(
        "pwmc-arjun-only",
        "c t pwmc\n\
         p cnf 5 5\n\
         c p show 1 2 3 0\n\
         c p weight 1 1/3 0\n\
         c p weight -1 2/3 0\n\
         c p weight 2 5/7 0\n\
         c p weight -2 3/11 0\n\
         c p weight 3 2 0\n\
         c p weight -3 1/5 0\n\
         1 2 0\n\
         -1 4 0\n\
         -2 -4 5 0\n\
         2 4 -5 0\n\
         3 5 0\n",
        &RunConfig {
            mode: Some(Mode::Pwmc),
            projection_policy: crate::config::ProjectionPolicy::ArjunOnly(
                crate::config::ProjectionNoGain::KeepSound,
            ),
            ..RunConfig::default()
        },
    );
    rt.assert_sound();
}

/// A `pwmc` round trip whose Arjun reduction was kept, after checking the
/// input's brute-force count against the value worked out by hand for it.
///
/// The cases below name ONE literal of a weighted variable. Its other literal
/// weighs 1, as for any literal no `c p weight` line names, and the reduction
/// has to be told so: left to itself it would weigh that literal `1 - w`.
fn kept_projected_weighted(tag: &str, dimacs: &str, count: BigRational) -> RoundTrip {
    let rt = round_trip(tag, dimacs);
    assert_eq!(
        rt.original_count(),
        count,
        "the unnamed literal weighs 1 in the brute-force count",
    );
    assert_eq!(
        rt.stages.arjun,
        Some(StageOutcome::Ran),
        "the case is about the weights Arjun is handed, so its reduction must be the one kept",
    );
    rt.assert_sound();
    rt
}

/// `w(x) + w(¬x) = 3/10 + 1`.
#[test]
fn a_projected_weight_named_on_a_positive_literal_leaves_the_negative_at_one() {
    kept_projected_weighted(
        "pwmc-positive-only",
        "c t pwmc\n\
         p cnf 1 0\n\
         c p show 1 0\n\
         c p weight 1 3/10 0\n",
        rat(13, 10),
    );
}

/// `w(x) + w(¬x) = 1 + 3/10`.
#[test]
fn a_projected_weight_named_on_a_negative_literal_leaves_the_positive_at_one() {
    kept_projected_weighted(
        "pwmc-negative-only",
        "c t pwmc\n\
         p cnf 1 0\n\
         c p show 1 0\n\
         c p weight -1 3/10 0\n",
        rat(13, 10),
    );
}

/// A literal named at weight 1 is still a named literal: `w(x) + w(¬x) = 1 + 1`.
#[test]
fn a_projected_weight_of_one_named_on_one_literal_leaves_the_other_at_one() {
    kept_projected_weighted(
        "pwmc-named-one",
        "c t pwmc\n\
         p cnf 1 0\n\
         c p show 1 0\n\
         c p weight 1 1 0\n",
        rat(2, 1),
    );
}

/// `v2` is projected out, and with it gone `v1` is free: `3/10 + 1`.
#[test]
fn a_one_literal_weight_on_a_show_variable_over_a_projected_one_leaves_the_other_at_one() {
    kept_projected_weighted(
        "pwmc-one-literal-over-projected",
        "c t pwmc\n\
         p cnf 2 1\n\
         c p show 1 0\n\
         c p weight 1 3/10 0\n\
         1 2 0\n",
        rat(13, 10),
    );
}

/// The show variables stay in the reduced formula, so their unnamed literals
/// reach the count through the reduced weights rather than through the lift.
/// The show projections are the models of `v1 ∨ v2`: `1/2·1/4 + 1·1 + 1/2·1`.
#[test]
fn one_literal_weights_on_show_variables_that_survive_the_reduction_leave_the_others_at_one() {
    let rt = kept_projected_weighted(
        "pwmc-one-literal-survivors",
        "c t pwmc\n\
         p cnf 3 2\n\
         c p show 1 2 0\n\
         c p weight 1 1/2 0\n\
         c p weight -2 1/4 0\n\
         1 2 0\n\
         -1 3 0\n",
        rat(13, 8),
    );
    assert_eq!(
        rt.reduced_show().map(|show| show.len()),
        Some(2),
        "both show variables must survive, or the reduced weights are not under test; record = {}",
        rt.record.to_json_string(),
    );
}

#[test]
fn round_trip_weighted_unsat() {
    let rt = round_trip(
        "wmc-unsat",
        "c t wmc\n\
         p cnf 3 4\n\
         c p weight 1 1/3 0\n\
         c p weight -1 2/3 0\n\
         1 0\n\
         -1 0\n\
         2 3 0\n\
         -2 -3 0\n",
    );
    rt.assert_sound();
}
