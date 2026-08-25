//! Filling a configuration from the environment, over the per-variable readers
//! this module keeps to itself.

use crate::config::RunConfig;

/// The environment-filled config differs from [`Default`] only in the knobs
/// that have a variable, and agrees with the pure parser on whatever the shell
/// actually has set — so the reader and the accepted spellings cannot drift.
/// Reads the environment, never writes it.
#[test]
fn from_env_defaults_is_the_default_plus_the_variables() {
    let raw = std::env::var("VITRI_ARJUN_SBVA").ok();
    let expected = crate::preprocess::arjun::arjun_sbva_policy(raw.as_deref());
    let raw_budget = std::env::var("VITRI_BUDGET_MS").ok();
    match (RunConfig::from_env_defaults(), expected) {
        (Ok(c), Ok(want)) => {
            assert_eq!(c.arjun.sbva, want);
            assert_eq!(
                c.budget_ms,
                crate::config::budget_hint_ms(raw_budget.as_deref())
            );
            let d = RunConfig::default();
            assert_eq!(c.vtree_spec, d.vtree_spec);
            assert_eq!(c.candidates, d.candidates);
            assert_eq!(c.components, d.components);
            assert_eq!(c.mode, d.mode);
        }
        (Err(_), Err(_)) => {}
        (got, want) => panic!("reader and parser disagree: {got:?} vs {want:?}"),
    }
}

/// `VITRI_BUDGET_MS` is a DEFAULT for [`RunConfig::budget_ms`], and the one knob
/// that reads a value it cannot use as unset rather than refusing the run.
#[test]
fn the_budget_hint_takes_a_u64_and_reads_anything_else_as_unset() {
    use crate::config::budget_hint_ms;
    assert_eq!(budget_hint_ms(Some("120000")), Some(120_000));
    assert_eq!(budget_hint_ms(Some("3600000")), Some(3_600_000));
    assert_eq!(budget_hint_ms(Some("0")), Some(0));
    assert_eq!(budget_hint_ms(None), None);
    for unusable in ["not-a-number", "", " ", "12x", "-1", "1.5"] {
        assert_eq!(budget_hint_ms(Some(unusable)), None, "{unusable}");
    }
}
