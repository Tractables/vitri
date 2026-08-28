//! Reusing the one owned count-preserving simplify checkpoint.

use super::*;
use crate::bundle::stage::arjun_stage;
use crate::cnf::ShowSet;
use crate::config::PreprocessStages;
use crate::preprocess::VarMap;
use crate::tests::common::make_formula;

fn candidate(
    formula: CnfFormula,
    multiplier_exp: u32,
    map: VarMap<Reduced, Reduced>,
) -> ArjunResult {
    let independent_support = ShowSet::from_zero_based(0..formula.num_vars);
    ArjunResult {
        formula,
        multiplier_exp,
        backbone: Vec::new(),
        equiv: Vec::new(),
        learnt_clauses: Vec::new(),
        independent_support,
        input_to_reduced_lit: map,
    }
}

fn finish_with(
    stage1: &CountStage1,
    config: &RunConfig,
    candidate: ArjunResult,
    expected_input: *const CnfFormula,
) -> PreprocessBundle {
    finish_count_preserving_attempt_using(stage1, config, |formula, _weights, report, telemetry| {
        assert_eq!(
            formula as *const CnfFormula, expected_input,
            "each attempt must borrow the same stage-1 formula",
        );
        let result = arjun_stage(
            formula,
            config,
            report,
            telemetry,
            |_budget, _no_sbva| Ok(Some(candidate)),
            |_candidate| None,
        )?;
        Ok(result.map_or(CountArjun::Skipped, CountArjun::Plain))
    })
    .expect("the synthetic finish must succeed")
}

#[test]
fn one_stage1_checkpoint_supports_two_independent_truthful_arjun_finishes() {
    // Variable 3 is free. Simplify is disabled so the first synthetic Arjun
    // outcome may keep it, while the second may remove it and earn exactly ×2.
    let raw = make_formula(3, vec![vec![1, 2]]);
    let config = RunConfig {
        mode: Some(Mode::Mc),
        stages: PreprocessStages {
            simplify: false,
            arjun: true,
        },
        ..RunConfig::default()
    };
    let stage1 = count_stage1(&raw, &CnfMeta::default(), &config, Mode::Mc);
    let stage1_formula = stage1.simplified.reduced_formula() as *const CnfFormula;
    let stage1_stages = stage1.stages.clone();
    let stage1_telemetry = stage1.telemetry;

    let kept = finish_with(
        &stage1,
        &config,
        candidate(raw.clone(), 0, VarMap::identity(3)),
        stage1_formula,
    );
    let reduced = finish_with(
        &stage1,
        &config,
        candidate(
            make_formula(2, vec![vec![1, 2]]),
            1,
            VarMap::from_entries(vec![Some(1), Some(2), None]),
        ),
        stage1_formula,
    );

    assert_eq!(stage1.simplified.reduced_formula(), &raw);
    assert_eq!(stage1.stages, stage1_stages);
    assert_eq!(stage1.telemetry, stage1_telemetry);
    assert_eq!(
        stage1.stages.simplify,
        Some(StageOutcome::Skipped(SkipReason::NotRequested)),
    );
    assert_eq!(stage1.stages.arjun, None);

    assert_eq!(kept.reduced, raw);
    assert_eq!(kept.count_lift, CountLift::default());
    assert_eq!(kept.record.count_lift_pow2, 0);
    assert_eq!(kept.record.reduced_to_original_dimacs, VarMap::identity(3),);
    assert_eq!(kept.stages.simplify, stage1.stages.simplify);
    assert_eq!(kept.stages.arjun, Some(StageOutcome::Ran));
    assert_eq!(kept.telemetry.simplify_ms, None);

    assert_eq!(reduced.reduced, make_formula(2, vec![vec![1, 2]]));
    assert_eq!(
        reduced.count_lift,
        CountLift {
            simplify_pow2: 0,
            arjun_pow2: 1,
        },
    );
    assert_eq!(reduced.record.count_lift_pow2, 1);
    assert_eq!(
        reduced.record.reduced_to_original_dimacs,
        VarMap::from_entries(vec![Some(1), Some(2)]),
    );
    assert_eq!(reduced.stages.simplify, stage1.stages.simplify);
    assert_eq!(reduced.stages.arjun, Some(StageOutcome::Ran));
    assert_eq!(reduced.telemetry.simplify_ms, None);
}
