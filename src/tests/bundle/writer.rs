use super::*;

/// Three variables and two clauses: enough that a show set can name a proper
/// subset of them, which is what a round trip through the show line has to
/// preserve.
const THREE_VARS: &str = "p cnf 3 2\n1 -2 0\n2 3 0\n";

#[test]
fn writer_omits_headers_when_absent() {
    let (formula, _) = parse("p cnf 2 1\n1 -2 0\n");
    let dir = Scratch::new("writer");
    let path = dir.out("out.cnf");
    write_dimacs(&formula, &DimacsHeader::<Reduced>::default(), &path).expect("write");
    let text = std::fs::read_to_string(&path).expect("read");
    assert!(
        !text.contains("c p show"),
        "no show set ⇒ no show line, got:\n{text}"
    );
    assert!(
        !text.contains("c p weight"),
        "no weights ⇒ no weight lines, got:\n{text}"
    );
    assert!(
        !text.contains("c t "),
        "no mode ⇒ no track header, got:\n{text}"
    );
    let (reparsed, meta) = parse(&text);
    assert_eq!(
        reparsed, formula,
        "the writer must round-trip the clause set"
    );
    assert!(meta.declared_show_vars().is_none());
    assert!(meta.declared_weights().is_none());
}

/// ...and it must emit every header it IS given, in a form this crate's own
/// parser reads back identically.
#[test]
fn writer_round_trips_every_header() {
    let (formula, _) = parse(THREE_VARS);
    let dir = Scratch::new("writer-headers");
    let path = dir.out("out.cnf");
    let weights = vec![
        LiteralWeight {
            literal: 1,
            weight: "1/3".into(),
        },
        LiteralWeight {
            literal: -1,
            weight: "5/7".into(),
        },
    ];
    write_dimacs(
        &formula,
        &DimacsHeader {
            track: Some("pwmc"),
            show: Some(&ShowSet::<Reduced>::from_dimacs_ids(&[1, 3]).expect("valid ids")),
            weights: Some(&weights),
        },
        &path,
    )
    .expect("write");
    let (reparsed, meta) = parse(&std::fs::read_to_string(&path).expect("read"));
    assert_eq!(reparsed, formula);
    assert_eq!(meta.mode, Mode::Pwmc);
    assert_eq!(
        meta.declared_show_vars().map(|s| s.to_dimacs()),
        Some(vec![1, 3]),
    );
    let w: Weights<Reduced> = meta.declared_weights().expect("weights").resolve(3);
    assert_eq!(
        w[VarId(0)],
        (rat(5, 7), rat(1, 3)),
        "polarity must survive the round trip"
    );
}

/// An EMPTY show set must round-trip as an empty show set, not as "unprojected".
///
/// A projection-set minimization can legitimately retire every show variable —
/// the answer is then 1 or 0 — while leaving variables in the formula that the
/// bounded BVE did not eliminate. Reading the emitted `c p show 0` back as "no
/// projection" would make a consumer count models over those leftover variables
/// instead, which is a silent miscount rather than a lost optimization. Found by
/// the randomized sweep, which reached exactly that shape.
#[test]
fn writer_round_trips_an_empty_show_set() {
    let (formula, _) = parse(THREE_VARS);
    let dir = Scratch::new("writer-empty-show");
    let path = dir.out("out.cnf");
    write_dimacs(
        &formula,
        &DimacsHeader {
            track: Some("pmc"),
            show: Some(&ShowSet::<Reduced>::empty()),
            ..Default::default()
        },
        &path,
    )
    .expect("write");
    let (_, meta) = parse(&std::fs::read_to_string(&path).expect("read"));
    assert_eq!(
        meta.declared_show_vars(),
        Some(&ShowSet::empty()),
        "`c p show 0` declares a projection onto nothing — it is not the absence of one",
    );
}
