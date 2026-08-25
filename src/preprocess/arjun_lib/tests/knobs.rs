use super::*;

/// The three-way split of a reduce's budget outcome. Boundaries are the whole
/// point — the class decides whether a sound checkpoint is handed back or
/// thrown away:
///   * at or before the deadline ⇒ IN-BUDGET (unchanged, always accepted);
///   * past it but within `DEADLINE_CUT_GRACE`, deadline ARMED ⇒ DEADLINE-CUT
///     (inclusive at exactly the grace);
///   * past the grace ⇒ OVERRUN;
///   * deadline NOT armed ⇒ no cut class exists at all (nothing stopped the
///     stage on time), so any past-deadline return is an OVERRUN.
#[test]
fn deadline_class_boundaries() {
    let started = Instant::now();
    let deadline = started + Duration::from_secs(10);

    assert_eq!(
        classify_budget(started, deadline, true),
        BudgetClass::InBudget
    );
    assert_eq!(
        classify_budget(deadline - Duration::from_millis(1), deadline, true),
        BudgetClass::InBudget
    );
    assert_eq!(
        classify_budget(deadline, deadline, true),
        BudgetClass::InBudget
    );
    // Armed-ness is irrelevant while inside the budget.
    assert_eq!(
        classify_budget(deadline, deadline, false),
        BudgetClass::InBudget
    );

    // 1, 100, 1_200, 2_100 ms: the measured 0.1–2.1 s band real deadline cuts
    // land in.
    for over in [1u64, 100, 1_200, 2_100] {
        assert_eq!(
            classify_budget(deadline + Duration::from_millis(over), deadline, true),
            BudgetClass::DeadlineCut,
            "{over}ms past the deadline should be a cut"
        );
    }
    assert_eq!(
        classify_budget(deadline + DEADLINE_CUT_GRACE, deadline, true),
        BudgetClass::DeadlineCut
    );

    assert_eq!(
        classify_budget(
            deadline + DEADLINE_CUT_GRACE + Duration::from_millis(1),
            deadline,
            true
        ),
        BudgetClass::Overrun
    );
    assert_eq!(
        classify_budget(deadline + Duration::from_secs(300), deadline, true),
        BudgetClass::Overrun
    );

    assert_eq!(
        classify_budget(deadline + Duration::from_millis(1), deadline, false),
        BudgetClass::Overrun
    );
    assert_eq!(
        classify_budget(deadline + Duration::from_millis(500), deadline, false),
        BudgetClass::Overrun
    );
}

/// The acceptance policy over those classes: a deadline cut is KEPT, an overrun
/// is DISCARDED unless the keep-overrun knob is on, and that knob is strictly
/// more permissive (it keeps everything).
#[test]
fn deadline_cut_acceptance_policy() {
    let started = Instant::now();
    let deadline = started + Duration::from_secs(10);
    let in_budget = deadline - Duration::from_millis(1);
    let cut = deadline + Duration::from_millis(1_200);
    let overrun = deadline + DEADLINE_CUT_GRACE + Duration::from_secs(5);
    let keep = |finished, keep_overrun| {
        keep_after_deadline("test", finished, started, deadline, true, keep_overrun)
    };

    assert!(keep(in_budget, false));
    assert!(keep(cut, false));
    assert!(!keep(overrun, false));

    assert!(keep(in_budget, true));
    assert!(keep(cut, true));
    assert!(keep(overrun, true));

    // An unarmed deadline has no cut class: the same instant is an overrun,
    // so it is discarded by default and kept only by keep-overrun.
    assert!(!keep_after_deadline(
        "test", cut, started, deadline, false, false
    ));
    assert!(keep_after_deadline(
        "test", cut, started, deadline, false, true
    ));
}

/// Every reduce path abandons its reduction through the one give-up emitter, so
/// the line's three shapes are pinned here rather than at the sites: no timing
/// at all (a read-back that produced nothing), an elapsed time (a stage that
/// ran), and an elapsed time against the budget it was measured on. The wording
/// is what a consumer greps its logs for, so it is the thing under test.
#[test]
fn giveup_line_shapes() {
    assert_eq!(
        giveup_line(
            "arjun-anytime",
            format_args!("the count multiplier is not text"),
            Spent::Unmeasured,
        ),
        "[arjun-anytime] give-up: the count multiplier is not text"
    );
    assert_eq!(
        giveup_line(
            "arjun-anytime",
            format_args!("multiplier not a power of two"),
            Spent::Elapsed(Duration::from_millis(1_240)),
        ),
        "[arjun-anytime] give-up: multiplier not a power of two after 1.2s"
    );
    assert_eq!(
        giveup_line(
            "arjun-anytime-wmc",
            format_args!("overrun-discard"),
            Spent::ElapsedOfBudget(Duration::from_millis(12_400), Duration::from_secs(10)),
        ),
        "[arjun-anytime-wmc] give-up: overrun-discard after 12.4s (budget 10.0s)"
    );
}

/// The knob readers in this module go through the crate's shared on/off
/// parser. What is pinned HERE is that the live readers are that parser
/// applied to their own variable, on the process's ACTUAL environment — so
/// they cannot drift into a private spelling, whether or not the variable is
/// exported.
#[test]
fn knob_readers_are_the_shared_flag_parser() {
    for (name, read) in [
        (
            "VITRI_ARJUN_KEEP_OVERRUN",
            keep_overrun_enabled as fn() -> Result<bool, VitriError>,
        ),
        (
            "VITRI_ARJUN_EXPORT_LEARNED_CLAUSES",
            export_learned_clauses_enabled as fn() -> Result<bool, VitriError>,
        ),
    ] {
        let raw = std::env::var(name).ok();
        assert_eq!(
            read(),
            crate::env::flag_value(name, raw.as_deref(), false),
            "{name}"
        );
    }
}

/// The shim's own reader takes the growth budget from `0` to `INT_MAX` and
/// returns failure outside that — which costs the whole heavy stage. So both
/// edges have to be refused HERE, where it is still a reportable error rather
/// than a stage that quietly did not run, and the accepted set has to be the
/// shim's exactly: one value too wide is a silent weaker run, one too narrow
/// refuses a budget the shim would have honoured.
#[test]
fn a_clause_growth_budget_outside_the_vendored_range_is_refused() {
    assert_eq!(bve_grow_value(None).expect("unset is not an error"), 0);
    assert_eq!(
        bve_grow_value(Some("0")).expect("the bottom of the range"),
        0
    );
    assert_eq!(bve_grow_value(Some("6")).expect("six clauses of growth"), 6);
    assert_eq!(
        bve_grow_value(Some("2147483647")).expect("the top of the range"),
        i32::MAX
    );
    for bad in ["-1", "-6", "2147483648", "1.5", "lots"] {
        let err = match bve_grow_value(Some(bad)) {
            Err(err) => err,
            Ok(taken) => panic!("{bad:?} must not be accepted, was read as {taken}"),
        };
        assert!(
            err.to_string().contains("VITRI_ARJUN_BVE_GROW"),
            "the error must name the variable, got: {err}"
        );
    }
}

/// A presence-only switch turns its pass off by being set at all, so a value
/// that reads as "off" would do the opposite of what it says and is refused
/// instead. What reads as "off" comes off the crate's one rule for writing a
/// value, so the case it is exported in does not decide whether the trap is
/// caught.
#[test]
fn a_presence_only_switch_refuses_an_off_looking_value() {
    for off in ["", "0", "off", "false", "OFF", " False "] {
        assert!(reads_as_off(off), "{off:?} reads as off");
    }
    for on in ["1", "yes", "please", "on"] {
        assert!(!reads_as_off(on), "{on:?} does not read as off");
    }
}

/// `oracle_mult_for_budget` is the pure cap-sizing function: remaining budget
/// (ms) -> `oracle_mult` in `[ORACLE_MULT_MIN, 1.0]`, scaling linearly and
/// clamping at both ends. Guards the runaway-bound contract:
///   * ample runway (>= worst-case ms) leaves the oracle UNCAPPED (1.0), so
///     the full-config path is byte-identical when there is time to spare;
///   * the live-path minimum (the 6000 ms oracle gate) yields a small-but-
///     nonzero scalar (~0.2), keeping the oracle's worst case near budget;
///   * it never returns 0 or > 1, and is monotonic in the remaining budget.
#[test]
fn oracle_mult_sizing() {
    assert_eq!(oracle_mult_for_budget(ORACLE_FULL_WORSTCASE_MS), 1.0);
    assert_eq!(
        oracle_mult_for_budget(ORACLE_FULL_WORSTCASE_MS + 50_000),
        1.0
    );
    assert_eq!(oracle_mult_for_budget(u128::MAX), 1.0);

    let half = oracle_mult_for_budget(ORACLE_FULL_WORSTCASE_MS / 2);
    assert!((half - 0.5).abs() < 1e-9, "got {half}");

    // At the live 6000 ms oracle pre-start gate the floor does NOT bind
    // (6000/30000 = 0.2 > ORACLE_MULT_MIN), so the oracle still does real work.
    let at_gate = oracle_mult_for_budget(6_000);
    assert!((at_gate - 0.2).abs() < 1e-9, "got {at_gate}");
    assert!(at_gate > ORACLE_MULT_MIN);

    // Floor and never-zero: even a tiny/zero remaining budget clamps up to
    // the utility floor (soundness holds at any value; this is not a budget).
    assert_eq!(oracle_mult_for_budget(0), ORACLE_MULT_MIN);
    assert_eq!(oracle_mult_for_budget(1), ORACLE_MULT_MIN);

    let mut prev = 0.0;
    for ms in [0u128, 1_000, 6_000, 15_000, 29_999, 30_000, 100_000] {
        let m = oracle_mult_for_budget(ms);
        assert!((ORACLE_MULT_MIN..=1.0).contains(&m), "ms={ms} m={m}");
        assert!(m >= prev, "not monotonic at ms={ms}: {m} < {prev}");
        prev = m;
    }
}

/// The projected pre-passes (integer PMC and weighted PWMC) cap the
/// uninterruptible oracle at `PROJECTED_ORACLE_MAX_VARS_DEFAULT` so large
/// formulas skip it: on those the oracle can overrun the whole budget while the
/// cheap pipeline reaches the same reduction. Guards:
///   * the default is finite (a regression reintroducing `u32::MAX` would let
///     the oracle run on the overrun class again) and strictly below the var
///     count of a formula in that class, so it and larger ones skip the oracle
///     on BOTH projected paths;
///   * the shared env reader honors a set value and falls back to the given
///     default when the variable is absent — and reports a value it cannot
///     parse rather than quietly using the default.
#[test]
fn projected_oracle_cap() {
    const OVERRUN_CLASS_VARS: u32 = 129_689;
    // Both guards hold over constants alone, so they are checked while the test
    // crate compiles and a regression cannot even build.
    const {
        assert!(
            PROJECTED_ORACLE_MAX_VARS_DEFAULT < OVERRUN_CLASS_VARS,
            "default cap must be below the var count of the overrun class so the \
             oracle is skipped there"
        );
        assert!(
            PROJECTED_ORACLE_MAX_VARS_DEFAULT < u32::MAX,
            "default cap must be finite — u32::MAX reintroduces the overrun"
        );
    }

    let cap = |v: Option<&str>, default: u32| {
        crate::env::parse_value(
            "VITRI_PMC_ARJUN_ORACLE_MAX_VARS",
            v,
            default,
            ORACLE_MAX_VARS_FORM,
        )
    };
    assert_eq!(
        cap(None, PROJECTED_ORACLE_MAX_VARS_DEFAULT).unwrap(),
        PROJECTED_ORACLE_MAX_VARS_DEFAULT
    );
    // env present ⇒ overrides the default, per-track:
    // VITRI_PMC_/VITRI_PWMC_ARJUN_ORACLE_MAX_VARS.
    assert_eq!(
        cap(Some("50000"), PROJECTED_ORACLE_MAX_VARS_DEFAULT).unwrap(),
        50_000
    );
    // A value that cannot be a cap is reported, not quietly replaced by the
    // default — a typo'd knob must not read as "no override".
    assert!(cap(Some("not-a-number"), 100).is_err());
}

#[test]
fn arjun_config_resolves() {
    assert_eq!(parse_arjun_effort(None), Ok(ArjunEffort::Full));
    assert_eq!(parse_arjun_effort(Some("full")), Ok(ArjunEffort::Full));
    assert_eq!(parse_arjun_effort(Some("  full ")), Ok(ArjunEffort::Full));
    assert_eq!(parse_arjun_effort(Some("FULL")), Ok(ArjunEffort::Full));
    assert_eq!(parse_arjun_effort(Some("lite")), Ok(ArjunEffort::Lite));
    let err = parse_arjun_effort(Some("aggressive"))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("VITRI_ARJUN_EFFORT"),
        "err must name the var: {err}"
    );
    assert!(
        err.contains("full") && err.contains("lite"),
        "err must list values: {err}"
    );
    assert!(
        err.contains("must be") && err.contains("got \"aggressive\""),
        "err must have the shape every rejected value gets: {err}"
    );
}
