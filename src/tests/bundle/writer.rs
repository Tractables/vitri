use super::*;

/// Three variables and two clauses: enough that a show set can name a proper
/// subset of them, which is what a round trip through the show line has to
/// preserve.
const THREE_VARS: &str = "p cnf 3 2\n1 -2 0\n2 3 0\n";

/// What the DIMACS writer emits for `formula` under `header`.
fn written(formula: &CnfFormula, header: &DimacsHeader<'_, Reduced>) -> String {
    let mut bytes = Vec::new();
    write_dimacs(formula, header, &mut bytes).expect("writing into memory cannot fail");
    String::from_utf8(bytes).expect("the writer emits UTF-8")
}

#[test]
fn writer_omits_headers_when_absent() {
    let (formula, _) = parse("p cnf 2 1\n1 -2 0\n");
    let text = written(&formula, &DimacsHeader::<Reduced>::default());
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
    let text = written(
        &formula,
        &DimacsHeader {
            track: Some("pwmc"),
            show: Some(&ShowSet::<Reduced>::from_dimacs_ids(&[1, 3]).expect("valid ids")),
            weights: Some(&weights),
        },
    );
    let (reparsed, meta) = parse(&text);
    assert_eq!(reparsed, formula);
    assert_eq!(meta.mode(), Mode::Pwmc);
    assert_eq!(
        meta.declared_show_vars().map(|s| s.as_dimacs().to_vec()),
        Some(vec![1, 3]),
    );
    let w: Weights<Reduced> = meta.declared_weights().expect("weights").resolve(3);
    assert_eq!(
        w[VarId::from_dimacs(1)],
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
    let text = written(
        &formula,
        &DimacsHeader {
            track: Some("pmc"),
            show: Some(&ShowSet::<Reduced>::empty()),
            ..Default::default()
        },
    );
    let (_, meta) = parse(&text);
    assert_eq!(
        meta.declared_show_vars(),
        Some(&ShowSet::empty()),
        "`c p show 0` declares a projection onto nothing — it is not the absence of one",
    );
}
