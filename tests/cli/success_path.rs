//! The success path: which files, named where, under which flag

use super::*;

/// The documented file set for a connected instance under default flags:
/// exactly four files, no `components/` (the manifest points at the top-level
/// pair instead) and no `candidates/`. Every one is also named on stdout, and
/// the summary carries each of the lines the README's example shows.
#[test]
fn a_default_run_writes_exactly_the_documented_file_set() {
    let t = Scratch::new("default");
    let input = t.file("in.cnf", IRREDUCIBLE_5);
    let out = t.out("bundle");
    let r = run(&[s(&input), "-o", s(&out)]).exit(0);

    assert_eq!(
        entries(&out),
        set(&[
            REDUCED_CNF_NAME,
            PREPROCESS_RECORD_NAME,
            VTREE_NAME,
            COMPONENTS_JSON_NAME,
        ]),
    );
    for line in [
        "input:",
        "reduced:",
        "vtree:",
        "components:",
        "wrote:",
        "elapsed:",
    ] {
        r.assert_stdout(line);
    }
    for name in [
        REDUCED_CNF_NAME,
        PREPROCESS_RECORD_NAME,
        VTREE_NAME,
        COMPONENTS_JSON_NAME,
    ] {
        r.assert_stdout(s(&out.join(name)));
    }
    assert!(
        !r.stdout.contains("candidates:"),
        "no candidate set was asked for:\n{}",
        r.stdout,
    );
    // The vtree line names the spec that was asked for.
    r.assert_stdout(&format!("vtree:        {DEFAULT_VTREE_SPEC} "));
}

/// `--dot` reaches the writer: a `.dot` with the same stem lands beside every
/// `.vtree` the run emits, the whole-formula one and each component's, and
/// each is named in the summary.
#[test]
fn the_dot_flag_puts_a_picture_beside_every_vtree() {
    let t = Scratch::new("dot");
    let input = t.file("in.cnf", TWO_COMPONENTS);
    let out = t.out("bundle");
    let r = run(&[s(&input), "-o", s(&out), "--dot"]).exit(0);

    let dot = out.join("vtree.dot");
    assert!(dot.exists(), "a picture beside the whole-formula vtree");
    assert!(read(&dot).contains("graph vtree {"), "a Graphviz document");
    r.assert_stdout(s(&dot));

    let mut pictured = 0;
    for name in entries(&out.join(COMPONENTS_DIR)) {
        if name.ends_with(".vtree") {
            let sibling = out
                .join(COMPONENTS_DIR)
                .join(name.replace(".vtree", ".dot"));
            assert!(sibling.exists(), "missing {}", sibling.display());
            pictured += 1;
        }
    }
    assert_eq!(pictured, 2, "the fixture's two component vtrees");
}

/// Every path the manifest names resolves from the bundle directory, and the
/// files the summary claims to have written are the files that are there. The
/// library tests prove the manifest is internally correct; this is the CLI's
/// own bookkeeping.
#[test]
fn every_manifest_path_resolves_and_is_named_on_stdout() {
    let t = Scratch::new("paths");
    let input = t.file("in.cnf", TWO_COMPONENTS);
    let out = t.out("bundle");
    let r = run(&[s(&input), "-o", s(&out)]).exit(0);

    let manifest = json(&out.join(COMPONENTS_JSON_NAME));
    let components = manifest["components"].as_array().expect("components[]");
    assert_eq!(components.len(), 2, "the fixture is disconnected");
    for entry in components {
        for field in ["cnf", "vtree"] {
            let rel = entry[field].as_str().expect("a relative path");
            assert!(
                out.join(rel).exists(),
                "{field} = {rel} does not resolve from the bundle directory",
            );
            assert!(
                rel.starts_with(COMPONENTS_DIR),
                "a split instance keeps its component files in {COMPONENTS_DIR}/, got {rel}",
            );
        }
    }
    r.assert_stdout(s(&out.join(COMPONENTS_DIR)));
    r.assert_stdout("components:   2");
}

/// `--components whole` really suppresses the split rather than being accepted
/// and ignored: a disconnected formula comes back as ONE entry covering all of
/// it, and no per-component files are written.
#[test]
fn the_whole_formula_policy_suppresses_the_split() {
    let t = Scratch::new("whole");
    let input = t.file("in.cnf", TWO_COMPONENTS);
    let out = t.out("bundle");
    let r = run(&[s(&input), "-o", s(&out), "--components", "whole"]).exit(0);

    let manifest = json(&out.join(COMPONENTS_JSON_NAME));
    assert_eq!(
        manifest["components"]
            .as_array()
            .expect("components[]")
            .len(),
        1
    );
    assert!(
        !out.join(COMPONENTS_DIR).exists(),
        "one entry points at the top-level files, so nothing is copied",
    );
    r.assert_stdout("components:   1");
}

/// Component file names are zero-padded to three digits, so an instance with
/// more than ten components does not fall off the format. Every fixture the
/// library tests use has at most two, which exercises `comp000` / `comp001`
/// and nothing else.
///
/// Both preprocessing stages are off: the split under test is then exactly the
/// one written here, rather than whatever preprocessing happened to leave.
#[test]
fn component_file_names_are_zero_padded_past_one_digit() {
    let t = Scratch::new("padding");
    let input = t.file("many.cnf", &many_components(13));
    let out = t.out("bundle");
    let r = run(&[s(&input), "-o", s(&out), "--no-arjun", "--no-simplify"]).exit(0);
    r.assert_stdout("components:   13");

    let names = entries(&out.join(COMPONENTS_DIR));
    for want in ["comp000.cnf", "comp009.cnf", "comp010.cnf", "comp012.cnf"] {
        assert!(names.contains(want), "missing {want} among {names:?}");
    }
    let manifest = json(&out.join(COMPONENTS_JSON_NAME));
    let last = &manifest["components"][12];
    assert_eq!(
        last["cnf"].as_str(),
        Some(format!("{COMPONENTS_DIR}/comp012.cnf").as_str()),
        "the manifest and the file name must agree on the padding",
    );
}

/// `--candidates N` reaches the retention policy: the runners-up land under
/// `candidates/` with rank in their names, the manifest says which score they
/// are ordered by, and the summary prints the candidate set block that has no other
/// coverage.
///
/// Deliberately no assertion on how MANY candidates come back: specs that
/// converge on one tree are collapsed into a single entry, so the honest
/// number is a property of the formula, not of `N`.
#[test]
fn a_requested_candidate_set_writes_ranked_runner_up_files_and_reports_them() {
    let t = Scratch::new("candidates");
    let input = t.file("wide.cnf", &wide_component_dimacs(None));
    let out = t.out("bundle");
    let r = run(&[s(&input), "-o", s(&out), "--candidates", "3"]).exit(0);

    let manifest = json(&out.join(COMPONENTS_JSON_NAME));
    assert_eq!(manifest["candidate_rank_metric"], "clause_load_stddev");
    r.assert_stdout("candidates:   ranked by clause_load_stddev (ascending — lower is better)");
    r.assert_stdout("component 000:");

    let candidates = manifest["components"][0]["vtree_candidates"]
        .as_array()
        .expect("a candidate set was asked for");
    assert!(!candidates.is_empty());
    assert_eq!(
        candidates[0]["vtree"], manifest["components"][0]["vtree"],
        "entry 0 is the SELECTED vtree and points back at it, not at a copy",
    );
    for entry in candidates {
        assert_eq!(keys(entry), set(&["built_by", "vtree", "scores"]));
        let path = entry["vtree"].as_str().expect("a path");
        assert!(out.join(path).exists(), "{path} does not resolve");
    }
    for entry in candidates.iter().skip(1) {
        let path = entry["vtree"].as_str().expect("a path");
        let name = path.rsplit('/').next().expect("a file name");
        assert!(
            path.starts_with(CANDIDATES_DIR)
                && name.starts_with("comp000.rank")
                && name.ends_with(".vtree"),
            "a runner-up is `{CANDIDATES_DIR}/compNNN.rankRR.vtree`, got {path}",
        );
    }
}

/// The rank metric follows the counting MODE, not the formula: a projected
/// instance is ranked on the widest cut its compile has to carry, because that
/// is what a projected compile pays for.
#[test]
fn a_projected_candidate_set_is_ranked_on_the_metric_a_projected_compile_pays() {
    let t = Scratch::new("candproj");
    let input = t.file("widep.cnf", &wide_component_dimacs(Some("c t pmc\n")));
    let out = t.out("bundle");
    let r = run(&[s(&input), "-o", s(&out), "--candidates", "4"]).exit(0);

    assert_eq!(
        json(&out.join(COMPONENTS_JSON_NAME))["candidate_rank_metric"],
        "peak_context_width_show",
    );
    r.assert_stdout("ranked by peak_context_width_show");
}

/// With both preprocessing stages off there is nothing left to remove, so the
/// emitted formula is the input's own clause set under the identity map. This
/// is what makes `--no-arjun --no-simplify` usable as the control arm the
/// tests above rely on, and it pins that the two flags reach
/// [`vitri::config::PreprocessStages`] rather than being accepted and ignored.
#[test]
fn disabling_both_stages_leaves_the_formula_as_it_was() {
    let t = Scratch::new("nostages");
    let input = t.file("in.cnf", IRREDUCIBLE_5);
    let out = t.out("bundle");
    run(&[s(&input), "-o", s(&out), "--no-arjun", "--no-simplify"]).exit(0);

    let record = json(&out.join(PREPROCESS_RECORD_NAME));
    assert_eq!(record["original_num_vars"], 5);
    assert_eq!(
        record["reduced_to_original_dimacs"]
            .as_array()
            .expect("the map"),
        &(1..=5).map(Value::from).collect::<Vec<_>>(),
    );
    assert_eq!(record["count_lift_pow2"], 0);
    assert_eq!(record["weight_lift"], "1/1");

    let cnf = read(&out.join(REDUCED_CNF_NAME));
    assert!(cnf.starts_with("p cnf 5 5\n"), "{cnf}");
    for clause in ["1 2 0", "-1 3 0", "-2 -3 4 0", "2 3 -4 0", "4 5 0"] {
        assert!(cnf.lines().any(|l| l == clause), "missing {clause}:\n{cnf}");
    }

    // ...and each flag on its own is accepted too, so neither depends on the
    // other to be legal.
    for flag in ["--no-arjun", "--no-simplify"] {
        run(&[s(&input), "-o", s(&t.out(&flag[2..])), flag]).exit(0);
    }
}

/// A stage toggle naming a stage the run's mode does not have is refused, and
/// the message names both halves of the mismatch. The projected chain has no
/// simplify chain and `compile` has no Arjun stage, so under those modes the
/// flag could only be accepted and dropped.
///
/// 2, not 1: nothing ran, the invocation was wrong.
#[test]
fn a_stage_flag_the_mode_has_no_stage_for_is_refused() {
    let t = Scratch::new("inertstage");
    let plain = t.file("plain.cnf", IRREDUCIBLE_5);
    let projected = t.file("proj.cnf", PROJECTED_WEIGHTED);

    for (input, mode, flag, stage) in [
        (&projected, "pmc", "--no-simplify", "simplify"),
        (&projected, "pwmc", "--no-simplify", "simplify"),
        (&plain, "compile", "--no-arjun", "Arjun"),
    ] {
        let out = t.out(&format!("{mode}{flag}"));
        let r = run(&[s(input), "-o", s(&out), "--mode", mode, flag]).exit(2);
        r.assert_stderr(flag);
        r.assert_stderr(&format!("mode {mode}"));
        r.assert_stderr(stage);
        assert!(
            !out.exists(),
            "a refused invocation leaves no bundle behind"
        );
    }
}

/// The same refusal on the DETECTED route: no `--mode` on the command line, the
/// projected mode read off the instance's own `c p show` header. The message
/// has to say the mode was detected, or the user goes looking for a `--mode`
/// they never typed.
#[test]
fn a_stage_flag_inert_under_a_detected_mode_is_refused_too() {
    let t = Scratch::new("inertdetected");
    // `IRREDUCIBLE_5` with a `c p show` line spliced in after the header, which
    // no constant can express. No `c t` line either: the show set alone is what
    // makes this projected.
    let input = t.file(
        "show.cnf",
        "p cnf 5 5\nc p show 2 4 5 0\n1 2 0\n-1 3 0\n-2 -3 4 0\n2 3 -4 0\n4 5 0\n",
    );
    let out = t.out("bundle");
    let r = run(&[s(&input), "-o", s(&out), "--no-simplify"]).exit(2);
    r.assert_stderr("--no-simplify");
    r.assert_stderr("mode pmc");
    r.assert_stderr("detected");
    assert!(
        !out.exists(),
        "a refused invocation leaves no bundle behind"
    );
}

/// The other side of the same rule: under every mode whose preprocessing DOES
/// have the stage, the flag is still accepted. Without this the refusal above could
/// be over-broad and nothing would say so.
#[test]
fn a_stage_flag_the_mode_does_have_a_stage_for_is_accepted() {
    let t = Scratch::new("livestage");
    let plain = t.file("plain.cnf", IRREDUCIBLE_5);
    let projected = t.file("proj.cnf", PROJECTED_WEIGHTED);

    for (input, mode, flag) in [
        (&plain, "mc", "--no-simplify"),
        (&plain, "wmc", "--no-simplify"),
        (&plain, "compile", "--no-simplify"),
        (&plain, "mc", "--no-arjun"),
        (&plain, "wmc", "--no-arjun"),
        (&projected, "pmc", "--no-arjun"),
        (&projected, "pwmc", "--no-arjun"),
    ] {
        run(&[
            s(input),
            "-o",
            s(&t.out(&format!("{mode}{flag}"))),
            "--mode",
            mode,
            flag,
        ])
        .exit(0);
    }
}

/// `--budget-ms` is accepted and bounds the run rather than being rejected as
/// unknown. What it BUYS is a different (better) vtree, which is not a
/// deterministic observable — the budget is a hint the construction spends,
/// not a number echoed back — so this pins acceptance only.
#[test]
fn a_budget_hint_is_accepted() {
    let t = Scratch::new("budget");
    let input = t.file("in.cnf", IRREDUCIBLE_5);
    let out = t.out("bundle");
    run(&[s(&input), "-o", s(&out), "--budget-ms", "60000"]).exit(0);
    assert!(out.join(VTREE_NAME).exists());
}

/// A budget spent before vtree construction even starts is a hard failure, not
/// a degraded run: `--budget-ms 0` sets the whole-run deadline to the instant
/// the process started, so it has already passed by the time preprocessing hands
/// off to vtree construction. The run must exit 1 (a construction failure, not
/// a bad invocation) and leave the output directory untouched — no partial
/// bundle, because nothing is written until after the vtree is built.
#[test]
fn a_spent_budget_fails_construction_and_writes_nothing() {
    let t = Scratch::new("spent-budget");
    let input = t.file("in.cnf", IRREDUCIBLE_5);
    let out = t.out("bundle");
    let run = run(&[s(&input), "-o", s(&out), "--budget-ms", "0"]).exit(1);
    run.assert_stderr("every candidate failed");
    assert!(
        !out.exists(),
        "the output directory must not be created when construction fails",
    );
}

/// `--out-dir` is created if missing, and used AS-IS when it is not: a
/// directory the caller already keeps things in is written into, not cleared.
#[test]
fn an_out_dir_that_already_exists_is_written_into() {
    let t = Scratch::new("existing-out");
    let input = t.file("in.cnf", IRREDUCIBLE_5);
    let out = t.out("bundle");
    std::fs::create_dir_all(&out).expect("a directory the caller made first");
    let unrelated = out.join("notes.txt");
    std::fs::write(&unrelated, "kept").expect("a file of the caller's own");

    run(&[s(&input), "-o", s(&out)]).exit(0);

    assert!(out.join(REDUCED_CNF_NAME).exists());
    assert!(out.join(VTREE_NAME).exists());
    assert_eq!(
        read(&unrelated),
        "kept",
        "an existing directory is used as-is, so unrelated files survive",
    );
}
