//! PACE `.td` text: what a decomposer's output parses to, and what a rejection names

use super::*;

#[test]
fn test_parse_pace_td_small() {
    // Hand-crafted .td: 2 bags, 1 tree edge
    // Bag 1: vertices 1,2 → stored as 0,1
    // Bag 2: vertices 2,3 → stored as 1,2
    // Tree edge: 1-2
    let td_str = "s td 2 2 3\nb 1 1 2\nb 2 2 3\n1 2\n";
    let td = parse_pace_td(td_str, GraphKind::Primal, 3).expect("Should parse");
    assert_eq!(td.bags.len(), 2);

    assert_eq!(td.bags[0].id, 0);
    assert_eq!(td.bags[0].vertices, vec![0, 1]);
    assert_eq!(td.bags[1].id, 1);
    assert_eq!(td.bags[1].vertices, vec![1, 2]);

    assert!(td.adj[0].contains(&1), "Bag 0 should be adjacent to bag 1");
    assert!(td.adj[1].contains(&0), "Bag 1 should be adjacent to bag 0");
}

#[test]
fn test_parse_pace_td_with_comments() {
    let td_str = "c comment line\ns td 1 2 2\nb 1 1 2\n";
    let td = parse_pace_td(td_str, GraphKind::Primal, 2).expect("Should parse with comments");
    assert_eq!(td.bags.len(), 1);
    assert_eq!(td.bags[0].vertices, vec![0, 1]);
}

#[test]
fn test_parse_pace_td_empty() {
    let result = parse_pace_td("", GraphKind::Primal, 3);
    assert!(result.is_err(), "empty TD should error");
}

#[test]
fn test_parse_pace_td_no_bags() {
    let result = parse_pace_td("s td 2 2 3\n", GraphKind::Primal, 3);
    assert!(result.is_err(), "TD with header but no bags should error");
}

#[test]
fn test_parse_pace_td_blank_lines_only() {
    let result = parse_pace_td("\n\n\n", GraphKind::Primal, 3);
    assert!(result.is_err(), "blank lines should give no bags");
}

/// Malformed output is rejected, never a panic — and the rejection names the id
/// it read and the count that id had to fit inside, which is the pair a reader
/// of the file needs to find the line at fault. Bag and vertex ids are written
/// 1-based, so `0` is not an id and the solution line's counts bound the rest.
///
/// The two cases whose expectation is empty are rejected by the number parser,
/// whose wording belongs to it.
#[test]
fn a_rejected_td_names_the_offending_id_and_the_count_it_was_checked_against() {
    let cases: &[(&str, &str, &[&str])] = &[
        (
            "s td 2 2 3\nb 0 1 2\nb 2 2 3\n",
            "bag id 0",
            &["bag id 0", "1-based"],
        ),
        (
            "s td 1 2 2\nb 1 0 2\n",
            "vertex 0",
            &["vertex 0", "1-based"],
        ),
        (
            "s td 2 2 3\nb 1 1 2\nb 2 2 3\n0 2\n",
            "bag id 0 on a tree edge",
            &["bag id 0", "1-based"],
        ),
        (
            "s td 1 2 2\nb 5 1 2\n",
            "a bag id past the declared bag count",
            &["bag id 5", "declares 1"],
        ),
        (
            "s td 1 2 2\nb 1 1 9\n",
            "a vertex past the declared vertex count",
            &["vertex 9", "declares 2"],
        ),
        (
            "s td 2 2 3\nb 1 1 2\nb 2 2 3\n1 9\n",
            "a tree edge past the declared bag count",
            &["bag id 9", "declares 2"],
        ),
        (
            "b 1 1 2\n",
            "a bag line before the solution line",
            &["before the solution line"],
        ),
        ("s td x 2 3\nb 1 1 2\n", "a non-numeric bag count", &[]),
        (
            "s td 1\nb 1 1 2\n",
            "a truncated solution line",
            &["Malformed solution line"],
        ),
        ("s td 1 2 2\nb 1 1 x\n", "a non-numeric vertex", &[]),
    ];
    for &(td_str, what, expected) in cases {
        let err = parse_pace_td(td_str, GraphKind::Primal, 3)
            .map(|_| ())
            .expect_err(what)
            .to_string();
        for want in expected {
            assert!(
                err.contains(want),
                "{what}: the message must name {want:?}, got: {err}",
            );
        }
    }
}
