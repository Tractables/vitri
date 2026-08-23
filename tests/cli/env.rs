//! The environment boundary

use super::*;

/// One knob `docs/env.md` documents: a value its own table row names, a value
/// it cannot mean, and the mode whose run reaches its reader.
///
/// `bad` is `None` for the two knobs that refuse nothing — the tolerant budget
/// hint, and the trace switch, whose vocabulary is open by design.
struct Knob {
    var: &'static str,
    good: &'static str,
    bad: Option<&'static str>,
    mode: &'static str,
}

/// Every `VITRI_*` knob the documentation publishes. A knob added to that page
/// without a row here is one nothing drives from the command line.
const KNOBS: &[Knob] = &[
    Knob {
        var: "VITRI_BUDGET_MS",
        good: "60000",
        bad: None,
        mode: "mc",
    },
    Knob {
        var: "VITRI_PORTFOLIO_SEED",
        good: "7",
        bad: Some("not-a-value"),
        mode: "mc",
    },
    Knob {
        var: "VITRI_PORTFOLIO_TRACE",
        good: "all",
        bad: None,
        mode: "mc",
    },
    Knob {
        var: "VITRI_GOATD_REFINE_BUDGET_MS",
        good: "1000",
        bad: Some("not-a-value"),
        mode: "mc",
    },
    Knob {
        var: "VITRI_PMC_FLOWCUTTER_CAP_MS",
        good: "1000",
        bad: Some("not-a-value"),
        mode: "mc",
    },
    Knob {
        var: "VITRI_ARJUN_SBVA",
        good: "auto",
        bad: Some("not-a-value"),
        mode: "mc",
    },
    Knob {
        var: "VITRI_ARJUN_EFFORT",
        good: "lite",
        bad: Some("not-a-value"),
        mode: "mc",
    },
    Knob {
        var: "VITRI_ARJUN_KEEP_OVERRUN",
        good: "on",
        bad: Some("not-a-value"),
        mode: "mc",
    },
    Knob {
        var: "VITRI_PMC_ARJUN_ORACLE_MAX_VARS",
        good: "50000",
        bad: Some("not-a-value"),
        mode: "pmc",
    },
    Knob {
        var: "VITRI_PWMC_ARJUN_ORACLE_MAX_VARS",
        good: "50000",
        bad: Some("not-a-value"),
        mode: "pwmc",
    },
    Knob {
        var: "VITRI_ARJUN_EXPORT_LEARNED_CLAUSES",
        good: "1",
        bad: Some("not-a-value"),
        mode: "mc",
    },
    Knob {
        var: "VITRI_ARJUN_NO_BVE",
        // Presence-only: being set at all turns the pass off, so what it
        // refuses is a value that reads as leaving it on.
        good: "1",
        bad: Some("0"),
        mode: "mc",
    },
    Knob {
        var: "VITRI_ARJUN_BVE_GROW",
        good: "6",
        bad: Some("not-a-value"),
        mode: "mc",
    },
    Knob {
        var: "VITRI_ARJUN_NO_ORACLE",
        good: "1",
        bad: Some("off"),
        mode: "mc",
    },
    Knob {
        var: "VITRI_ARJUN_SEED",
        good: "12345",
        bad: Some("not-a-value"),
        mode: "mc",
    },
];

/// The instance a knob's run is given. The two projected oracle caps are read
/// by the projected pre-pass, so only a projected run consults them.
fn instance_for(mode: &str) -> &'static str {
    match mode {
        "pmc" | "pwmc" => PROJECTED_WEIGHTED,
        _ => IRREDUCIBLE_5,
    }
}

/// Run the binary over an instance the knob's own mode reaches, with `var` set
/// to `value` and every other `VITRI_*` stripped.
fn run_under(t: &Scratch, knob: &Knob, tag: &str, value: &str) -> Run {
    let input = t.file(&format!("{}-{tag}.cnf", knob.var), instance_for(knob.mode));
    let out = t.out(&format!("{}-{tag}", knob.var));
    run_with_env(
        &[s(&input), "-o", s(&out), "--mode", knob.mode],
        &[(knob.var, value)],
    )
}

/// Every documented knob takes the value its own row names, and the run
/// completes — which is what makes the table a description of the binary
/// rather than of a reader nothing reaches.
#[test]
fn every_documented_knob_accepts_the_value_its_table_row_names() {
    let t = Scratch::new("knobaccept");
    for knob in KNOBS {
        run_under(&t, knob, "good", knob.good).exit(0);
    }
}

/// A value a strict knob cannot mean stops the run, exits 2, and names both the
/// variable and the value as it was actually set — never a silent fallback to
/// the default, which would be a weaker run nobody asked for.
#[test]
fn every_strict_knob_refuses_a_value_it_cannot_mean_and_exits_two() {
    let t = Scratch::new("knobreject");
    for knob in KNOBS {
        let Some(bad) = knob.bad else { continue };
        let r = run_under(&t, knob, "bad", bad).exit(2);
        r.assert_stderr(knob.var);
        r.assert_stderr(bad);
    }
}

/// The empty string counts as SET, not as unset: a variable exported with no
/// value is a value the knob cannot mean, and is reported rather than read as
/// absent.
#[test]
fn the_empty_string_counts_as_set_for_every_strict_knob() {
    let t = Scratch::new("knobempty");
    for knob in KNOBS {
        if knob.bad.is_none() {
            continue;
        }
        let r = run_under(&t, knob, "empty", "").exit(2);
        r.assert_stderr(knob.var);
    }
}

/// `0` is how both millisecond knobs spell "no cap" — a legal value that leaves
/// the run as it was, not a cap of nothing.
#[test]
fn zero_is_the_spelling_of_no_cap_on_both_millisecond_knobs() {
    let t = Scratch::new("knobzero");
    for var in [
        "VITRI_GOATD_REFINE_BUDGET_MS",
        "VITRI_PMC_FLOWCUTTER_CAP_MS",
    ] {
        let input = t.file(&format!("{var}.cnf"), IRREDUCIBLE_5);
        let out = t.out(var);
        run_with_env(&[s(&input), "-o", s(&out)], &[(var, "0")]).exit(0);
        assert!(
            out.join(VTREE_NAME).exists(),
            "{var}=0 must leave construction its time, not cap it to nothing",
        );
    }
}

/// A presence-only switch turns its pass off by BEING SET, whatever it is set
/// to. So an off-looking value would do the opposite of what it says and is
/// refused, while anything else — including a word that means nothing in
/// particular — is obeyed.
#[test]
fn a_presence_only_switch_refuses_an_off_looking_value_and_obeys_any_other() {
    let t = Scratch::new("presenceonly");
    let input = t.file("in.cnf", IRREDUCIBLE_5);
    for var in ["VITRI_ARJUN_NO_BVE", "VITRI_ARJUN_NO_ORACLE"] {
        for off in ["0", "off", "false", "FALSE"] {
            let r = run_with_env(
                &[s(&input), "-o", s(&t.out(&format!("{var}-{off}")))],
                &[(var, off)],
            )
            .exit(2);
            r.assert_stderr(var);
        }
        for on in ["1", "please"] {
            run_with_env(
                &[s(&input), "-o", s(&t.out(&format!("{var}-{on}")))],
                &[(var, on)],
            )
            .exit(0);
        }
    }
}

/// A value past the top of a knob's type, or below its bottom, is refused
/// rather than wrapped into something the reader can hold — which would be a
/// budget or a seed nobody asked for.
#[test]
fn a_value_out_of_range_for_its_type_is_refused_not_wrapped() {
    let t = Scratch::new("knobrange");
    let input = t.file("in.cnf", IRREDUCIBLE_5);
    for (var, value) in [
        ("VITRI_ARJUN_BVE_GROW", "2147483648"),
        ("VITRI_ARJUN_BVE_GROW", "-1"),
        ("VITRI_ARJUN_SEED", "-1"),
        ("VITRI_PORTFOLIO_SEED", "-1"),
    ] {
        let r = run_with_env(
            &[s(&input), "-o", s(&t.out(&format!("{var}{value}")))],
            &[(var, value)],
        )
        .exit(2);
        r.assert_stderr(var);
        r.assert_stderr(value);
    }
}

/// The environment holds bytes, and a knob set to bytes that are not text is
/// reported as a value the reader cannot use — not skipped as though the
/// variable were unset.
#[cfg(unix)]
#[test]
fn a_non_utf8_knob_value_is_reported_rather_than_ignored() {
    use std::os::unix::ffi::OsStrExt;
    let t = Scratch::new("knobbytes");
    let input = t.file("in.cnf", IRREDUCIBLE_5);
    let out = t.out("bundle");
    let value = OsStr::from_bytes(&[b'a', 0xff, b'z']);
    let r = run_with_raw_env(&[s(&input), "-o", s(&out)], &[("VITRI_ARJUN_SBVA", value)]).exit(2);
    r.assert_stderr("VITRI_ARJUN_SBVA");
    r.assert_stderr("UTF-8");
    assert!(
        !out.exists(),
        "nothing may be written when the environment stopped the run",
    );
}

/// The harvest leaves no file behind, so the count is how a run that asked for
/// it tells "the stage derived none" from "the request went nowhere". It rides
/// the library's diagnostic channel, which this binary opts into, so it lands
/// on stderr.
#[test]
fn the_binary_reports_how_many_learnt_clauses_it_harvested() {
    let t = Scratch::new("learntreport");
    let input = t.file("in.cnf", IRREDUCIBLE_5);
    let out = t.out("bundle");
    let r = run_with_env(
        &[s(&input), "-o", s(&out), "--mode", "mc"],
        &[("VITRI_ARJUN_EXPORT_LEARNED_CLAUSES", "1")],
    )
    .exit(0);
    r.assert_stderr("learnt clauses");
    assert!(
        !entries(&out).iter().any(|n| n.contains("learnt")),
        "the clauses are a hint for the process holding the bundle, not a file",
    );
}

/// One stage of one chain harvests, so asking for the export under any other
/// mode — or with that stage switched off — is a mistake in the invocation:
/// exit 2, naming the variable and the mode that could have answered.
#[test]
fn a_learnt_clause_export_asked_of_a_mode_that_cannot_harvest_is_refused_at_the_command_line() {
    let t = Scratch::new("learntrefused");
    let input = t.file("in.cnf", IRREDUCIBLE_5);
    for mode in ["wmc", "compile"] {
        let out = t.out(mode);
        let r = run_with_env(
            &[s(&input), "-o", s(&out), "--mode", mode],
            &[("VITRI_ARJUN_EXPORT_LEARNED_CLAUSES", "1")],
        )
        .exit(2);
        r.assert_stderr("VITRI_ARJUN_EXPORT_LEARNED_CLAUSES");
        r.assert_stderr("preprocess under mc");
        assert!(
            !out.exists(),
            "nothing may be written when the request was refused",
        );
    }

    let out = t.out("stage-off");
    let r = run_with_env(
        &[s(&input), "-o", s(&out), "--mode", "mc", "--no-arjun"],
        &[("VITRI_ARJUN_EXPORT_LEARNED_CLAUSES", "1")],
    )
    .exit(2);
    r.assert_stderr("--no-arjun");
}

/// A malformed `VITRI_*` variable stops the run before any work, exits 2, and
/// names the variable and the form it wants.
///
/// Both constructors the binary starts from are covered: the preprocessing knobs
/// ([`vitri::config::RunConfig::from_env_defaults`]) and the construction
/// knobs ([`vitri::decompose::SelectionCtx::with_env_defaults`]). The `--help`
/// pointer is deliberately absent — the usage text describes flags, not
/// variables — and `docs/env.md`, which does describe them, is named instead.
#[test]
fn a_malformed_knob_stops_the_run_before_any_work() {
    let t = Scratch::new("badenv");
    let input = t.file("in.cnf", IRREDUCIBLE_5);
    for var in ["VITRI_ARJUN_SBVA", "VITRI_PORTFOLIO_SEED"] {
        let out = t.out(&format!("bundle-{var}"));
        let r = run_with_env(&[s(&input), "-o", s(&out)], &[(var, "not-a-value")]).exit(2);
        r.assert_stderr("environment variable");
        r.assert_stderr(var);
        r.assert_stderr("not-a-value");
        r.assert_stderr("docs/env.md");
        assert!(
            !r.stderr.contains("--help"),
            "the usage text does not document variables:\n{}",
            r.stderr,
        );
        assert!(
            !out.exists(),
            "nothing may be written when the environment stopped the run",
        );
    }
}

/// `--help` answers whatever the environment holds.
///
/// It is the one command a confused user reaches for, and a stale export in
/// their shell must not be what disables it: the argument loop reads no
/// variable, so the usage text is printed and the process exits 0 before the
/// value that would have stopped the run is ever looked at.
#[test]
fn help_answers_under_a_malformed_knob() {
    let r = run_with_env(&["--help"], &[("VITRI_ARJUN_SBVA", "bogus")]).exit(0);
    r.assert_stdout("USAGE:");
    assert!(r.stderr.is_empty(), "help belongs on stdout: {}", r.stderr);
}

/// The documented exception to that rule: `VITRI_BUDGET_MS` is a FALLBACK
/// under a budget the caller may set instead, and a hint it cannot parse
/// leaves the run unbounded exactly as an unset variable does. It is the one
/// knob that is resolved where it is consumed rather than at the boundary, so
/// it is also the one that stays tolerant.
#[test]
fn the_budget_hint_is_the_one_tolerant_knob() {
    let t = Scratch::new("walltolerant");
    let input = t.file("in.cnf", IRREDUCIBLE_5);
    let out = t.out("bundle");
    run_with_env(
        &[s(&input), "-o", s(&out)],
        &[("VITRI_BUDGET_MS", "not-a-number")],
    )
    .exit(0);
    assert!(out.join(PREPROCESS_RECORD_NAME).exists());
}
