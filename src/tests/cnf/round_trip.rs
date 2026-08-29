//! What the writer emits, the reader reads back.
//!
//! The two halves are one pair — the line kinds one accepts are the ones the
//! other emits — so the contract worth stating is the fixed point: parse, write,
//! parse again, over each meta line and over the clause shapes the reader
//! normalizes on the way in.

use super::*;
use crate::cnf::weights::LiteralWeight;
use crate::tests::common::{Scratch, parse};

/// Write `formula` under `header` and read it straight back, so no test spells
/// the two halves of the round trip itself.
fn write_then_read(
    tag: &str,
    formula: &CnfFormula,
    header: &DimacsHeader<'_, Original>,
) -> (CnfFormula, CnfMeta) {
    let scratch = Scratch::new(tag);
    let path = scratch.out("round-trip.cnf");
    write_dimacs(formula, header, &path).expect("the writer must produce the file");
    let text = std::fs::read_to_string(&path).expect("the file the writer just produced");
    parse(&text)
}

/// The sparse `c p weight` rows a parsed table declares, in the writer's own
/// row shape — exactly the literals the file named, no defaults filled in.
fn declared_rows(weights: &WeightTable) -> Vec<LiteralWeight> {
    weights
        .to_literal_pairs()
        .iter()
        .map(|(literal, w)| LiteralWeight {
            literal: *literal,
            weight: rational_string(w),
        })
        .collect()
}

/// A file carrying every meta line at once comes back as the same clause set
/// and the same declarations, so nothing a reader accepts is lost by writing it.
#[test]
fn a_parsed_formula_reparses_from_the_dimacs_it_writes() {
    let (formula, meta) = parse(
        "c t pwmc\np cnf 4 3\nc p show 1 3 0\nc p weight 1 1/2 0\nc p weight -1 3/4 0\n\
         1 -2 0\n2 3 4 0\n-3 4 0\n",
    );
    let declared = meta.weights.as_ref().expect("the fixture declares weights");
    let rows = declared_rows(declared);
    let header = DimacsHeader {
        track: meta.declared_track().map(Mode::token),
        show: meta.declared_show_vars(),
        weights: Some(&rows),
    };

    let (again, again_meta) = write_then_read("dimacs-fixed-point", &formula, &header);

    assert_eq!(again, formula, "the clause set changed through write→read");
    assert_eq!(
        again_meta.declared_track(),
        meta.declared_track(),
        "the track the file declared changed through write→read",
    );
    assert_eq!(
        again_meta.declared_show_vars(),
        meta.declared_show_vars(),
        "the show set changed through write→read",
    );
    assert_eq!(
        again_meta
            .weights
            .expect("the written weight lines must parse back")
            .to_literal_pairs(),
        declared.to_literal_pairs(),
        "the declared literal weights changed through write→read",
    );
}

/// The `c p show` line the writer emits is the set's own ascending, deduplicated
/// form, so reading it back yields the set it was written from rather than the
/// order the caller happened to assemble.
#[test]
fn a_written_show_set_comes_back_as_the_same_ascending_ids() {
    let formula = parse(TWO_DISJOINT_CLAUSES).0;
    let show = ShowSet::<Original>::from_dimacs_ids(&[3, 1, 3]).expect("1-based ids");
    let header = DimacsHeader {
        track: Some("pmc"),
        show: Some(&show),
        weights: None,
    };

    let (_, meta) = write_then_read("dimacs-show", &formula, &header);

    let read_back = meta
        .declared_show_vars()
        .expect("the writer emitted a `c p show` line");
    assert_eq!(read_back, &show, "the show set changed through write→read");
    assert_eq!(
        read_back.to_dimacs(),
        vec![1, 3],
        "the ids must come back ascending and deduplicated",
    );
}

/// A weight is an exact rational, and the writer's `numerator/denominator`
/// spelling carries it without an `f64` in the middle: a fraction, a negative
/// literal's weight and a scientific-notation value all resolve back to the
/// table they were written from.
#[test]
fn a_written_weight_row_comes_back_as_the_same_exact_rational() {
    let formula = parse("p cnf 2 1\n1 2 0\n").0;
    let w = |s: &str| parse_weight(s).expect("an exact rational");
    let weights = Weights::<Original>::from_dimacs_pairs(
        &[(1, w("3/4")), (-1, w("-1/2")), (2, w("9.5367431640625E-7"))],
        2,
    );
    let rows = weights.to_record_rows();
    let header = DimacsHeader {
        track: Some("wmc"),
        show: None,
        weights: Some(&rows),
    };

    let (_, meta) = write_then_read("dimacs-weights", &formula, &header);

    assert_eq!(
        meta.weights
            .expect("the writer emitted `c p weight` lines")
            .resolve::<Original>(2),
        weights,
        "an exact weight changed through write→read",
    );
}

/// The `c t` line names the track, and every token the crate can write is one
/// the reader accepts back.
#[test]
fn a_written_track_header_comes_back_as_the_mode_it_names() {
    let formula = parse("p cnf 2 1\n1 2 0\n").0;
    for mode in [Mode::Mc, Mode::Wmc, Mode::Pmc, Mode::Pwmc] {
        let header: DimacsHeader<'_, Original> = DimacsHeader {
            track: Some(mode.token()),
            show: None,
            weights: None,
        };
        let (_, meta) = write_then_read("dimacs-track", &formula, &header);
        assert_eq!(
            meta.declared_track(),
            Some(mode),
            "the `c t` line written for {mode:?} read back as something else",
        );
    }
}

/// A formula declaring nothing is still a formula: the header alone is a
/// complete file, and reading it back gives the same empty variable space rather
/// than a missing-problem-line failure.
#[test]
fn a_formula_with_no_variables_and_no_clauses_survives_the_round_trip() {
    let empty = CnfFormula {
        num_vars: 0,
        clauses: Vec::new(),
    };
    let header: DimacsHeader<'_, Original> = DimacsHeader::default();

    let (again, meta) = write_then_read("dimacs-empty", &empty, &header);

    assert_eq!(again, empty, "an empty formula changed through write→read");
    assert!(
        meta.declared_show_vars().is_none(),
        "a bare header declares no show set",
    );
}

/// Every id the writer emits sits inside the count it writes in the header —
/// including the widest one, which the reader's range rule accepts only because
/// the bound is inclusive. A clause literal, a show variable and a weight
/// literal all sit on it at once.
#[test]
fn the_widest_id_the_header_declares_survives_the_round_trip() {
    let formula = parse("p cnf 5 2\n-1 5 0\n-5 0\n").0;
    let show = ShowSet::<Original>::from_dimacs_ids(&[5]).expect("1-based ids");
    let rows = vec![LiteralWeight {
        literal: -5,
        weight: "1/3".to_string(),
    }];
    let header = DimacsHeader {
        track: Some("pwmc"),
        show: Some(&show),
        weights: Some(&rows),
    };

    let (again, meta) = write_then_read("dimacs-widest-id", &formula, &header);

    assert_eq!(again, formula, "the clause set changed through write→read");
    assert_eq!(meta.declared_show_vars(), Some(&show));
    assert_eq!(
        meta.weights
            .expect("the written weight line must parse back")
            .to_literal_pairs(),
        vec![(-5, parse_weight("1/3").expect("an exact rational"))],
    );
}

/// The reader normalizes on the way in — a repeated literal collapses, a
/// tautology is dropped — and what the writer emits from the result needs no
/// second normalization to read back as the same clause set.
#[test]
fn a_clause_set_the_reader_normalized_is_written_back_unchanged() {
    let formula = parse("p cnf 3 3\n1 1 -2 0\n3 -3 1 0\n2 3 0\n").0;
    assert_eq!(
        formula.clauses.len(),
        2,
        "the fixture must exercise both normalizations: {:?}",
        formula.clauses,
    );
    let header: DimacsHeader<'_, Original> = DimacsHeader::default();

    let (again, _) = write_then_read("dimacs-normalized", &formula, &header);

    assert_eq!(
        again, formula,
        "a normalized clause set changed through write→read",
    );
}

/// The in-memory writer, which a caller reaches without a file: what it writes
/// parses back to the formula it was given.
mod in_memory {
    use super::*;

    fn written(formula: &CnfFormula) -> String {
        let mut out = Vec::new();
        formula
            .write_dimacs(&mut out)
            .expect("a `Vec` accepts every byte offered to it");
        String::from_utf8(out).expect("DIMACS is ASCII")
    }

    /// The fixed point, as everywhere else in this module: the reader
    /// normalizes a clause on the way in — sorted by variable, deduplicated —
    /// so what is worth pinning is that a formula the reader produced writes
    /// and reads back unchanged.
    #[test]
    fn a_formula_written_in_memory_parses_back_to_itself() {
        let formula = parse(&written(&crate::tests::circuit_fixture::multiplier())).0;
        assert_eq!(parse(&written(&formula)).0, formula);
        assert!(
            formula.clauses.len() > 100,
            "the fixture is what makes this more than a two-clause round trip",
        );
    }

    /// A universe wider than the clauses is a different formula — it has models
    /// the narrower one does not — so the header line has to carry it.
    #[test]
    fn a_universe_wider_than_the_clauses_survives_the_round_trip() {
        let formula = CnfFormula {
            num_vars: 40,
            clauses: vec![Clause::new(vec![
                Literal::pos(VarId::from_dimacs(1)),
                Literal::neg(VarId::from_dimacs(2)),
            ])],
        };
        let text = written(&formula);
        assert!(
            text.starts_with("p cnf 40 1\n"),
            "the declared universe is what the header states, got: {text}",
        );
        assert_eq!(parse(&text).0, formula);
    }

    /// The body alone, for a caller writing its own preamble above it.
    #[test]
    fn the_clause_body_can_be_written_without_the_header_line() {
        let formula = crate::tests::circuit_fixture::multiplier();
        let mut body = Vec::new();
        formula
            .write_dimacs_clauses(&mut body)
            .expect("a `Vec` accepts every byte offered to it");
        let body = String::from_utf8(body).expect("DIMACS is ASCII");
        assert!(
            !body.contains("p cnf"),
            "the body carries no problem line of its own",
        );
        assert_eq!(
            written(&formula),
            format!(
                "p cnf {} {}\n{body}",
                formula.num_vars,
                formula.clauses.len()
            ),
            "the whole file is the problem line and this same body",
        );
    }
}
