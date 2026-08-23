//! The declared variable count is a bound, and it is enforced.
//!
//! Every id the file mentions — in a clause, in a meta-line above the
//! problem line, anywhere — is checked against it, so a file that
//! under-declares is refused rather than silently read into a formula
//! whose `num_vars` does not cover its own literals.

use super::*;

use crate::tests::common::{CLAUSE_ID_ABOVE_COUNT, SHOW_ID_ABOVE_COUNT};

/// An id above the `p cnf` count comes back as an `Err`, from every construct
/// that carries one. An embedder has to get a value it can report: before, a
/// clause literal above the count panicked inside preprocessing's per-variable
/// tables, and a show variable above it reached the vendored sampling-set
/// assertion and ended the process.
#[test]
fn ids_above_the_declared_variable_count_are_rejected() {
    let cases: [(&str, &str); 4] = [
        (CLAUSE_ID_ABOVE_COUNT, "clause literal 5"),
        ("p cnf 2 1\n1 -5 0\n", "clause literal -5"),
        (SHOW_ID_ABOVE_COUNT, "show var 9"),
        (
            "c t wmc\np cnf 2 1\nc p weight -9 1/3 0\n1 2 0\n",
            "weight literal -9",
        ),
    ];
    for (text, named) in cases {
        let err = CnfFormula::from_dimacs(std::io::Cursor::new(text))
            .expect_err("an id above the declared count is a parse failure, not a panic")
            .to_string();
        assert!(err.contains(named), "{err:?} must name {named:?}");
        assert!(
            err.contains("declared variable count 2"),
            "{err:?} must name the count it exceeded",
        );
        assert!(err.starts_with("line "), "{err:?} must name the line");
    }
}

/// ...and the id EQUAL to the declared count is inside it. The rule is
/// `1 <= |id| <= num_vars` on every construct, so no well-formed file can be
/// refused by it.
#[test]
fn the_declared_count_itself_is_in_range() {
    let (formula, meta) = CnfFormula::from_dimacs(std::io::Cursor::new(
        "c t pwmc\np cnf 2 1\nc p show 2 0\nc p weight -2 1/3 0\n1 2 0\n",
    ))
    .expect("every id sits exactly on the boundary");
    assert_eq!(formula.num_vars, 2);
    assert_eq!(
        meta.declared_show_vars(),
        Some(&ShowSet::from_zero_based([1]))
    );
    assert_eq!(
        meta.weights.expect("weights parsed").to_literal_pairs(),
        vec![(-2, num_rational::BigRational::new(1.into(), 3.into()))]
    );
}

/// The check is made once the whole file has been read, which is what lets it
/// cover the real MCC shape that writes `c t` and `c p show` ABOVE the `p cnf`
/// line: a file whose ids fit is still accepted even though the count was
/// unknown when they were read, and one whose ids do not is still rejected.
#[test]
fn meta_lines_above_the_problem_line_are_checked_against_it() {
    let (formula, meta) = CnfFormula::from_dimacs(std::io::Cursor::new(
        "c t pmc\nc p show 1 3 0\np cnf 3 1\n1 2 0\n",
    ))
    .expect("a show set written above the header is still a show set");
    assert_eq!(formula.num_vars, 3);
    assert_eq!(
        meta.declared_show_vars(),
        Some(&ShowSet::from_zero_based([0, 2]))
    );

    let err = CnfFormula::from_dimacs(std::io::Cursor::new(
        "c t pmc\nc p show 1 9 0\np cnf 3 1\n1 2 0\n",
    ))
    .expect_err("9 is above the count the header goes on to declare")
    .to_string();
    assert!(err.contains("line 2: show var 9"), "{err:?}");
}

/// A `w <var> <weight>` line is discarded whole, so the variable it names never
/// enters the formula or the metadata and carries no id to range-check. A file
/// with one stays readable exactly as it was.
#[test]
fn a_skipped_w_line_carries_no_id_to_check() {
    let input = b"p cnf 2 1\nw\t99\t0.5\n1 2 0\n";
    let formula = CnfFormula::from_dimacs(&input[..])
        .expect("a `w` line is not clause data")
        .0;
    assert_eq!(formula.num_vars, 2);
    assert_eq!(formula.clauses.len(), 1);
}
