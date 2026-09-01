//! The two edge shapes: refuted, and resolved outright

use super::*;

/// A refuted instance is written as an explicit `x ∧ ¬x` over the ORIGINAL
/// variable count, with the identity map — the doc's literal claim. DIMACS has
/// no spelling for the empty clause, so the alternative would re-parse as a
/// nonzero count.
///
/// The identity is asserted as the identity, not merely as "some injective
/// map": any permutation would satisfy a consistency check, and a consumer
/// reading the record takes the claim at its word.
#[test]
fn a_refuted_instance_is_an_explicit_contradiction_under_the_identity_map() {
    let t = Scratch::new("unsat");
    let input = t.file("un.cnf", REFUTED_WEIGHTED);
    let out = t.out("bundle");
    run(&[s(&input), "-o", s(&out)]).exit(0);

    let record = json(&out.join(PREPROCESS_RECORD_NAME));
    assert_eq!(record["unsat"], true);
    let original_num_vars = record["original_num_vars"].as_u64().expect("var count");
    let map: Vec<i64> = record["reduced_to_original_dimacs"]
        .as_array()
        .expect("the map")
        .iter()
        .map(|v| v.as_i64().expect("a signed id"))
        .collect();
    assert_eq!(
        map,
        (1..=original_num_vars as i64).collect::<Vec<_>>(),
        "a refutation keeps the original numbering",
    );

    let cnf = read(&out.join(REDUCED_CNF_NAME));
    let clauses: Vec<Vec<i32>> = cnf
        .lines()
        .filter(|l| !l.starts_with('c') && !l.starts_with('p') && !l.trim().is_empty())
        .map(|l| {
            l.split_whitespace()
                .map(|t| t.parse::<i32>().expect("a literal"))
                .take_while(|t| *t != 0)
                .collect()
        })
        .collect();
    assert_eq!(clauses.len(), 2, "an explicit contradiction is two clauses");
    assert_eq!(clauses[0].len(), 1);
    assert_eq!(clauses[1], vec![-clauses[0][0]]);
    assert!(
        cnf.starts_with(&format!("p cnf {original_num_vars} 2\n")),
        "over the original variable count:\n{cnf}",
    );
}

/// A refutation is the other outcome preprocessing can reach on its own: the
/// count is 0, so there is nothing left to compile. The contradiction is
/// exported because the record has to point at a formula, but no vtree is built
/// over it — the bundle is the two files that carry the verdict, and the summary
/// says which verdict it is.
#[test]
fn a_refuted_instance_writes_two_files_and_no_vtree() {
    let t = Scratch::new("unsatfiles");
    let input = t.file("un.cnf", REFUTED_WEIGHTED);
    let out = t.out("bundle");
    let r = run(&[s(&input), "-o", s(&out)]).exit(0);

    assert_eq!(
        entries(&out),
        set(&[REDUCED_CNF_NAME, PREPROCESS_RECORD_NAME])
    );
    r.assert_stdout("unsat:        preprocessing refuted the instance; count(original) = 0");
    r.assert_stdout("vtree:        none (the count is already 0)");
}

/// Preprocessing can settle the instance outright. Then there is nothing to
/// build a vtree over, and emitting a degenerate one would be a lie about what
/// was computed — so the bundle is the two files that still mean something,
/// and the summary says so.
#[test]
fn a_fully_resolved_instance_writes_two_files_and_no_vtree() {
    let t = Scratch::new("resolved");
    let input = t.file("fr.cnf", FULLY_RESOLVED);
    let out = t.out("bundle");
    let r = run(&[s(&input), "-o", s(&out)]).exit(0);

    assert_eq!(
        entries(&out),
        set(&[REDUCED_CNF_NAME, PREPROCESS_RECORD_NAME])
    );
    assert!(read(&out.join(REDUCED_CNF_NAME)).starts_with("p cnf 0 0\n"));
    r.assert_stdout("reduced:      0 vars — fully resolved");
    r.assert_stdout("vtree:        none (no variables to build one over)");
    // The lift as the one factor the mode uses: unweighted, so the power of
    // two alone. One variable is free and two are forced, and only the free one
    // doubles the count.
    r.assert_stdout("count(original) = 2^1");
}

/// `--dot` is never refused as inert, because every mode emits at least one
/// vtree when there is anything to build one over. On the one instance where
/// there is not, the flag is an ordinary request that produces no picture —
/// the resolved answer comes first, and a picture of nothing is not written.
#[test]
fn the_picture_flag_on_a_fully_resolved_instance_exits_zero_and_writes_no_picture() {
    let t = Scratch::new("resolveddot");
    let input = t.file("fr.cnf", FULLY_RESOLVED);
    let out = t.out("bundle");
    run(&[s(&input), "-o", s(&out), "--dot"]).exit(0);
    assert_eq!(
        entries(&out),
        set(&[REDUCED_CNF_NAME, PREPROCESS_RECORD_NAME]),
        "there is no vtree to picture, so asking for one adds no file",
    );
}
