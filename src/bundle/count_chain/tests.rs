//! Reusing the one owned count-preserving simplify checkpoint.

use super::*;
use crate::bundle::stage::arjun_stage;
use crate::cnf::{ShowSet, VarId};
use crate::config::PreprocessStages;
use crate::preprocess::VarMap;
use crate::tests::common::make_formula;

fn candidate(
    formula: CnfFormula,
    multiplier_exp: u32,
    map: VarMap<Reduced, Reduced>,
) -> ArjunResult {
    let independent_support = ShowSet::from_vars(VarId::all(formula.num_vars()));
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
            Mode::Mc,
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

    assert_finish_matches(&kept, &stage1, &raw, 0, VarMap::identity(3));
    assert_finish_matches(
        &reduced,
        &stage1,
        &make_formula(2, vec![vec![1, 2]]),
        1,
        VarMap::from_entries(vec![Some(1), Some(2)]),
    );
}

fn assert_finish_matches(
    bundle: &PreprocessBundle,
    stage1: &CountStage1,
    expected_reduced: &CnfFormula,
    expected_arjun_pow2: u32,
    expected_map: VarMap<Reduced, Original>,
) {
    assert_eq!(&bundle.reduced, expected_reduced);
    assert_eq!(
        bundle.count_lift,
        CountLift {
            simplify_pow2: 0,
            arjun_pow2: expected_arjun_pow2,
        },
    );
    assert_eq!(bundle.record.count_lift_pow2, expected_arjun_pow2);
    assert_eq!(bundle.record.reduced_to_original_dimacs, expected_map);
    assert_eq!(bundle.stages.simplify, stage1.stages.simplify);
    assert_eq!(bundle.stages.arjun, Some(StageOutcome::Ran));
    assert_eq!(bundle.telemetry.simplify_ms, None);
}
