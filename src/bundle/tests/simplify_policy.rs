//! The public simplify policy threaded into the one internal simplify path.

use super::super::plumbing::preprocess_config;
use crate::cnf::{Original, Weights};
use crate::config::{DvePolicy, RunConfig, SimplifyPolicy};
use crate::preprocess::simplify::SimplifyPurpose;

fn no_weights() -> Weights<Original> {
    Weights::empty()
}

#[test]
fn custom_policy_reaches_the_count_simplify_config() {
    let config = RunConfig {
        simplify: SimplifyPolicy {
            backbone_budget_ms: Some(17),
            equivalence_budget_ms: None,
            detect_gates: false,
            dve: Some(DvePolicy {
                rounds: 4,
                budget_ms: 29,
            }),
        },
        ..RunConfig::default()
    };

    let internal = preprocess_config(&config, SimplifyPurpose::Count, &no_weights());
    assert_eq!(internal.backbone_budget_ms, Some(17));
    assert_eq!(internal.equiv_budget_ms, None);
    assert!(!internal.stages.gates);
    assert_eq!(
        internal.stages.dve.map(|dve| (dve.rounds, dve.budget_ms)),
        Some((4, 29)),
    );
}

#[test]
fn function_contract_caps_count_only_stages() {
    let config = RunConfig {
        simplify: SimplifyPolicy {
            detect_gates: true,
            dve: Some(DvePolicy {
                rounds: 4,
                budget_ms: 29,
            }),
            ..SimplifyPolicy::default()
        },
        ..RunConfig::default()
    };

    let internal = preprocess_config(&config, SimplifyPurpose::Function, &no_weights());
    assert!(!internal.stages.gates, "gate detection is count-only");
    assert!(internal.stages.dve.is_none(), "DVE is count-only");
}
