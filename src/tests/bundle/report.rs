//! What the bundle says about the RUN rather than about the lift: which stage
//! did what, which stage earned which half of the count lift, and the formula
//! the Arjun stage was handed.
//!
//! These are the fields a caller reads to decide what to do NEXT — call again
//! on a bigger budget, re-reduce a formula derived from this one — so each case
//! below is written as that decision rather than as a field read.

use super::*;

use crate::config::PreprocessStages;
use crate::preprocess::OriginalTarget;
use crate::tests::common::IRREDUCIBLE_5;

/// A backbone (`v1`), two definitions (`v5`, `v6`) and a variable no clause
/// mentions (`v7`): both the simplify chain and the Arjun stage have something
/// to remove, and the free variable is the one whose `×2` the lift is made of.
const FREE_VAR_AND_DEFINITIONS: &str = "p cnf 7 9\n\
     1 0\n\
     -5 2 0\n\
     -5 3 0\n\
     5 -2 -3 0\n\
     -6 4 0\n\
     6 -4 0\n\
     2 3 4 0\n\
     -5 -6 0\n\
     -1 5 6 0\n";

/// [`IRREDUCIBLE_5`] under a projected mode, showing two of its variables.
/// Nothing here lets the projection-set minimization retire a show variable, so
/// its reduction is the one the keep-gate refuses.
const PROJECTION_ALREADY_MINIMAL: &str =
    "c t pmc\np cnf 5 5\nc p show 1 2 0\n1 2 0\n-1 3 0\n-2 -3 4 0\n2 3 -4 0\n4 5 0\n";

/// `v1 ≡ ¬v2` (both binary clauses), inside a formula the equivalence does not
/// settle on its own — so one of the pair survives the reduction and the other
/// is named through it, NEGATED.
const ANTI_EQUIVALENCE: &str = "p cnf 4 6\n\
     1 2 0\n\
     -1 -2 0\n\
     1 3 0\n\
     -3 4 0\n\
     2 4 0\n\
     -1 -3 -4 0\n";

/// One unique backbone (`x1`) and one unique equivalence (`x3 ≡ x4`), with
/// enough remaining freedom that probing has candidates to discharge.
const PROBE_TELEMETRY: &str = "p cnf 5 5\n\
     1 2 0\n\
     1 -2 0\n\
     -3 4 0\n\
     3 -4 0\n\
     3 5 0\n";

/// Preprocess `dimacs` under `config`, naming the mode in the failure.
fn bundle_of(dimacs: &str, config: &RunConfig) -> PreprocessBundle {
    let (formula, meta) = parse(dimacs);
    preprocess(&formula, &meta, config).expect("preprocessing must run")
}

/// `mc`, with everything else left at its default.
fn counting() -> RunConfig {
    RunConfig {
        mode: Some(Mode::Mc),
        ..RunConfig::default()
    }
}

/// **The distinction the whole report exists for.** A caller that gets no
/// reduction has one question: is calling again with more wall worth anything?
/// A stage that ran out of budget may well answer differently next time; one
/// whose result was refused refuses it again on the same input. Reporting both
/// as "no reduction" would leave the caller guessing, so they are separate
/// outcomes and this pins that they stay separate — and that each means what it
/// says.
#[test]
fn a_stage_that_ran_out_of_budget_is_not_reported_as_one_that_was_refused() {
    let expired = RunConfig {
        deadline: Some(std::time::Instant::now() - std::time::Duration::from_secs(1)),
        ..counting()
    };
    assert_eq!(
        bundle_of(IRREDUCIBLE_5, &expired).stages.arjun,
        Some(StageOutcome::GaveUp),
        "a deadline already in the past leaves the stage no budget to produce \
         anything in, which is a give-up and not a judgement about the formula",
    );
    assert_eq!(
        bundle_of(IRREDUCIBLE_5, &counting()).stages.arjun,
        Some(StageOutcome::Ran),
        "the same formula on a real budget reduces — which is what makes the \
         give-up above worth calling again on",
    );

    // A projected reduction that leaves the show set exactly as it found it is
    // refused: the run bought no counting benefit and can compile worse than
    // the formula it started from.
    for budget in [None, Some(600_000)] {
        let config = RunConfig {
            mode: Some(Mode::Pmc),
            budget_ms: budget,
            ..RunConfig::default()
        };
        assert_eq!(
            bundle_of(PROJECTION_ALREADY_MINIMAL, &config).stages.arjun,
            Some(StageOutcome::Discarded(DiscardReason::NoProjectionGain)),
            "the refusal is about the formula, so more budget buys the caller \
             the same answer rather than a different one",
        );
    }
}

/// Turning a stage off is not the same as it having nothing to do, and neither
/// is a stage this mode's chain does not have: three different reports, because
/// only one of them changes if the caller changes its mind.
#[test]
fn a_stage_that_was_never_asked_for_says_so() {
    let off = RunConfig {
        stages: PreprocessStages {
            simplify: false,
            arjun: false,
        },
        ..counting()
    };
    let bundle = bundle_of(IRREDUCIBLE_5, &off);
    assert_eq!(
        bundle.stages.simplify,
        Some(StageOutcome::Skipped(SkipReason::NotRequested)),
    );
    assert_eq!(
        bundle.stages.arjun,
        Some(StageOutcome::Skipped(SkipReason::NotRequested)),
    );
    assert_eq!(
        bundle.stages.sbva, None,
        "bounded variable addition is part of the reduction, so with no \
         reduction there is nothing for it to report",
    );
    assert_eq!(bundle.telemetry.simplify_ms, None);
    assert_eq!(bundle.telemetry.backbone_ms, None);
    assert_eq!(bundle.telemetry.equivalence_ms, None);
    assert_eq!(bundle.telemetry.dve_ms, None);
    assert_eq!(bundle.telemetry.arjun_ms, None);

    let compile = RunConfig {
        mode: Some(Mode::Compile),
        ..RunConfig::default()
    };
    let bundle = bundle_of(IRREDUCIBLE_5, &compile);
    assert_eq!(bundle.stages.simplify, Some(StageOutcome::Ran));
    assert!(bundle.telemetry.simplify_ms.is_some());
    assert!(bundle.telemetry.backbone_ms.is_some());
    assert!(bundle.telemetry.equivalence_ms.is_some());
    assert_eq!(bundle.telemetry.dve_ms, None);
    assert_eq!(bundle.telemetry.arjun_ms, None);
    assert_eq!(
        bundle.stages.arjun, None,
        "the compile chain has no Arjun stage at all, which is not the same as \
         one that was turned off",
    );
}

/// The projected chains run Arjun FIRST and have no simplify chain at all, so a
/// stage report from one of them names a different set of stages — the absence
/// is the shape of the chain, not a stage that declined.
#[test]
fn a_projected_run_reports_no_simplify_stage_because_its_chain_has_none() {
    let config = RunConfig {
        mode: Some(Mode::Pmc),
        ..RunConfig::default()
    };
    let bundle = bundle_of(PROJECTION_ALREADY_MINIMAL, &config);
    assert_eq!(bundle.stages.simplify, None);
    assert_eq!(bundle.telemetry.simplify_ms, None);
    assert_eq!(bundle.telemetry.backbone_ms, None);
    assert_eq!(bundle.telemetry.equivalence_ms, None);
    assert_eq!(bundle.telemetry.dve_ms, None);
    assert!(bundle.telemetry.arjun_ms.is_some());
    assert!(
        bundle.stages.arjun.is_some(),
        "the projected chain's first stage is Arjun, so it always has one to \
         report on",
    );
}

/// The public telemetry shape carries phase presence independently of whether
/// a duration rounded to zero, plus the probing counts from the one probe run.
#[test]
fn preprocessing_telemetry_reports_attempted_phases_and_probe_counts() {
    let config = RunConfig {
        stages: PreprocessStages {
            simplify: true,
            arjun: false,
        },
        ..counting()
    };
    let bundle = bundle_of(PROBE_TELEMETRY, &config);
    let telemetry = bundle.telemetry;

    let _: u64 = telemetry.total_ms;
    assert!(telemetry.simplify_ms.is_some());
    assert!(telemetry.backbone_ms.is_some());
    assert!(telemetry.equivalence_ms.is_some());
    assert!(telemetry.dve_ms.is_some());
    assert_eq!(telemetry.arjun_ms, None);
    assert_eq!(telemetry.backbone_found, 1);
    assert!(
        telemetry.backbone_probes > 0,
        "the fixture leaves non-backbone candidates for the probe loop",
    );
}

/// The split is an ATTRIBUTION, not a decomposition of a number: the same
/// formula reduced by different stages puts the same lift in different halves.
/// A caller re-reducing a formula derived from this one reconciles against the
/// Arjun half alone, so which half a variable landed in is the whole point.
#[test]
fn the_lift_is_attributed_to_the_stage_that_actually_earned_it() {
    let both = bundle_of(FREE_VAR_AND_DEFINITIONS, &counting());
    assert!(
        both.count_lift.simplify_pow2 > 0,
        "the free variable is the simplify chain's to remove when it runs",
    );

    let arjun_only = RunConfig {
        stages: PreprocessStages {
            simplify: false,
            arjun: true,
        },
        ..counting()
    };
    let arjun_only = bundle_of(FREE_VAR_AND_DEFINITIONS, &arjun_only);
    assert_eq!(
        arjun_only.count_lift.simplify_pow2, 0,
        "a chain whose simplify stage never ran cannot have earned a lift with it",
    );
    assert!(
        arjun_only.count_lift.arjun_pow2 > 0,
        "the same free variable is still gone, and now it is Arjun that removed it",
    );
}

/// The split and the number written to disk are one value, so a caller that
/// lifts through the record and one that lifts through the split agree.
#[test]
fn the_split_lift_totals_the_recorded_one() {
    for dimacs in [FREE_VAR_AND_DEFINITIONS, IRREDUCIBLE_5, ANTI_EQUIVALENCE] {
        let bundle = bundle_of(dimacs, &counting());
        assert_eq!(
            bundle.count_lift.total_pow2(),
            bundle.record.count_lift_pow2,
            "the halves must add up to the exponent the record lifts by",
        );
    }
}

/// A weighted run's lift is a rational, and splitting a rational across stages
/// as a power of two would be a lie in whichever half was not one. Both halves
/// stay zero and the record's weight lift carries all of it.
#[test]
fn a_weighted_run_has_no_power_of_two_lift_to_split() {
    let config = RunConfig {
        mode: Some(Mode::Wmc),
        ..RunConfig::default()
    };
    let bundle = bundle_of(
        "c t wmc\np cnf 7 9\nc p weight 1 1/3 0\nc p weight -1 2/3 0\n\
         1 0\n-5 2 0\n-5 3 0\n5 -2 -3 0\n-6 4 0\n6 -4 0\n2 3 4 0\n-5 -6 0\n-1 5 6 0\n",
        &config,
    );
    assert_eq!(bundle.count_lift, CountLift::default());
    assert_eq!(
        bundle.count_lift.total_pow2(),
        bundle.record.count_lift_pow2
    );
}

/// What `arjun_input` promises is a POSITION in the chain — the formula after
/// the simplify chain and before Arjun — so it is checked against the run that
/// stops exactly there rather than against a hand-written formula.
#[test]
fn the_retained_arjun_input_is_the_formula_the_stage_before_it_produced() {
    let retaining = RunConfig {
        retain_arjun_input: true,
        ..counting()
    };
    let bundle = bundle_of(FREE_VAR_AND_DEFINITIONS, &retaining);

    let simplify_only = RunConfig {
        stages: PreprocessStages {
            simplify: true,
            arjun: false,
        },
        ..counting()
    };
    let stops_before_arjun = bundle_of(FREE_VAR_AND_DEFINITIONS, &simplify_only);

    assert_eq!(
        bundle.arjun_input.as_ref(),
        Some(&stops_before_arjun.reduced),
        "the formula Arjun was handed is what the chain had produced when it \
         reached that stage",
    );
}

/// It is a second whole formula held in memory, so a caller that never asked
/// does not pay for it — and a chain with no Arjun stage has nothing to hand it
/// even when asked.
#[test]
fn the_arjun_input_is_kept_only_when_it_was_asked_for() {
    assert_eq!(
        bundle_of(FREE_VAR_AND_DEFINITIONS, &counting()).arjun_input,
        None,
        "the default run must not carry a copy nobody wants",
    );

    let compile_retaining = RunConfig {
        mode: Some(Mode::Compile),
        retain_arjun_input: true,
        ..RunConfig::default()
    };
    assert_eq!(
        bundle_of(FREE_VAR_AND_DEFINITIONS, &compile_retaining).arjun_input,
        None,
        "the compile chain runs no Arjun, so there is no such formula to retain",
    );
}

/// A caller that hands the bundle to two consumers clones it, and both copies
/// must lift a count the same way — the clone is of the whole statement, not of
/// the formula with a fresh record beside it.
#[test]
fn a_cloned_bundle_lifts_a_count_the_same_way() {
    let bundle = bundle_of(FREE_VAR_AND_DEFINITIONS, &counting());
    let clone = bundle.clone();

    assert_eq!(clone.reduced, bundle.reduced);
    assert_eq!(clone.stages, bundle.stages);
    assert_eq!(clone.count_lift, bundle.count_lift);
    assert_eq!(
        serde_json::to_string(&clone.record).expect("a record must serialize"),
        serde_json::to_string(&bundle.record).expect("a record must serialize"),
        "the two halves that must travel together stay together across a clone",
    );
}

/// The maps are SIGNED, and a variable that survives as the negation of an
/// original is the only thing that makes the sign load-bearing. A map that
/// named variables instead of literals would pass every count-lift assertion in
/// this suite and silently invert a lifted assignment, so this pins that a real
/// chain produces a negative entry.
#[test]
fn an_anti_equivalent_variable_is_named_through_a_negated_literal() {
    let config = RunConfig {
        mode: Some(Mode::Compile),
        ..RunConfig::default()
    };
    let bundle = bundle_of(ANTI_EQUIVALENCE, &config);
    let map = bundle
        .record
        .original_to_reduced_dimacs
        .as_ref()
        .expect("the compile mode's map is total");

    assert!(
        map.iter()
            .any(|t| matches!(t, OriginalTarget::Literal(l) if l < 0)),
        "one of the anti-equivalent pair survives and the other is that \
         survivor's NEGATION, which only a signed entry can say: {:?}",
        map.iter().collect::<Vec<_>>(),
    );
}
