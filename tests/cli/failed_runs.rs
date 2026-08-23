//! Runs that started and failed: exit 1, and no `--help` pointer

use super::*;

/// The 1-versus-2 split, stated on one input each way round. A missing file is
/// a runtime failure however well formed the command line was; a bad flag is a
/// wrong invocation however present the file is. Only the second is answered
/// by `--help`, so only the second points there.
#[test]
fn a_runtime_failure_exits_1_and_does_not_point_at_help() {
    let t = Scratch::new("nofile");
    let missing = t.out("nosuch.cnf");
    let r = run(&[s(&missing), "-o", s(&t.out("bundle"))]).exit(1);
    r.assert_stderr("cannot open");
    r.assert_stderr("nosuch.cnf");
    assert!(
        !r.stderr.contains("--help"),
        "a runtime failure is not answered by the usage text:\n{}",
        r.stderr,
    );

    let bad = run(&["--bogus"]).exit(2);
    bad.assert_stderr("run `vitri --help` for usage");
}

#[test]
fn a_cnf_declaring_no_variables_is_refused() {
    let t = Scratch::new("zerovar");
    for text in ["p cnf 0 0\n", "", "c only a comment\n"] {
        let input = t.file("zero.cnf", text);
        run(&[s(&input), "-o", s(&t.out("bundle"))])
            .exit(1)
            .assert_stderr("declares no variables");
    }
}

/// A CNF the library parser rejects surfaces as the CLI's own `error: parsing
/// <path>: <reason>` — the parser's sentence, kept, with the file it was
/// reading attached.
#[test]
fn malformed_dimacs_is_reported_as_a_parse_failure() {
    let t = Scratch::new("badcnf");
    let input = t.file("bad.cnf", "p cnf 2 1\n1 two 0\n");
    let r = run(&[s(&input), "-o", s(&t.out("bundle"))]).exit(1);
    r.assert_stderr("parsing");
    r.assert_stderr("bad.cnf");
    r.assert_stderr("invalid token");

    let headerless = t.file("noheader.cnf", "1 2 0\n");
    run(&[s(&headerless), "-o", s(&t.out("bundle2"))])
        .exit(1)
        .assert_stderr("parsing");
}

/// A variable id above the `p cnf` count is an INPUT defect, and the parser is
/// where it is caught — so it comes back as the same `error: parsing <path>:
/// <reason>` any other malformed file does, exit 1.
///
/// Each construct used to die somewhere else entirely: a clause literal above
/// the count indexed past the end of a per-variable table inside preprocessing
/// (a panic, exit 101), or exhausted memory inside the vendored C++ with both
/// preprocessing stages off; a `c p show` variable above the count reached Arjun's
/// own sampling-set assertion and killed the process (exit 134). Every stage
/// combination is asserted here, because the point of checking in the parser is
/// that it fires before any of them is chosen.
#[test]
fn a_variable_id_above_the_declared_count_is_a_parse_failure() {
    let t = Scratch::new("idrange");
    let out = t.out("bundle");

    // (file, contents, the construct and id the message must name, the line)
    let cases: [(&str, &str, &str, &str); 3] = [
        (
            "clause.cnf",
            CLAUSE_ID_ABOVE_COUNT,
            "clause literal 5",
            "line 2:",
        ),
        ("show.cnf", SHOW_ID_ABOVE_COUNT, "show var 9", "line 3:"),
        (
            "weight.cnf",
            "c t wmc\np cnf 2 1\nc p weight 9 1/3 0\n1 2 0\n",
            "weight literal 9",
            "line 3:",
        ),
    ];

    for (name, text, construct, line) in cases {
        let input = t.file(name, text);
        for stages in [
            &[][..],
            &["--no-arjun"][..],
            &["--no-simplify"][..],
            &["--no-arjun", "--no-simplify"][..],
        ] {
            let mut args = vec![s(&input), "-o", s(&out)];
            args.extend_from_slice(stages);
            let r = run(&args).exit(1);
            r.assert_stderr("parsing");
            r.assert_stderr(name);
            r.assert_stderr(construct);
            r.assert_stderr("declared variable count 2");
            r.assert_stderr(line);
        }
        assert!(!out.exists(), "a rejected input leaves no bundle behind");
    }
}

/// The output directory cannot be created because a FILE is already there: the
/// invocation was fine and the write failed, so this is 1.
#[test]
fn an_out_dir_that_is_an_existing_file_fails_the_run() {
    let t = Scratch::new("outfile");
    let input = t.file("in.cnf", IRREDUCIBLE_5);
    let blocker = t.file("blocker", "not a directory\n");
    run(&[s(&input), "-o", s(&blocker)])
        .exit(1)
        .assert_stderr("blocker");
}
