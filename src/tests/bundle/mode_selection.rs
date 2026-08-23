use super::*;

/// Both projected declarations on one two-variable formula, under the track
/// that wants both: the file every case here needs when it has to distinguish
/// a show set from a weight table.
const PROJECTED_WEIGHTED: &str = "c t pwmc\np cnf 2 1\nc p show 1 0\nc p weight 1 1/2 0\n1 2 0\n";

/// With no override, the mode comes from the file — including from a bare `c p
/// show` / `c p weight` line with no `c t` header at all, which is the case where
/// silently counting plainly would be wrong.
#[test]
fn mode_is_detected_from_the_headers() {
    for (dimacs, expect) in [
        ("p cnf 2 1\n1 2 0\n", Mode::Mc),
        ("c t wmc\np cnf 2 1\n1 2 0\n", Mode::Wmc),
        ("p cnf 2 1\nc p weight 1 1/2 0\n1 2 0\n", Mode::Wmc),
        ("p cnf 2 1\nc p show 1 0\n1 2 0\n", Mode::Pmc),
        (PROJECTED_WEIGHTED, Mode::Pwmc),
    ] {
        let (_, meta) = parse(dimacs);
        let r = RunConfig::default()
            .resolve_mode(&meta)
            .expect("detection cannot fail");
        assert_eq!(r.mode, expect, "for {dimacs:?}");
        assert!(
            r.notices.is_empty(),
            "detection ignores nothing, so it reports nothing"
        );
    }
}

/// `c t` and `--mode` accept DIFFERENT token sets. A `c t` line names a
/// competition track, and `compile` is not one; what a file that declares it
/// anyway is answered with belongs to the parser and is pinned there.
#[test]
fn the_header_and_the_flag_take_different_tokens() {
    for tok in ["mc", "wmc", "pmc", "pwmc"] {
        assert_eq!(Mode::parse_track(tok), Mode::parse_mode(tok), "for {tok:?}");
        assert!(Mode::parse_track(tok).is_some(), "{tok} is a track");
    }
    assert_eq!(Mode::parse_mode("compile"), Some(Mode::Compile));
    assert_eq!(
        Mode::parse_track("compile"),
        None,
        "`c t compile` is not a track"
    );
    assert_eq!(Mode::parse_mode("nonsense"), None);
}

#[test]
fn an_explicit_mode_wins_and_reports_what_it_ignores() {
    let (_, projected) = parse("c t pmc\np cnf 2 1\nc p show 1 0\n1 2 0\n");
    let (_, weighted) = parse("c t wmc\np cnf 2 1\nc p weight 1 1/2 0\n1 2 0\n");
    let (_, both) = parse(PROJECTED_WEIGHTED);

    for (meta, asked, mentions) in [
        (&projected, Mode::Mc, vec!["show set"]),
        (&projected, Mode::Wmc, vec!["show set"]),
        (&weighted, Mode::Mc, vec!["weight"]),
        (&both, Mode::Mc, vec!["weight", "show set"]),
        (&both, Mode::Pmc, vec!["weight"]),
    ] {
        let config = RunConfig {
            mode: Some(asked),
            ..Default::default()
        };
        let r = config
            .resolve_mode(meta)
            .expect("a subsumed task is allowed, not refused");
        assert_eq!(r.mode, asked);
        assert_eq!(
            r.notices.len(),
            mentions.len(),
            "one notice per ignored declaration"
        );
        for (n, m) in r.notices.iter().zip(&mentions) {
            assert!(
                n.starts_with("c "),
                "a notice is comment-prefixed, got: {n}"
            );
            assert!(
                n.contains(m) && n.contains(asked.token()),
                "the notice must name what is ignored and the mode, got: {n}",
            );
        }
    }

    let config = RunConfig {
        mode: Some(Mode::Compile),
        ..Default::default()
    };
    let r = config
        .resolve_mode(&both)
        .expect("compile accepts any instance");
    assert_eq!(r.mode, Mode::Compile);
    assert!(
        r.notices.is_empty(),
        "compile drops nothing, so it reports nothing"
    );

    let config = RunConfig {
        mode: Some(Mode::Pmc),
        ..Default::default()
    };
    assert!(
        config
            .resolve_mode(&projected)
            .expect("same task")
            .notices
            .is_empty()
    );
}

/// A projection over no show set is inert, not narrower.
#[test]
fn a_mode_that_needs_absent_data_is_refused() {
    let (f, plain) = parse("p cnf 2 1\n1 2 0\n");
    for asked in [Mode::Pmc, Mode::Pwmc] {
        let config = RunConfig {
            mode: Some(asked),
            ..Default::default()
        };
        let err = config
            .resolve_mode(&plain)
            .expect_err("an inert mode must be refused")
            .to_string();
        assert!(
            err.contains(asked.token()) && err.contains("show"),
            "the error must name the mode and the missing declaration, got: {err}",
        );
        // The entry point refuses it too, not just resolve_mode.
        assert!(preprocess(&f, &plain, &config).is_err());
    }
}

#[test]
fn a_projected_track_header_with_no_show_set_is_refused() {
    let (_, no_header) = parse("p cnf 2 1\n1 2 0\n");
    for track in [Mode::Pmc, Mode::Pwmc] {
        let (f, meta) = parse(&format!("c t {}\np cnf 2 1\n1 2 0\n", track.token()));
        let explicit = RunConfig {
            mode: Some(track),
            ..Default::default()
        };

        let detected = RunConfig::default()
            .resolve_mode(&meta)
            .expect_err("a projected track header with no show set has nothing to project onto")
            .to_string();
        let asked = explicit
            .resolve_mode(&meta)
            .expect_err("the explicit route must refuse the same file")
            .to_string();
        let bare = explicit
            .resolve_mode(&no_header)
            .expect_err("the pre-existing refusal, for comparison")
            .to_string();
        assert_eq!(detected, asked, "both routes must refuse in the same words");
        assert_eq!(
            detected, bare,
            "and in the same words as a file that declares no track at all",
        );
        assert!(
            detected.contains(track.token()) && detected.contains("`c p show`"),
            "the error must name the mode and the missing declaration, got: {detected}",
        );

        assert!(preprocess(&f, &meta, &RunConfig::default()).is_err());
        assert!(preprocess(&f, &meta, &explicit).is_err());

        for ok in [Mode::Mc, Mode::Wmc, Mode::Compile] {
            let config = RunConfig {
                mode: Some(ok),
                ..Default::default()
            };
            assert_eq!(
                config
                    .resolve_mode(&meta)
                    .expect("not a projected mode")
                    .mode,
                ok,
            );
            assert!(preprocess(&f, &meta, &config).is_ok());
        }
    }
}

#[test]
fn a_mode_may_add_what_the_file_leaves_unstated() {
    let config = RunConfig {
        mode: Some(Mode::Wmc),
        ..Default::default()
    };
    let rt = round_trip_with(
        "mode-add",
        "p cnf 4 4\n\
         1 2 0\n\
         -1 3 0\n\
         -2 -3 4 0\n\
         2 3 -4 0\n",
        &config,
    );
    rt.assert_sound();
    assert_eq!(rt.record.mode, Mode::Wmc);
}

#[test]
fn a_weighted_file_preprocesses_under_mc() {
    let config = RunConfig {
        mode: Some(Mode::Mc),
        ..Default::default()
    };
    let rt = round_trip_with(
        "mode-downgrade",
        "c t wmc\n\
         p cnf 4 4\n\
         c p weight 1 1/3 0\n\
         c p weight -1 2/3 0\n\
         1 2 0\n\
         -1 3 0\n\
         -2 -3 4 0\n\
         2 3 -4 0\n",
        &config,
    );
    rt.assert_sound();
    assert_eq!(rt.record.mode, Mode::Mc);
    assert_eq!(
        rt.record.weight_lift, "1/1",
        "an unweighted mode has no weighted lift"
    );
    assert!(
        rt.record.reduced_weights.is_none(),
        "the weights are ignored, not carried"
    );
    assert!(!rt.reduced_cnf_text.contains("c p weight"));
}
