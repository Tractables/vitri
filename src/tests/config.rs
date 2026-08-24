use crate::cnf::Mode;
use crate::config::*;
use crate::error::VitriError;
use crate::preprocess::ArjunSbva;
use crate::spec::DEFAULT_VTREE_SPEC;
use std::time::Duration;
use std::time::Instant;

/// Every mode, so a table below covers the whole partition rather than the
/// cases someone remembered.
const EVERY_MODE: [Mode; 5] = [Mode::Mc, Mode::Wmc, Mode::Pmc, Mode::Pwmc, Mode::Compile];

#[test]
fn default_is_the_production_configuration() {
    let c = RunConfig::default();
    assert_eq!(c.vtree_spec, DEFAULT_VTREE_SPEC);
    assert_eq!(c.budget_ms, None);
    assert!(c.stages.simplify && c.stages.arjun);
    assert_eq!(c.components, ComponentPolicy::Split);
    assert_eq!(c.candidates, 1, "the default keeps only the winner");
    assert!(c.validate().is_ok());
}

/// The retained-candidate count is bounded, and the bound is an ERROR naming the
/// ceiling — not a silent clamp — because it is a peak-memory decision.
#[test]
fn the_candidate_count_is_bounded_and_says_so() {
    let over = RunConfig {
        candidates: crate::candidates::MAX_CANDIDATES + 1,
        ..Default::default()
    };
    let e = over.validate().unwrap_err().to_string();
    assert!(
        e.contains(&crate::candidates::MAX_CANDIDATES.to_string()),
        "the error must name the ceiling, got: {e}"
    );
    assert!(
        RunConfig {
            candidates: 0,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
}

/// Asking a single-vtree spec for a candidate set is inert, so it fails fast naming
/// both the field and the spec rather than returning one candidate and
/// letting the caller assume the rest were pruned on merit.
#[test]
fn a_candidate_set_on_a_single_vtree_spec_is_rejected() {
    let c = RunConfig {
        candidates: 3,
        vtree_spec: "minfill-primal".to_string(),
        ..Default::default()
    };
    let e = c.validate().unwrap_err().to_string();
    assert!(
        e.contains("minfill-primal") && e.contains("candidates"),
        "got: {e}"
    );
    let ok = RunConfig {
        candidates: 3,
        vtree_spec: DEFAULT_VTREE_SPEC.to_string(),
        ..Default::default()
    };
    assert!(
        ok.validate().is_ok(),
        "{DEFAULT_VTREE_SPEC} must accept a candidate set"
    );
}

#[test]
fn explicit_deadline_wins_over_budget_as_the_cutoff() {
    let now = Instant::now();
    let d = now + Duration::from_secs(5);
    let c = RunConfig {
        budget_ms: Some(60_000),
        deadline: Some(d),
        ..Default::default()
    };
    assert_eq!(c.resolved_deadline(now), Some(d));
    // ...but the budget still supplies the SCALE.
    assert_eq!(c.effective_budget_ms(now), Some(60_000));
}

#[test]
fn budget_alone_yields_a_deadline_and_a_scale() {
    let now = Instant::now();
    let c = RunConfig {
        budget_ms: Some(1_000),
        ..Default::default()
    };
    assert_eq!(
        c.resolved_deadline(now),
        Some(now + Duration::from_millis(1_000))
    );
    assert_eq!(c.effective_budget_ms(now), Some(1_000));
}

#[test]
fn deadline_alone_still_supplies_a_scale() {
    // The footgun this guards: a deadline-only caller must not get the
    // *unbounded* sub-budget defaults while its cutoff truncates them.
    let now = Instant::now();
    let c = RunConfig {
        deadline: Some(now + Duration::from_millis(30_000)),
        ..Default::default()
    };
    assert_eq!(c.effective_budget_ms(now), Some(30_000));
}

#[test]
fn unbounded_by_default() {
    let now = Instant::now();
    let c = RunConfig::default();
    assert_eq!(c.resolved_deadline(now), None);
    assert_eq!(c.effective_budget_ms(now), None);
}

#[test]
fn bounded_variable_addition_is_on_by_default() {
    assert_eq!(RunConfig::default().arjun_sbva, ArjunSbva::On);
}

/// A stage switched off under a mode whose preprocessing has no such stage is
/// refused by the validator itself, so a library caller is told before any
/// budget is spent — the same answer, from the same owner, that the command
/// line gives.
#[test]
fn a_stage_the_mode_does_not_have_is_refused_by_validate() {
    use crate::cnf::Mode;
    let inert = RunConfig {
        mode: Some(Mode::Pmc),
        stages: PreprocessStages {
            simplify: false,
            ..PreprocessStages::default()
        },
        ..RunConfig::default()
    };
    let err = inert
        .validate()
        .expect_err("the projected chain has no simplify stage to skip");
    let msg = err.to_string();
    assert!(
        msg.contains("--no-simplify") && msg.contains(Mode::Pmc.token()),
        "the error must name the stage and the mode, got: {msg}",
    );

    // The same stage under a mode that HAS it is an ordinary request.
    let live = RunConfig {
        mode: Some(Mode::Mc),
        ..inert
    };
    live.validate().expect("mc's chain has a simplify stage");
}

/// A run is several phases, and they share one budget only if the instant it
/// ends at is decided ONCE. Anchoring fixes it, and anchoring again — which is
/// what a phase reaching for the budget later would do — cannot move it.
#[test]
fn anchoring_freezes_one_deadline_that_both_halves_of_a_run_share() {
    let now = Instant::now();
    let c = RunConfig {
        budget_ms: Some(60_000),
        ..RunConfig::default()
    };
    let anchored = c.anchored(now);
    assert_eq!(
        anchored.deadline,
        Some(now + Duration::from_millis(60_000)),
        "the cutoff is the budget counted from the one clock reading",
    );
    assert_eq!(
        anchored.budget_ms,
        Some(60_000),
        "the scale sub-budgets derive from travels with it",
    );

    let half_way = now + Duration::from_millis(30_000);
    assert_eq!(
        anchored.anchored(half_way).deadline,
        anchored.deadline,
        "a second phase gets what is left of the original, not a fresh copy",
    );
}

/// Each mode's chain answers for which stage toggles it reads, and the answer
/// is the whole two-field struct — a `false` names a stage that mode's
/// preprocessing does not have at all.
#[test]
fn each_mode_reads_exactly_the_stage_toggles_its_chain_has() {
    for (mode, simplify, arjun) in [
        (Mode::Mc, true, true),
        (Mode::Wmc, true, true),
        (Mode::Pmc, false, true),
        (Mode::Pwmc, false, true),
        (Mode::Compile, true, false),
    ] {
        assert_eq!(
            PreprocessStages::read_under(mode),
            PreprocessStages { simplify, arjun },
            "mode {}",
            mode.token(),
        );
    }
}

/// The five modes partition across three chains, and that partition is what
/// decides both which route an instance takes and which refusal message it can
/// be given.
#[test]
fn every_mode_routes_to_the_chain_that_answers_for_it() {
    for (mode, chain) in [
        (Mode::Mc, Chain::Count),
        (Mode::Wmc, Chain::Count),
        (Mode::Pmc, Chain::Projection),
        (Mode::Pwmc, Chain::Projection),
        (Mode::Compile, Chain::Compile),
    ] {
        assert_eq!(Chain::for_mode(mode), chain, "mode {}", mode.token());
    }
    // Three chains, not five: a mode is routed, not given one of its own.
    let mut chains: Vec<Chain> = EVERY_MODE.iter().copied().map(Chain::for_mode).collect();
    chains.dedup();
    assert_eq!(chains.len(), 3);
}

/// The whole mode × stage-flag matrix: turning off a stage the mode's chain
/// does not have is refused, naming both the flag and the mode, and turning off
/// one it does have is an ordinary request.
#[test]
fn an_inert_stage_flag_is_refused_for_every_mode_that_lacks_that_stage() {
    for mode in EVERY_MODE {
        let read = PreprocessStages::read_under(mode);
        for (flag, stages, mode_reads_it) in [
            (
                "--no-simplify",
                PreprocessStages {
                    simplify: false,
                    arjun: true,
                },
                read.simplify,
            ),
            (
                "--no-arjun",
                PreprocessStages {
                    simplify: true,
                    arjun: false,
                },
                read.arjun,
            ),
        ] {
            let c = RunConfig {
                mode: Some(mode),
                stages,
                ..RunConfig::default()
            };
            let outcome = c.refuse_inert(mode);
            if mode_reads_it {
                outcome
                    .unwrap_or_else(|e| panic!("{flag} is live under mode {}: {e}", mode.token()));
                continue;
            }
            let msg = outcome
                .expect_err(&format!(
                    "{flag} does nothing under mode {}, so it must be refused",
                    mode.token(),
                ))
                .to_string();
            assert!(
                msg.contains(flag) && msg.contains(mode.token()),
                "the refusal must name the flag and the mode, got: {msg}",
            );
        }
    }
}

/// The learnt clauses come from one stage of one chain, so asking for them
/// under any other mode is refused — and the message names the mode that can,
/// which is the only actionable thing to say.
#[test]
fn a_learnt_clause_export_under_a_mode_that_cannot_harvest_names_the_mode_that_can() {
    for mode in EVERY_MODE {
        let c = RunConfig {
            mode: Some(mode),
            export_learned_clauses: true,
            ..RunConfig::default()
        };
        if mode == Mode::Mc {
            c.refuse_inert(mode)
                .expect("the count-preserving chain is the one that harvests");
            continue;
        }
        let msg = c
            .refuse_inert(mode)
            .expect_err(&format!("mode {} harvests nothing", mode.token()))
            .to_string();
        assert!(
            msg.contains("VITRI_ARJUN_EXPORT_LEARNED_CLAUSES") && msg.contains(Mode::Mc.token()),
            "the refusal must name the request and the mode that answers it, got: {msg}",
        );
    }
}

/// The second half of the same rule: the right mode with the harvesting stage
/// switched off has no source either, and says which stage that is.
#[test]
fn a_learnt_clause_export_with_the_reducing_stage_off_is_refused_too() {
    let c = RunConfig {
        mode: Some(Mode::Mc),
        export_learned_clauses: true,
        stages: PreprocessStages {
            simplify: true,
            arjun: false,
        },
        ..RunConfig::default()
    };
    let msg = c
        .refuse_inert(Mode::Mc)
        .expect_err("no stage is left to derive the clauses")
        .to_string();
    assert!(
        msg.contains("VITRI_ARJUN_EXPORT_LEARNED_CLAUSES") && msg.contains("--no-arjun"),
        "the refusal must name the request and the stage it needs, got: {msg}",
    );
}

/// An out-of-range parameter is a mistake in the REQUEST, so the validator reports it
/// as a spec error — before any budget is spent preprocessing the formula, and
/// as the variant whose fix is to change the spec string rather than some other
/// field.
#[test]
fn validate_refuses_a_spec_token_the_family_cannot_honor() {
    let c = RunConfig {
        vtree_spec: "force:dim=9".to_string(),
        ..RunConfig::default()
    };
    let err = c
        .validate()
        .expect_err("an axis value outside the grammar must not be dropped");
    assert!(
        matches!(err, VitriError::Spec { .. }),
        "the spec string is what needs fixing, got: {err:?}",
    );
    assert!(
        err.to_string().contains("9"),
        "the offending token must appear, got: {err}",
    );
}

/// The `--components` vocabulary is one table read three ways, so a shell over
/// this crate can offer what it will accept instead of keeping a copy.
#[test]
fn every_component_policy_token_parses_back_to_the_policy_that_wrote_it() {
    let offered: Vec<&str> = ComponentPolicy::names().collect();
    assert_eq!(offered, vec!["split", "whole"]);
    for token in &offered {
        let policy = ComponentPolicy::parse(token)
            .unwrap_or_else(|| panic!("{token} is offered, so it must parse"));
        assert_eq!(policy.token(), *token);
    }
    assert_eq!(
        ComponentPolicy::parse("Split"),
        None,
        "a token is the exact spelling the flag takes",
    );
    assert!(ComponentPolicy::Whole.is_whole());
    assert!(!ComponentPolicy::Split.is_whole());
}
