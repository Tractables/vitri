//! The argument grammar: every rejection, and the code the shell sees

use super::*;

/// Every long flag `--help` lists in its `OPTIONS:` block, in the order it
/// lists them.
///
/// Read off the text rather than restated here: the binary renders that block
/// from the one inventory it also dispatches against, so this reads the offer
/// and the test below checks the parser honours it.
fn flags_offered_by(stdout: &str) -> Vec<String> {
    let block = stdout
        .split("OPTIONS:\n")
        .nth(1)
        .expect("--help has an OPTIONS: block");
    block
        .lines()
        // The block ends at the next unindented section heading.
        .take_while(|l| l.is_empty() || l.starts_with(' '))
        // An option's own line is the only one whose text starts in the left
        // column; every continuation line is indented past it.
        .filter(|l| l.len() - l.trim_start().len() <= 8 && l.trim_start().starts_with('-'))
        .filter_map(|l| l.split_whitespace().find(|t| t.starts_with("--")))
        .map(str::to_string)
        .collect()
}

/// `--help` answers on stdout, exits 0, offers a flag the parser accepts on
/// every line of its option block, and names every file the bundle can contain.
///
/// Neither half is restated here: the flags are read back out of the help text
/// and put to the parser, and the file names come from the crate's own
/// constants — so a flag offered but not implemented, or a renamed emitted
/// file, fails rather than passing on a list that agrees with neither.
#[test]
fn help_names_every_flag_and_every_emitted_file() {
    for flag in ["--help", "-h"] {
        let r = run(&[flag]).exit(0);
        assert!(r.stderr.is_empty(), "help belongs on stdout: {}", r.stderr);
        let offered = flags_offered_by(&r.stdout);
        assert!(
            offered.len() >= 2 && offered.contains(&"--help".to_string()),
            "the option block should list the flags, got {offered:?}",
        );
        for token in &offered {
            let attempt = run(&[token]);
            assert!(
                !attempt.stderr.contains("unknown option"),
                "--help offers {token}, which the parser rejects:\n{}",
                attempt.stderr,
            );
        }
        for name in [
            REDUCED_CNF_NAME,
            PREPROCESS_RECORD_NAME,
            VTREE_NAME,
            COMPONENTS_JSON_NAME,
            COMPONENTS_DIR,
            CANDIDATES_DIR,
        ] {
            r.assert_stdout(name);
        }
    }
}

/// `--help` offers every base name the spec table holds: the two groups it
/// interpolates as lists, and the two it spells on its own.
///
/// The base table calls itself the single source for this vocabulary, so a name
/// added to it must arrive on the command line advertised, not merely accepted.
/// The standalone group is the one that had no accessor to check it with, and it
/// is the group whose names the help writes out by hand.
#[test]
fn help_offers_every_vtree_base() {
    let r = run(&["--help"]).exit(0);
    for name in decomposition_spec_names()
        .chain(baseline_spec_names())
        .chain(standalone_spec_names())
    {
        r.assert_stdout(name);
    }
}

#[test]
fn no_input_cnf_is_rejected() {
    run(&["-o", "d"]).exit(2).assert_stderr("no input CNF");
    run(&[]).exit(2).assert_stderr("no input CNF");
}

#[test]
fn a_missing_out_dir_is_rejected() {
    run(&["in.cnf"])
        .exit(2)
        .assert_stderr("--out-dir is required");
}

#[test]
fn a_second_positional_argument_is_rejected() {
    run(&["a.cnf", "b.cnf", "-o", "d"])
        .exit(2)
        .assert_stderr("expected exactly one input CNF");
}

/// An unknown option is quoted back verbatim, so a typo is visible in the
/// message rather than merely rejected.
#[test]
fn an_unknown_option_is_quoted_back() {
    let r = run(&["in.cnf", "-o", "d", "--bogus"]).exit(2);
    r.assert_stderr("unknown option");
    r.assert_stderr("\"--bogus\"");
}

/// Every flag that takes a value names ITSELF when the value is missing —
/// the failure mode this guards is one flag's message being copy-pasted onto
/// another's arm. `-o` reports the long spelling, which is the one the help
/// text documents.
#[test]
fn every_value_taking_flag_reports_its_own_missing_value() {
    for (flag, named) in [
        ("-o", "--out-dir"),
        ("--out-dir", "--out-dir"),
        ("--mode", "--mode"),
        ("--vtree", "--vtree"),
        ("--budget-ms", "--budget-ms"),
        ("--components", "--components"),
        ("--candidates", "--candidates"),
    ] {
        run(&["in.cnf", flag])
            .exit(2)
            .assert_stderr(&format!("{named} needs a value"));
    }
}

/// The rejected `--mode` value is echoed and every accepted one is listed —
/// `compile` included, which is not a competition track and so appears in no
/// `c t` header.
#[test]
fn a_bad_mode_lists_every_mode_it_accepts() {
    let r = run(&["in.cnf", "-o", "d", "--mode", "bogus"]).exit(2);
    r.assert_stderr("\"bogus\"");
    for mode in [Mode::Mc, Mode::Wmc, Mode::Pmc, Mode::Pwmc, Mode::Compile] {
        r.assert_stderr(mode.token());
    }
}

#[test]
fn a_bad_budget_ms_is_rejected() {
    run(&["in.cnf", "-o", "d", "--budget-ms", "notanumber"])
        .exit(2)
        .assert_stderr("--budget-ms expects an integer");
}

#[test]
fn a_bad_components_policy_names_both_policies() {
    let r = run(&["in.cnf", "-o", "d", "--components", "bogus"]).exit(2);
    r.assert_stderr("split");
    r.assert_stderr("whole");
}

#[test]
fn a_non_integer_candidate_count_is_rejected() {
    for bad in ["abc", "-1", "2.5"] {
        run(&["in.cnf", "-o", "d", "--candidates", bad])
            .exit(2)
            .assert_stderr("positive integer");
    }
}

/// `--candidates 0` is refused by the SHARED validator, in the validator's own
/// words — proof that the binary calls
/// [`vitri::config::RunConfig::validate`] rather than re-implementing the
/// rule, which is the only way an embedding caller and the tool can stay in
/// agreement about what is legal.
#[test]
fn a_zero_candidate_count_reaches_the_shared_validator() {
    run(&["in.cnf", "-o", "d", "--candidates", "0"])
        .exit(2)
        .assert_stderr("at least 1");
}

/// An unrecognized `--vtree` spec is a WRONG INVOCATION (2), not a failed run
/// (1), even though it is only discovered once construction is reached: the
/// thing that needs fixing is the command line. The message names the spec and
/// lists what would have been accepted.
#[test]
fn an_unknown_vtree_spec_is_a_usage_error() {
    let t = Scratch::new("badspec");
    let input = t.file("in.cnf", IRREDUCIBLE_5);
    let r = run(&[
        s(&input),
        "-o",
        s(&t.out("bundle")),
        "--vtree",
        "nosuchspec",
    ])
    .exit(2);
    r.assert_stderr("nosuchspec");
    r.assert_stderr("unknown vtree type");
    // The list of valid types is what makes the message actionable, so it is
    // part of the surface: a spec name this crate does not document — including
    // one carried over from some other tool's configuration — lands here and
    // gets told what it could have said instead.
    r.assert_stderr(DEFAULT_VTREE_SPEC);
}

/// A `--vtree force` axis the grammar does not accept is a WRONG INVOCATION (2),
/// named token by token. Neither an out-of-range value nor an unknown key is ever
/// taken as "leave that axis at its default".
#[test]
fn a_bad_force_axis_is_a_usage_error_naming_the_token() {
    let t = Scratch::new("badforce");
    let input = t.file("in.cnf", IRREDUCIBLE_5);
    for (spec, needle) in [
        ("force:dim=9", "9"),
        ("force:bogus=1", "bogus"),
        ("force:treeify=cut,feedback=2", "feedback"),
        ("force:sideways", "sideways"),
    ] {
        let r = run(&[s(&input), "-o", s(&t.out("bundle")), "--vtree", spec]).exit(2);
        r.assert_stderr(needle);
    }
}

/// The force-directed embedding is reachable from the command line — bare, with a
/// tree-ifier, and with axes — and the SAME spec on the same CNF writes the same
/// vtree file every time. That reproducibility is the property the spec exists
/// for, and it is not true of the deadline-truncated default, so it is pinned
/// here on the real binary rather than only in a unit test.
///
/// Both preprocessing stages are off, so what construction is handed is exactly
/// the formula written above and the comparison is between two identical builds
/// rather than between two different formulas.
#[test]
fn the_force_spec_is_reachable_and_reproducible() {
    let t = Scratch::new("force");
    let input = t.file("in.cnf", &wide_component_dimacs(None));
    for (tag, spec) in [
        ("bare", "force"),
        ("cut", "force:treeify=cut,dim=3"),
        ("axes", "force:feedback=2,restarts=2,clause-weight=short"),
    ] {
        let mut written: Vec<String> = Vec::new();
        for pass in 0..2 {
            let out = t.out(&format!("{tag}{pass}"));
            run(&[
                s(&input),
                "-o",
                s(&out),
                "--vtree",
                spec,
                "--no-arjun",
                "--no-simplify",
            ])
            .exit(0)
            .assert_stdout(spec);
            written.push(read(&out.join(VTREE_NAME)));
        }
        assert!(
            written[0].starts_with("vtree "),
            "{spec} must write an SDD-format vtree, got: {}",
            &written[0][..written[0].len().min(40)],
        );
        assert_eq!(
            written[0], written[1],
            "{spec} must write the same vtree on every run",
        );
    }
}

/// Every construction the portfolio can build is also reachable by name, which
/// includes the one that takes a FlowCutter incidence decomposition and
/// assembles it under its own rule. Asking for it by name writes an ordinary
/// bundle with an ordinary vtree in it. The edge-aligned assembly is here too,
/// spelled out of the general construction suffixes.
///
/// Reproducibility is deliberately not asserted — these run the same
/// wall-clock-bounded decomposition search as the rest of the
/// decomposition-based specs. What is asserted is that the file written is a
/// well-formed vtree over exactly this formula's variables.
#[test]
fn the_portfolio_combiner_specs_are_reachable_by_name() {
    let t = Scratch::new("combiners");
    let input = t.file("in.cnf", &wide_component_dimacs(None));
    for (tag, spec) in [
        ("hybrid", "flowcutter-incidence:assembly=hybrid"),
        // The step-budgeted effort shape, which is what reproduces the
        // portfolio candidate of the same name.
        (
            "hybrid-steps",
            "flowcutter-incidence:assembly=hybrid,budget=150000steps,iters=15",
        ),
        (
            "edge",
            "flowcutter-incidence:order=td-edge,assign=shallow,td-root=centroid",
        ),
    ] {
        let out = t.out(tag);
        run(&[
            s(&input),
            "-o",
            s(&out),
            "--vtree",
            spec,
            "--no-arjun",
            "--no-simplify",
        ])
        .exit(0)
        .assert_stdout(spec);
        // Against the formula the vtree was actually built over, read back off
        // the bundle, so the check cannot drift from what the run reduced to.
        let reduced = std::fs::File::open(out.join(REDUCED_CNF_NAME)).expect("open reduced.cnf");
        let (formula, _) = CnfFormula::from_dimacs(std::io::BufReader::new(reduced))
            .expect("the emitted CNF must parse");
        assert_well_formed_vtree(&read(&out.join(VTREE_NAME)), formula.num_vars);
    }
}

/// A step-budgeted FlowCutter spec runs on a formula small enough for the
/// `best=auto` size rule.
///
/// The two features meet only here: that mode assembles from the bag assignment
/// alone, so there is no candidate list to rank and `best=on` is refused. The
/// formula is small enough that the size rule would otherwise turn ranking on,
/// so this pins that the rule declines instead of building a spec the grammar
/// refuses.
#[test]
fn a_step_budgeted_flowcutter_spec_survives_the_best_upgrade() {
    let t = Scratch::new("stepbudget");
    let input = t.file("in.cnf", &wide_component_dimacs(None));
    for spec in [
        "flowcutter-primal:budget=100000steps,iters=10",
        "flowcutter-incidence:budget=100000steps,iters=10",
    ] {
        let out = t.out(spec.split(':').next().expect("a base name"));
        run(&[
            s(&input),
            "-o",
            s(&out),
            "--vtree",
            spec,
            "--no-arjun",
            "--no-simplify",
        ])
        .exit(0)
        .assert_stdout(spec);
        let reduced = std::fs::File::open(out.join(REDUCED_CNF_NAME)).expect("open reduced.cnf");
        let (formula, _) = CnfFormula::from_dimacs(std::io::BufReader::new(reduced))
            .expect("the emitted CNF must parse");
        assert_well_formed_vtree(&read(&out.join(VTREE_NAME)), formula.num_vars);
    }
}

/// The whole mapping in one place: an error class decides an exit code, and a
/// shell reads that code rather than the message. The invocation being wrong is
/// 2 and nothing ran; the invocation being fine and the work failing is 1.
#[test]
fn every_failure_class_maps_to_the_exit_code_its_reader_expects() {
    let t = Scratch::new("exitcodes");
    let good = t.file("in.cnf", IRREDUCIBLE_5);
    let malformed = t.file("bad.cnf", "p cnf 2 1\nthis is not a clause\n");

    // A bad argument, an inert or unhonorable request, and a variable this
    // crate cannot use: nothing ran.
    run(&[s(&good)]).exit(2);
    run(&[s(&good), "-o", s(&t.out("spec")), "--vtree", "nosuchspec"]).exit(2);
    run_with_env(
        &[s(&good), "-o", s(&t.out("env"))],
        &[("VITRI_ARJUN_SBVA", "not-a-value")],
    )
    .exit(2);

    // The invocation was fine and the work failed: a file that is not there,
    // and one that is not a CNF. Every other class this binary does not
    // recognize lands here too, by the catch-all arm.
    run(&[s(&t.out("absent.cnf")), "-o", s(&t.out("io"))]).exit(1);
    run(&[s(&malformed), "-o", s(&t.out("parse"))]).exit(1);

    run(&[s(&good), "-o", s(&t.out("ok"))]).exit(0);
}

/// A value flag given twice takes the LAST value — the earlier one is replaced
/// rather than merged with it or refused.
#[test]
fn a_repeated_value_flag_takes_the_last_value_it_was_given() {
    let t = Scratch::new("repeated");
    let input = t.file("in.cnf", IRREDUCIBLE_5);

    // The first spec would exit 2 on its own, so a run that succeeds under the
    // second is one where the second replaced it.
    let out = t.out("spec");
    run(&[
        s(&input),
        "-o",
        s(&out),
        "--vtree",
        "nosuchspec",
        "--vtree",
        "minfill-primal",
    ])
    .exit(0)
    .assert_stdout("minfill-primal");

    // The same for the flag that decides where everything lands.
    let first = t.out("first");
    let second = t.out("second");
    run(&[s(&input), "-o", s(&first), "-o", s(&second)]).exit(0);
    assert!(second.join(REDUCED_CNF_NAME).exists());
    assert!(
        !first.exists(),
        "the replaced value must not also be written to",
    );

    // ...and for a closed vocabulary, where both values are legal.
    run(&[
        s(&input),
        "-o",
        s(&t.out("mode")),
        "--mode",
        "compile",
        "--mode",
        "mc",
    ])
    .exit(0)
    .assert_stdout(&format!("mode {}", Mode::Mc.token()));
}

/// This tool has no version flag, under either spelling, so both fall through
/// to the unknown-option arm and are quoted back.
#[test]
fn asking_for_a_version_is_not_a_flag_this_tool_has() {
    for spelling in ["--version", "-V"] {
        let r = run(&["in.cnf", "-o", "d", spelling]).exit(2);
        r.assert_stderr("unknown option");
        r.assert_stderr(&format!("{spelling:?}"));
    }
}

/// A lone dash is an option this tool does not have, not a positional argument
/// standing for the input — it is quoted back like any other unknown one.
#[test]
fn a_lone_dash_is_refused_as_an_unknown_option() {
    let r = run(&["in.cnf", "-o", "d", "-"]).exit(2);
    r.assert_stderr("unknown option");
    r.assert_stderr("\"-\"");
}

/// The two closed vocabularies the command line takes are OFFERED, not merely
/// accepted: `--help` interpolates both tables, so a mode or a policy added to
/// one arrives advertised.
#[test]
fn help_lists_every_mode_and_every_component_policy_token() {
    let r = run(&["--help"]).exit(0);
    for token in Mode::names().chain(vitri::config::ComponentPolicy::names()) {
        r.assert_stdout(token);
    }
}

/// The usage text states what each exit code means, so the mapping above is
/// documented where a user looks for it rather than only pinned here.
#[test]
fn help_states_the_exit_status_of_each_failure_class() {
    let r = run(&["--help"]).exit(0);
    let block = r
        .stdout
        .split("EXIT STATUS:\n")
        .nth(1)
        .expect("--help has an EXIT STATUS: block");
    let codes: Vec<&str> = block
        .lines()
        // The block ends at the next unindented section heading.
        .take_while(|l| l.is_empty() || l.starts_with(' '))
        .filter_map(|l| l.split_whitespace().next())
        .filter(|t| t.chars().all(|c| c.is_ascii_digit()))
        .collect();
    assert_eq!(
        codes,
        vec!["0", "1", "2"],
        "each status a run can exit with needs its own entry, got:\n{block}",
    );
    assert!(
        block.contains("VITRI_"),
        "the wrong-invocation entry must say a variable can put a run there:\n{block}",
    );
}

/// Assert that `text` is a well-formed SDD-format vtree over `num_vars`
/// variables: the header states the node count, every variable 1..=`num_vars`
/// labels exactly one leaf, every internal node names children declared before
/// it, and exactly one node is nobody's child.
fn assert_well_formed_vtree(text: &str, num_vars: u32) {
    let (header, node_lines) = tokenize_vtree_text(text);
    let mut declared: BTreeSet<u32> = BTreeSet::new();
    let mut children: BTreeSet<u32> = BTreeSet::new();
    let mut leaf_vars: BTreeSet<u32> = BTreeSet::new();
    let mut declared_nodes = 0usize;
    for f in &node_lines {
        match f.first().map(String::as_str) {
            Some("L") => {
                let id: u32 = f[1].parse().expect("a leaf line names its node id");
                let var: u32 = f[2].parse().expect("a leaf line names its variable");
                assert!(declared.insert(id), "node {id} is declared twice");
                assert!(leaf_vars.insert(var), "variable {var} labels two leaves");
                declared_nodes += 1;
            }
            Some("I") => {
                let id: u32 = f[1].parse().expect("an internal line names its node id");
                for child in [
                    f[2].parse::<u32>().expect("a left child id"),
                    f[3].parse::<u32>().expect("a right child id"),
                ] {
                    assert!(
                        declared.contains(&child),
                        "node {id} names child {child}, which is declared after it",
                    );
                    assert!(children.insert(child), "node {child} has two parents");
                }
                assert!(declared.insert(id), "node {id} is declared twice");
                declared_nodes += 1;
            }
            _ => {}
        }
    }
    assert_eq!(header, declared_nodes, "the header must count the nodes");
    assert_eq!(
        leaf_vars,
        (1..=num_vars).collect::<BTreeSet<u32>>(),
        "the leaves must cover every variable exactly once",
    );
    let roots: Vec<u32> = declared.difference(&children).copied().collect();
    assert_eq!(
        roots.len(),
        1,
        "a vtree has exactly one root, got {roots:?}"
    );
}

/// `--help` offers every base and every parameter the grammar accepts, each
/// with the values it takes and what leaving it out means.
///
/// The blurb is rendered from the parser's own tables, so this is not a second
/// copy of the vocabulary to keep in step — it is the check that the rendering
/// reaches all of it, on the surface a caller reads before they type a spec.
#[test]
fn help_offers_every_vtree_base_and_parameter() {
    let stdout = run(&["--help"]).exit(0).stdout;
    for base in vtree_spec_bases() {
        assert!(
            stdout.contains(&base),
            "--help does not offer the base {base}",
        );
        for p in spec_param_docs(&base) {
            assert!(
                stdout.contains(&format!("{}=", p.key)),
                "--help does not offer {}=, which {base} takes",
                p.key,
            );
            assert!(
                stdout.contains(&p.values) && stdout.contains(p.default),
                "--help does not say what {}= takes and defaults to",
                p.key,
            );
        }
    }
}
