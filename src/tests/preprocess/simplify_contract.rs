//! The stage list each preprocessing contract permits, pinned.
//!
//! [`SimplifyPurpose`] is the only thing that turns a variable-eliminating stage
//! on, so these tests are the whole statement of "which stages may run under
//! which contract". What they cannot state — because it is not expressible — is
//! the negative: [`StageSet`]'s fields are `pub(crate)` with no constructor
//! beyond [`SimplifyPurpose::stages`] and [`StageSet::none`], so no caller,
//! inside the crate or out, can assemble a stage list a contract did not hand
//! it. The behavioural consequences are pinned by the bundle oracle suite
//! (`crate::tests::bundle`), whose `property_compile_reconstructs_the_function`
//! enumerates every assignment of a random instance and checks the
//! function-preserving run reconstructs it exactly.
//!
//! A stage's admission under the function-preserving contract is decided by one
//! question: can the record name what the stage removed? The equivalence
//! reduction can — a dropped partner is one signed literal of its surviving
//! representative — so it runs; gate detection and DVE cannot, so they do not.

use crate::cnf::Clause;
use crate::cnf::CnfFormula;
use crate::cnf::VarId;
use crate::config::SimplifyPolicy;
use crate::preprocess::simplify::*;
use crate::tests::common::{clause, lit};

/// The function-preserving contract runs the equivalence reduction and nothing
/// else that eliminates a variable — `original_to_reduced_dimacs` can name a
/// dropped equivalence partner, but not a gate- or DVE-eliminated variable.
#[test]
fn function_contract_permits_only_the_equivalence_reduction() {
    let stages = SimplifyPurpose::Function.stages();
    assert!(!stages.gates, "gate detection feeds DVE; it is count-only");
    assert!(stages.dve.is_none(), "DVE removes a definable variable");
    assert!(
        stages.reduce_equivalences,
        "a dropped partner is recoverable from its representative"
    );
}

/// Both count-preserving contracts run all three: a count pays for the removed
/// variables once through the lift, so there is nothing to reconstruct. Under a
/// count the two extra stages are the whole difference from the contract above.
#[test]
fn count_contracts_permit_every_stage() {
    for purpose in [SimplifyPurpose::Count, SimplifyPurpose::WeightedCount] {
        let stages = purpose.stages();
        assert!(stages.gates, "{purpose:?}");
        assert!(stages.dve.is_some(), "{purpose:?}");
        assert!(stages.reduce_equivalences, "{purpose:?}");
    }
}

/// Weighted counting differs from plain counting in what the CALLER freezes out
/// of DVE, never in the stage list itself — one chain, one set of stages.
#[test]
fn weighting_does_not_change_the_stage_list() {
    assert_eq!(
        SimplifyPurpose::Count.stages(),
        SimplifyPurpose::WeightedCount.stages()
    );
}

/// `keep_all_vars` is a caller-side veto: it resolves every stage off whatever
/// the contract would have allowed, and can never turn one on.
#[test]
fn keep_all_vars_resolves_every_stage_off() {
    for purpose in [
        SimplifyPurpose::Count,
        SimplifyPurpose::WeightedCount,
        SimplifyPurpose::Function,
    ] {
        let config = SimplifyConfig::for_purpose(purpose, /*keep_all_vars=*/ true);
        assert_eq!(config.stages, StageSet::none(), "{purpose:?}");
    }
}

/// Without the veto, the configuration carries the contract's own list verbatim
/// — `for_purpose` adds no stage arithmetic of its own.
#[test]
fn for_purpose_carries_the_contract_list_verbatim() {
    for purpose in [
        SimplifyPurpose::Count,
        SimplifyPurpose::WeightedCount,
        SimplifyPurpose::Function,
    ] {
        let config = SimplifyConfig::for_purpose(purpose, /*keep_all_vars=*/ false);
        assert_eq!(config.stages, purpose.stages(), "{purpose:?}");
    }
}

/// One equivalence class, `a ≡ b`, with `c` and `d` in a ternary clause that
/// implies nothing binary: the only reduction opportunity in the formula.
fn one_equivalence() -> CnfFormula {
    CnfFormula {
        num_vars: 4,
        clauses: vec![
            Clause::new(vec![lit(0, true), lit(1, false)]),
            Clause::new(vec![lit(0, false), lit(1, true)]),
            Clause::new(vec![lit(0, true), lit(2, true), lit(3, true)]),
            Clause::new(vec![lit(0, false), lit(2, false), lit(3, false)]),
        ],
    }
}

fn simplify_for_function(f: &CnfFormula) -> SimplifiedFormula {
    let policy = SimplifyPolicy::default();
    let config = SimplifyConfig {
        prefix: SimplifyPrefix::Backbone {
            budget_ms: policy
                .backbone_budget_ms
                .expect("the default probes backbones"),
            equivalence_budget_ms: policy.equivalence_budget_ms,
        },
        ..SimplifyConfig::for_purpose(SimplifyPurpose::Function, /*keep_all_vars=*/ false)
    };
    simplify(f, &config)
}

#[test]
fn eq_iter_prefix_runs_equivalence_reduction_and_the_count_tail() {
    let config = SimplifyConfig {
        prefix: SimplifyPrefix::EqIter,
        ..SimplifyConfig::for_purpose(SimplifyPurpose::Count, /*keep_all_vars=*/ false)
    };
    let simplified = simplify(&one_equivalence(), &config);

    assert!(config.stages.gates, "gate detection must remain enabled");
    assert!(config.stages.dve.is_some(), "DVE must remain enabled");
    assert!(
        simplified.equiv_reduced.is_some(),
        "ordinary equivalence iteration must still feed equivalence reduction",
    );
    assert!(
        simplified.telemetry.dve_ms.is_some(),
        "the shared count tail must attempt DVE after eq-iter",
    );
    assert_eq!(
        simplified.telemetry.backbone_ms, None,
        "eq-iter must not fabricate backbone telemetry",
    );
}

/// The function-preserving contract must survive the full `simplify` call, not
/// just the stage list: the equivalence's partner really is dropped, and DVE
/// really does not run.
#[test]
fn function_contract_drops_an_equivalence_partner() {
    let f = one_equivalence();
    let simplified = simplify_for_function(&f);
    assert!(
        simplified.equiv_reduced.is_some(),
        "the equivalence reduction did not run under the function-preserving contract"
    );
    assert!(
        simplified.dve_reduced.is_none(),
        "DVE ran under the function-preserving contract"
    );
    assert_eq!(
        simplified.reduced_formula().num_vars,
        f.num_vars - 1,
        "the partner of the one equivalence class was not dropped"
    );
}

/// Every original variable has a fate, including the one preprocessing dropped:
/// the partner reports the SAME reduced variable as the representative it folded
/// onto, which is what makes the drop recoverable.
#[test]
fn every_original_has_a_fate_including_the_dropped_partner() {
    let f = one_equivalence();
    let simplified = simplify_for_function(&f);
    let fates = simplified.original_fates();
    assert_eq!(fates.len(), f.num_vars as usize);
    assert_eq!(
        fates[0], fates[1],
        "`a` and `b` are equivalent, so both must name the same reduced literal"
    );
    for (original, fate) in fates.iter().enumerate() {
        let OriginalFate::Variable { index, .. } = *fate else {
            panic!("original variable {original} has no reduced counterpart: {fate:?}");
        };
        assert!(index < simplified.reduced_formula().num_vars as usize);
    }
}

/// The two directions are one map read both ways: following a reduced variable
/// back to the original it stands for and then reading that original's fate has
/// to land on the reduced variable it started from. A composition assembled
/// separately per direction is exactly what would drift here.
#[test]
fn original_fates_is_the_total_inverse_of_reduced_var_to_original() {
    let f = one_equivalence();
    let simplified = simplify_for_function(&f);
    let fates = simplified.original_fates();
    let reduced_vars = simplified.reduced_formula().num_vars as usize;

    for i in 0..reduced_vars {
        let original = simplified.reduced_var_to_original(i);
        let OriginalFate::Variable { index, .. } = fates[original] else {
            panic!(
                "reduced variable {i} stands for original {original}, whose fate names no variable: {:?}",
                fates[original]
            );
        };
        assert_eq!(
            index, i,
            "reduced variable {i} → original {original} → reduced variable {index}",
        );
    }

    let mut named: Vec<usize> = fates
        .iter()
        .filter_map(|fate| match fate {
            OriginalFate::Variable { index, .. } => Some(*index),
            _ => None,
        })
        .collect();
    named.sort_unstable();
    named.dedup();
    assert_eq!(
        named,
        (0..reduced_vars).collect::<Vec<_>>(),
        "every reduced variable must be named by some original's fate",
    );
}

/// Freezing is a property of the equivalence CLASS, not of the member named.
/// The representative is the only member left in the reduced formula, so a
/// caller freezing a partner would otherwise freeze nothing at all and the
/// variable it meant to protect would be eliminated anyway.
#[test]
fn a_frozen_original_variable_freezes_its_representative_and_every_partner_folded_onto_it() {
    let f = one_equivalence();
    let simplified = simplify_for_function(&f);
    let reduced_vars = simplified.reduced_formula().num_vars;
    let freeze = |original: u32| {
        let asked: rustc_hash::FxHashSet<VarId> = [VarId(original)].into_iter().collect();
        simplified.frozen_in_dve_space(&asked, reduced_vars)
    };

    // `a` and `b` are one class, so whichever of them the reduction kept, both
    // spellings of the request must protect it.
    let from_one_member = freeze(0);
    assert_eq!(
        from_one_member.len(),
        1,
        "one frozen original variable protects the one reduced variable it folded onto",
    );
    assert_eq!(
        from_one_member,
        freeze(1),
        "freezing either member of `a ≡ b` must freeze the same reduced variable",
    );

    let frozen = *from_one_member.iter().next().expect("one frozen variable");
    let stands_for = simplified.pre_dve_var_to_original(frozen.idx());
    assert!(
        stands_for == 0 || stands_for == 1,
        "the frozen reduced variable must be the class's own, not another: {stands_for}",
    );
    assert!(
        freeze(2).is_disjoint(&from_one_member),
        "freezing a variable outside the class must not protect the class's",
    );
}

/// An explicitly disabled prefix makes `simplify` the identity: it produces no
/// layer at all, so the best formula is the one it was handed. A missing
/// backbone budget is deliberately not used as the disable sentinel.
#[test]
fn preprocess_none_is_identity() {
    let formula = CnfFormula {
        num_vars: 3,
        clauses: vec![clause(&[(0, true), (1, false)]), clause(&[(2, true)])],
    };
    let config = SimplifyConfig {
        prefix: SimplifyPrefix::Disabled,
        ..SimplifyConfig::for_purpose(SimplifyPurpose::Function, /*keep_all_vars=*/ true)
    };
    let simplified = simplify(&formula, &config);

    assert_eq!(config.prefix, SimplifyPrefix::Disabled);
    assert_eq!(simplified.reduced_formula().clauses.len(), 2);
    assert_eq!(simplified.reduced_formula().num_vars, 3);
    assert!(simplified.preprocessed.is_none());
    assert!(simplified.stripped.is_none());
    assert!(simplified.equiv_reduced.is_none());
    assert!(simplified.dve_reduced.is_none());
}
