//! The competition meta-comments: weights, show sets, and the track header.
//!
//! These lines decide which counting problem a file poses, so the reader
//! has to carry them exactly: rational weights in every spelling they
//! appear in, show sets accumulated across lines and one-based on disk,
//! and a track header that does not silently override what the file says.

use super::*;
use crate::error::VitriError;

/// PMC-format CNF files have `w <var> <weight>` lines that must be skipped.
/// Before the fix, weight lines were parsed as clause data, corrupting the formula.
#[test]
fn test_parse_skips_pmc_weight_lines() {
    // Simulates a PMC file: p cnf header, weight lines, then actual clauses.
    let input = b"p cnf 3 2\nw\t1\t0.5\nw\t2\t-1\nw\t3\t0.1\n1 -2 0\n2 3 0\n";
    let formula = CnfFormula::from_dimacs(&input[..]).unwrap().0;
    assert_eq!(formula.num_vars, 3);
    assert_eq!(
        formula.clauses.len(),
        2,
        "weight lines should be skipped, not parsed as clauses"
    );
    // First clause should be exactly [1, -2], not corrupted by weight data.
    assert_eq!(formula.clauses[0].literals.len(), 2);
    assert_eq!(formula.clauses[0].literals[0], Literal::pos(VarId(0)));
    assert_eq!(formula.clauses[0].literals[1], Literal::neg(VarId(1)));
}

#[test]
fn test_parse_weight_lines_with_integer_values() {
    // w 1 -1 has an integer weight that would parse as literal -1 if not skipped.
    let input = b"p cnf 2 1\nw\t1\t-1\nw\t2\t-1\n1 2 0\n";
    let formula = CnfFormula::from_dimacs(&input[..]).unwrap().0;
    assert_eq!(formula.clauses.len(), 1);
    assert_eq!(
        formula.clauses[0].literals.len(),
        2,
        "weight -1 values must not become literals"
    );
}

/// Scientific-notation weights (`9.5367431640625E-7`, `1.5e3`) appear throughout
/// the MC 2026 weighted tracks. They must parse exactly as `mantissa × 10^exp`,
/// not be rejected ("invalid decimal weight") nor lose precision via float.
#[test]
fn test_parse_weight_scientific_notation() {
    use num_bigint::BigInt;
    use num_rational::BigRational;

    let r = |a: i64, b: i64| BigRational::new(BigInt::from(a), BigInt::from(b));

    // 9.5367431640625E-7 = 95367431640625 / 10^13/10^6 ... compute exactly:
    // mantissa 9.5367431640625 = 95367431640625 / 10^13, × 10^-7 → /10^20.
    assert_eq!(
        parse_weight("9.5367431640625E-7").unwrap(),
        BigRational::new(BigInt::from(95367431640625i64), BigInt::from(10i64).pow(20)),
    );
    // 7.888609052210118E-31 — large negative exponent, must not reject.
    let v = parse_weight("7.888609052210118E-31").unwrap();
    assert!(v > BigRational::from_integer(BigInt::from(0)));
    // Positive exponent, lowercase e: 1.5e3 = 1500.
    assert_eq!(parse_weight("1.5e3").unwrap(), r(1500, 1));
    // Integer mantissa with exponent: 5E-2 = 1/20.
    assert_eq!(parse_weight("5E-2").unwrap(), r(1, 20));
    // Exponent zero: 3.25E0 = 13/4.
    assert_eq!(parse_weight("3.25E0").unwrap(), r(13, 4));
    // Negative mantissa: -2.5e1 = -25.
    assert_eq!(parse_weight("-2.5e1").unwrap(), r(-25, 1));
    // Plain forms still work.
    assert_eq!(parse_weight("0.5").unwrap(), r(1, 2));
    assert_eq!(parse_weight("3/4").unwrap(), r(3, 4));
    assert_eq!(parse_weight("7").unwrap(), r(7, 1));
    // Malformed exponent rejected, not silently mis-parsed.
    assert!(parse_weight("1.5eX").is_err());
}

#[test]
fn test_parse_mcc_weighted_meta() {
    let input =
        b"c t wmc\np cnf 2 1\nc p weight 1 0.7 0\nc p weight -1 0.3 0\nc p weight 2 1/4 0\n1 2 0\n";
    let (formula, meta) = CnfFormula::from_dimacs(&input[..]).unwrap();
    assert_eq!(formula.num_vars, 2);
    assert_eq!(formula.clauses.len(), 1);
    assert_eq!(meta.mode(), Mode::Wmc);
    let wt = meta.weights.expect("weights parsed");
    let resolved: Weights<Original> = wt.resolve(2); // (w_neg, w_pos) per var
    let r = |n: i64, d: i64| {
        num_rational::BigRational::new(num_bigint::BigInt::from(n), num_bigint::BigInt::from(d))
    };
    assert_eq!(resolved[VarId(0)], (r(3, 10), r(7, 10))); // var0: neg .3, pos .7
    assert_eq!(resolved[VarId(1)], (r(1, 1), r(1, 4))); // var1: neg unspecified→1, pos 1/4
}

#[test]
fn test_parse_show_set() {
    let input = b"c t pmc\np cnf 4 1\nc p show 1 3 0\n1 -2 3 0\n";
    let (_f, meta) = CnfFormula::from_dimacs(&input[..]).unwrap();
    assert_eq!(meta.mode(), Mode::Pmc);
    assert_eq!(
        meta.declared_show_vars(),
        Some(&ShowSet::from_zero_based([0, 2]))
    );
}

#[test]
fn test_plain_mc_meta_default() {
    let input = b"p cnf 2 1\n1 2 0\n";
    let (_f, meta) = CnfFormula::from_dimacs(&input[..]).unwrap();
    assert_eq!(meta.mode(), Mode::Mc);
    assert!(meta.declared_show_vars().is_none());
    assert!(meta.weights.is_none());
}

/// A `c t` line is what makes a file DECLARE a track, and `c t mc` declares one
/// as much as `c t pmc` does. `mode` cannot say so — it reads `Mc` for a file
/// with no such line too — which is why the declaration is reported separately.
#[test]
fn declared_track_reports_the_line_not_the_resolved_mode() {
    let (_, projected) =
        CnfFormula::from_dimacs(&b"c t pmc\np cnf 6 1\nc p show 2 3 0\n1 -2 0\n"[..])
            .expect("must parse");
    assert_eq!(projected.declared_track(), Some(Mode::Pmc));
    assert_eq!(projected.mode(), Mode::Pmc);

    let (_, plain) =
        CnfFormula::from_dimacs(&b"c t mc\np cnf 2 1\n1 2 0\n"[..]).expect("must parse");
    assert_eq!(
        plain.declared_track(),
        Some(Mode::Mc),
        "`c t mc` names a track, so the file declares one",
    );
    assert_eq!(plain.mode(), Mode::Mc);

    let (_, bare) =
        CnfFormula::from_dimacs(&b"c comment\np cnf 3 2\n1 -2 0\n2 3 0\n"[..]).expect("must parse");
    assert_eq!(
        bare.declared_track(),
        None,
        "no `c t` line means no declaration, however `mode` resolves",
    );
    assert_eq!(
        bare.mode(),
        Mode::Mc,
        "an absent track still reads as plain model counting",
    );
}

#[test]
fn test_parse_c_p_show_accumulates_and_dedups() {
    let input = b"p cnf 4 1\nc p show 3 1 0\nc p show 1 2 0\n1 2 3 4 0\n";
    let (_f, meta) = CnfFormula::from_dimacs(&input[..]).unwrap();
    // 0-based: {3,1} ∪ {1,2} = {2,0} ∪ {0,1} → sorted dedup {0,1,2}.
    assert_eq!(
        meta.declared_show_vars(),
        Some(&ShowSet::from_zero_based([0, 1, 2]))
    );
}

/// A `c p show` line is what makes a file DECLARE a show set, and an empty one
/// still declares it.
#[test]
fn declared_show_vars_reports_the_line_not_the_track() {
    let (_, m) =
        CnfFormula::from_dimacs(std::io::Cursor::new("p cnf 3 1\nc p show 1 3 0\n1 2 0\n"))
            .expect("must parse");
    assert_eq!(
        m.declared_show_vars(),
        Some(&ShowSet::from_zero_based([0, 2]))
    );

    let (_, empty) =
        CnfFormula::from_dimacs(std::io::Cursor::new("p cnf 3 1\nc p show 0\n1 2 0\n"))
            .expect("must parse");
    assert_eq!(
        empty.declared_show_vars(),
        Some(&ShowSet::empty()),
        "`c p show 0` projects onto nothing — that is a declaration, not its absence",
    );

    let (_, none) =
        CnfFormula::from_dimacs(std::io::Cursor::new("p cnf 3 1\n1 2 0\n")).expect("must parse");
    assert_eq!(none.declared_show_vars(), None);
}

/// `0` names no variable, so a weight line carrying one is a malformed file and
/// is refused where it is read.
#[test]
fn test_parse_rejects_a_weight_on_literal_zero() {
    let input = b"c t wmc\np cnf 2 1\nc p weight 0 0.5 0\n1 2 0\n";
    let err = CnfFormula::from_dimacs(&input[..]).expect_err("0 names no variable");
    assert!(err.to_string().contains("literal 0"), "{err}");
}

/// The fraction spelling of an exact weight. The sign may sit on either side of
/// the slash, and whitespace around each half is not part of the value — the one
/// rule for writing a weight, applied to the one spelling a written table uses.
#[test]
fn a_weight_written_as_a_fraction_parses_to_that_exact_rational() {
    use num_bigint::BigInt;
    use num_rational::BigRational;
    let r = |a: i64, b: i64| BigRational::new(BigInt::from(a), BigInt::from(b));

    assert_eq!(parse_weight("3/4").expect("a fraction"), r(3, 4));
    assert_eq!(
        parse_weight("-1/2").expect("a negative numerator"),
        r(-1, 2)
    );
    assert_eq!(
        parse_weight("1/-2").expect("a negative denominator"),
        r(-1, 2),
        "a rational keeps a positive denominator, so both spellings are one value",
    );
    assert_eq!(
        parse_weight(" 6 / 8 ").expect("whitespace around each half"),
        r(3, 4),
        "the value is in lowest terms, whatever the file wrote",
    );
}

/// A denominator of `0` names no rational, so the line is refused where it is
/// read rather than becoming an unusable weight downstream.
#[test]
fn a_weight_with_a_zero_denominator_is_refused_by_name() {
    let err = CnfFormula::from_dimacs(std::io::Cursor::new(
        "c t wmc\np cnf 2 1\nc p weight 1 3/0 0\n1 2 0\n",
    ))
    .expect_err("a zero denominator names no rational");
    assert!(
        matches!(err, crate::error::VitriError::Input { .. }),
        "a malformed file is an input failure, got {err:?}",
    );
    let text = err.to_string();
    assert!(
        text.contains("zero denominator"),
        "{text:?} must say what is wrong",
    );
    assert!(
        text.contains("3/0"),
        "{text:?} must quote the weight the line wrote",
    );
    assert!(text.starts_with("line 3"), "{text:?} must name the line");
}

/// A `c p show` line names variables, never literals, so a negative id is a
/// malformed file — `0` ends the line and everything before it is a variable.
#[test]
fn a_negative_show_variable_is_refused_by_name() {
    let err = CnfFormula::from_dimacs(std::io::Cursor::new(
        "c t pmc\np cnf 3 1\nc p show 1 -2 0\n1 2 0\n",
    ))
    .expect_err("a show set names variables, not literals");
    assert!(
        matches!(err, crate::error::VitriError::Input { .. }),
        "a malformed file is an input failure, got {err:?}",
    );
    let text = err.to_string();
    assert!(
        text.contains("-2"),
        "{text:?} must quote the id the line wrote",
    );
    assert!(text.starts_with("line 3"), "{text:?} must name the line");
}

/// The public weight parser is the one a file's `c p weight` lines go through,
/// so a caller reading weights from its own configuration and this crate
/// reading them from DIMACS cannot disagree about what a token means.
#[test]
fn the_public_weight_parser_reads_every_spelling_a_file_may_carry() {
    for token in [
        "3/4", "-1/2", "1/-2", " 6 / 8 ", "0.5", "1.5e3", "5E-2", "7",
    ] {
        assert_eq!(
            parse_rational_weight(token).ok(),
            parse_weight(token).ok(),
            "{token} reads differently through the two entry points",
        );
    }
}

/// A token that names no rational comes back as this crate's own error rather
/// than as a string a caller would have to classify itself.
#[test]
fn a_weight_token_that_names_no_rational_is_refused_by_name() {
    let err = parse_rational_weight("1/0").expect_err("a zero denominator names no rational");
    assert!(
        matches!(err, VitriError::Input { .. }),
        "a weight token is input data, got: {err:?}",
    );
    assert!(
        err.to_string().contains("1/0"),
        "the offending token must appear, got: {err}",
    );
}
