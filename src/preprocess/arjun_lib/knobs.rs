//! The `VITRI_*` knobs the Arjun binding reads, and the constants the oracle
//! gate is sized against. Pure of the shim and of the stage driver, so a form
//! or a bound can be checked without a reduction.

use crate::error::VitriError;
use crate::preprocess::arjun::ArjunEffort;

/// Reference budget (ms) the full oracle (`oracle_mult = 1.0`) can burn in its
/// pathological worst case, before any scaling. Anchored to an observed ~30s
/// uninterruptible overrun on a small post-stage-1 formula. Arjun's oracle mems
/// budget scales linearly with `oracle_mult` (CryptoMiniSat `oracle_use.cpp`:
/// every pass budget is `const × oracle_mult`), so worst-case oracle budget ≈
/// `oracle_mult × ORACLE_FULL_WORSTCASE_MS`.
///
/// A conservative runaway-guard reference, not a tuned average: at
/// `remaining ≥ 30s` the oracle runs uncapped (`oracle_mult = 1.0`); scaling
/// only engages below that, to stop a pathological 30s-against-a-10s-budget
/// blow-up. A 2–3× worst-case overrun is acceptable (mems→budget is
/// instance-variable); a 30s×3 one is not.
pub(super) const ORACLE_FULL_WORSTCASE_MS: u128 = 30_000;

/// Floor for the scaled oracle effort. Purely a utility floor — soundness holds
/// at any value (smaller ⇒ fewer proven removals ⇒ larger-but-exact), so this
/// only stops the oracle being throttled to do essentially nothing. The 6000 ms
/// oracle pre-start gate means `remaining ≥ 6000` whenever this is evaluated,
/// so the floor is defensive (raw ≥ 0.2 there).
pub(super) const ORACLE_MULT_MIN: f64 = 0.05;

/// Size the heavy stage's `oracle_mult` from the budget still remaining
/// when the oracle is about to start. The oracle's worst-case budget scales
/// linearly with `oracle_mult`, so choosing `remaining / ORACLE_FULL_WORSTCASE_MS`
/// keeps that worst case near the remaining budget. Clamped to
/// `[ORACLE_MULT_MIN, 1.0]`: `remaining ≥ ORACLE_FULL_WORSTCASE_MS` ⇒ `1.0`
/// (uncapped, matches today exactly); less ⇒ proportionally smaller, floored.
/// Pure (no I/O, no env) so it is unit-testable in isolation.
pub(super) fn oracle_mult_for_budget(remaining_ms: u128) -> f64 {
    let raw = remaining_ms as f64 / ORACLE_FULL_WORSTCASE_MS as f64;
    raw.clamp(ORACLE_MULT_MIN, 1.0)
}

/// The default both projected pre-passes take for
/// [`OracleCaps::projected`](crate::preprocess::arjun::OracleCaps::projected) and its
/// weighted twin.
///
/// Both are single-lane and keep their checkpoint regardless of overrun, so on
/// a large formula an oracle overrun consumes the whole budget while the cheap
/// BVE/SBVA/autarky pipeline reaches the same reduction in a fraction of the
/// time. The cap skips the oracle on the class that overruns while keeping it
/// for small formulas, where it is cheap and cannot overrun.
pub(in crate::preprocess) const PROJECTED_ORACLE_MAX_VARS_DEFAULT: u32 = 100_000;

/// What both `VITRI_*_ORACLE_MAX_VARS` knobs accept, in the words of whoever
/// sets one. Stated once, so the two knobs cannot come to describe themselves
/// differently.
pub(super) const ORACLE_MAX_VARS_FORM: &str =
    "a variable count, above which the reduce skips Arjun's oracle";

/// Read one projected pre-pass's oracle cap from its variable. The two differ
/// only in the name, so the default they fall back to and the form they accept
/// are settled here rather than at each call.
///
/// # Errors
///
/// [`VitriError::Env`] naming the variable.
pub(in crate::preprocess) fn projected_oracle_max_vars(
    var: &'static str,
) -> Result<u32, VitriError> {
    crate::env::parse(var, PROJECTED_ORACLE_MAX_VARS_DEFAULT, ORACLE_MAX_VARS_FORM)
}

/// What `VITRI_ARJUN_EFFORT` accepts, quoted in both of its messages.
const ARJUN_EFFORT_FORMS: &str = "`full` (default) or `lite`";

/// Env-free parser for `VITRI_ARJUN_EFFORT` (kept pure so it is unit-testable
/// without touching the process environment). Absent ⇒ [`ArjunEffort::Full`]
/// (production default). Unknown value ⇒ `Err` naming the var and valid values.
pub(super) fn parse_arjun_effort(val: Option<&str>) -> Result<ArjunEffort, VitriError> {
    crate::env::from_forms(
        "VITRI_ARJUN_EFFORT",
        val,
        ArjunEffort::Full,
        &[("full", ArjunEffort::Full), ("lite", ArjunEffort::Lite)],
        ARJUN_EFFORT_FORMS,
    )
}

/// Reads `VITRI_ARJUN_EFFORT` into an [`ArjunEffort`]; absent ⇒ `Full`. A bad
/// value is a hard, fail-fast error naming the var and valid values.
///
/// # Errors
///
/// [`VitriError::Env`] naming `VITRI_ARJUN_EFFORT` and the valid values.
pub(crate) fn resolve_arjun_effort() -> Result<ArjunEffort, VitriError> {
    let raw = crate::env::env_raw("VITRI_ARJUN_EFFORT", ARJUN_EFFORT_FORMS)?;
    parse_arjun_effort(raw.as_deref())
}

/// `VITRI_ARJUN_EXPORT_LEARNED_CLAUSES=1` (default off): harvest the
/// redundant/learnt clauses Arjun's internal solver derived during simplify.
/// Read in this one place, by
/// [`RunConfig::from_env_defaults`](crate::config::RunConfig::from_env_defaults),
/// which is what carries it to the reduce's `export_learned_clauses` argument —
/// so a caller that builds its own config decides the harvest itself and the
/// reduction below has one switch rather than two.
///
/// # Errors
///
/// [`VitriError::Env`] when the variable is set to neither an on nor an off
/// spelling.
pub(crate) fn export_learned_clauses_enabled() -> Result<bool, VitriError> {
    crate::env::env_flag("VITRI_ARJUN_EXPORT_LEARNED_CLAUSES")
}
