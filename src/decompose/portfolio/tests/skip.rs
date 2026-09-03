//! `VITRI_PORTFOLIO_SKIP`: parsing the variable, and the catalog it shrinks.

use crate::decompose::portfolio::driver::{catalog, catalog_with_knobs};
use crate::decompose::portfolio::parse_skip_names;

#[test]
fn names_parse_in_writing_order_without_repeats() {
    let names = parse_skip_names(" force ; hypergraph-bisect ;; force ").expect("both are entries");
    assert_eq!(names, vec!["force", "hypergraph-bisect"]);
}

#[test]
fn a_name_the_catalog_does_not_have_is_refused() {
    let err = parse_skip_names("force;minfill-incidence").expect_err("not a built-in entry");
    let text = err.to_string();
    assert!(text.contains("VITRI_PORTFOLIO_SKIP"), "{text}");
    assert!(text.contains("minfill-incidence"), "{text}");
}

#[test]
fn skipping_every_entry_is_refused() {
    let all: Vec<&str> = catalog().iter().map(|c| c.name).collect();
    let err = parse_skip_names(Box::leak(all.join(";").into_boxed_str()))
        .expect_err("nothing left to build");
    assert!(err.to_string().contains("nothing to build"), "{err}");
}

#[test]
fn skipped_entries_leave_the_catalog_and_the_rest_keep_their_order() {
    let full: Vec<&str> = catalog().iter().map(|c| c.name).collect();
    let kept: Vec<&str> = catalog_with_knobs(&["force", "hypergraph-bisect"])
        .iter()
        .map(|c| c.name)
        .collect();
    let expected: Vec<&str> = full
        .iter()
        .copied()
        .filter(|n| *n != "force" && *n != "hypergraph-bisect")
        .collect();
    assert_eq!(kept, expected);
    assert_eq!(kept.len(), full.len() - 2);
}
