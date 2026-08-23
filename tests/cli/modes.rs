//! Mode resolution, through the command line

use super::*;

/// A projected mode with nothing to project onto is refused identically
/// whichever route chose the mode — the file's own `c t pmc` header, or
/// `--mode pmc` over a file that declares no track at all — and the refusal is
/// a clean exit 2 with a sentence, not a crash inside the chain.
#[test]
fn a_projected_mode_without_a_show_set_is_refused_on_both_routes() {
    let t = Scratch::new("noshow");
    for track in ["pmc", "pwmc"] {
        let declared = t.file("declared.cnf", &format!("c t {track}\np cnf 2 1\n1 2 0\n"));
        let bare = t.file("bare.cnf", "p cnf 2 1\n1 2 0\n");

        let detected = run(&[s(&declared), "-o", s(&t.out("a"))]).exit(2);
        let asked = run(&[s(&bare), "-o", s(&t.out("b")), "--mode", track]).exit(2);

        for r in [&detected, &asked] {
            r.assert_stderr(track);
            r.assert_stderr("`c p show`");
            r.assert_stderr("no show set to preserve");
        }
        assert_eq!(
            detected.stderr, asked.stderr,
            "both routes must refuse in the same words",
        );

        // Nothing is wrong with the FILE: a mode that needs no show set
        // reduces it.
        run(&[s(&declared), "-o", s(&t.out("c")), "--mode", "mc"]).exit(0);
    }
}

/// An explicit `--mode` WINS over the file's own headers, and every
/// declaration the chosen mode does not use is reported rather than dropped in
/// silence. The reduced file then describes the mode that ran, not the one the
/// input declared.
#[test]
fn an_explicit_mode_wins_over_the_headers_and_says_what_it_drops() {
    let t = Scratch::new("modewins");
    let input = t.file("pw.cnf", PROJECTED_WEIGHTED);

    let out = t.out("as-mc");
    let r = run(&[s(&input), "-o", s(&out), "--mode", "mc"]).exit(0);
    r.assert_stderr("c note: ignoring weight declarations (mode mc)");
    r.assert_stderr("c note: ignoring the projection show set (mode mc)");
    r.assert_stdout("mode mc");

    let record = json(&out.join(PREPROCESS_RECORD_NAME));
    assert_eq!(record["mode"], "mc");
    assert!(record.get("show_vars_reduced_dimacs").is_none());
    assert!(record.get("reduced_weights").is_none());
    // A counting mode eliminates variables a reduced model does not determine,
    // so there is no total map to write and the field is absent, not partial.
    assert!(record.get("original_to_reduced_dimacs").is_none());

    let cnf = read(&out.join(REDUCED_CNF_NAME));
    assert!(cnf.contains("\nc t mc\n"), "{cnf}");
    assert!(!cnf.contains("c p show"), "{cnf}");
    assert!(!cnf.contains("c p weight"), "{cnf}");

    // Narrowing one axis reports only that axis.
    let out = t.out("as-wmc");
    let r = run(&[s(&input), "-o", s(&out), "--mode", "wmc"]).exit(0);
    r.assert_stderr("c note: ignoring the projection show set (mode wmc)");
    assert!(
        !r.stderr.contains("ignoring weight declarations"),
        "a weighted mode uses the weights:\n{}",
        r.stderr,
    );
    assert_eq!(json(&out.join(PREPROCESS_RECORD_NAME))["mode"], "wmc");
}

/// `compile` is the mode no `c t` line can name. The reduced file therefore
/// carries no track header at all, while the declarations it was given — the
/// show set and the weights — travel through renumbered and unfolded, because
/// `compile` preserves the function rather than counting under them.
#[test]
fn compile_mode_writes_no_track_header_and_keeps_the_declarations() {
    let t = Scratch::new("compile");
    let input = t.file("pw.cnf", PROJECTED_WEIGHTED);
    let out = t.out("bundle");
    run(&[s(&input), "-o", s(&out), "--mode", "compile"]).exit(0);

    let record = json(&out.join(PREPROCESS_RECORD_NAME));
    assert_eq!(record["mode"], "compile");
    assert!(record["show_vars_reduced_dimacs"].is_array());
    assert!(record["reduced_weights"].is_array());
    // The map that makes the mode reconstructible: one entry per ORIGINAL
    // variable, so a consumer needs no second pass to find the ones the
    // preprocessing dropped.
    let total = record["original_to_reduced_dimacs"]
        .as_array()
        .expect("`compile` writes the total original→reduced map");
    assert_eq!(
        total.len() as u64,
        record["original_num_vars"].as_u64().expect("var count"),
    );

    let cnf = read(&out.join(REDUCED_CNF_NAME));
    assert!(
        !cnf.contains("c t "),
        "`compile` is not a track, so no `c t` line can name it:\n{cnf}",
    );
    assert!(cnf.contains("c p show"), "{cnf}");
    assert!(cnf.contains("c p weight"), "{cnf}");
}
