//! The on-disk vtree format: what is written, and that reading it back
//! returns the same tree.
//!
//! Variable ids are one-based on disk and zero-based in memory, and nodes
//! are written children-first — two conventions a reader of the file has
//! to be able to rely on.

use super::*;

use crate::tests::common::tokenize_vtree_text;

/// The variable set under every node, sorted — what a vtree file says the tree
/// IS, independent of the node ids it happened to number them with. Reads the
/// lines in file order, which the format guarantees lists a child before its
/// parent.
fn var_sets(node_lines: &[Vec<String>]) -> Vec<Vec<u32>> {
    let mut under: Vec<Vec<u32>> = vec![Vec::new(); node_lines.len()];
    for toks in node_lines {
        let id: usize = toks[1].parse().unwrap();
        under[id] = if toks[0] == "L" {
            vec![toks[2].parse().unwrap()]
        } else {
            let (l, r): (usize, usize) = (toks[2].parse().unwrap(), toks[3].parse().unwrap());
            let mut s = under[l].clone();
            s.extend_from_slice(&under[r]);
            s.sort_unstable();
            s
        };
    }
    under.sort();
    under
}

#[test]
fn test_vtree_format_header_node_count() {
    for num_vars in [1u32, 2, 3, 4, 5, 8, 16] {
        let vtree = Vtree::balanced(num_vars);
        let fmt = vtree.to_vtree_text();
        let (n, node_lines) = tokenize_vtree_text(&fmt);
        let expected = 2 * num_vars as usize - 1;
        assert_eq!(n, expected, "num_vars={}", num_vars);
        assert_eq!(node_lines.len(), n, "num_vars={}", num_vars);
    }
}

#[test]
fn test_vtree_format_vars_one_indexed() {
    let num_vars = 5u32;
    let vtree = Vtree::balanced(num_vars);
    let fmt = vtree.to_vtree_text();
    let (_, node_lines) = tokenize_vtree_text(&fmt);
    let mut var_ids: Vec<u32> = node_lines
        .iter()
        .filter(|toks| toks[0] == "L")
        .map(|toks| toks[2].parse().unwrap())
        .collect();
    var_ids.sort();
    let expected: Vec<u32> = (1..=num_vars).collect();
    assert_eq!(var_ids, expected);
}

#[test]
fn test_vtree_format_children_before_parents() {
    for vtree in [Vtree::balanced(6), Vtree::linear(6), Vtree::random(6, 42)] {
        let fmt = vtree.to_vtree_text();
        let (_, node_lines) = tokenize_vtree_text(&fmt);
        for toks in &node_lines {
            if toks[0] == "I" {
                let id: usize = toks[1].parse().unwrap();
                let left: usize = toks[2].parse().unwrap();
                let right: usize = toks[3].parse().unwrap();
                assert!(left < id, "left {} >= parent {}", left, id);
                assert!(right < id, "right {} >= parent {}", right, id);
            }
        }
    }
}

#[test]
fn test_vtree_format_node_types() {
    // Every line is either "L id var" (3 tokens) or "I id left right" (4 tokens).
    let vtree = Vtree::balanced(4);
    let fmt = vtree.to_vtree_text();
    let (n, node_lines) = tokenize_vtree_text(&fmt);
    assert_eq!(node_lines.len(), n);
    let mut leaf_count = 0usize;
    let mut internal_count = 0usize;
    for toks in &node_lines {
        match toks[0].as_str() {
            "L" => {
                assert_eq!(toks.len(), 3, "leaf line should have 3 tokens");
                leaf_count += 1;
            }
            "I" => {
                assert_eq!(toks.len(), 4, "internal line should have 4 tokens");
                internal_count += 1;
            }
            other => panic!("unexpected node type: {}", other),
        }
    }
    assert_eq!(leaf_count, 4);
    assert_eq!(internal_count, 3);
}

/// A rotation relinks nodes without moving them in the node array, so the
/// array order stops being topological while the format still requires a child
/// to be listed before its parent. Writing a rotated tree has to follow the
/// bottom-up order, which is the one that survives.
#[test]
fn test_vtree_format_children_before_parents_after_rotation() {
    let mut vtree = Vtree::balanced(4);
    let root = vtree.root();
    rotate::rotate_right(&mut vtree, root).expect("right rotation applies at the root");

    let fmt = vtree.to_vtree_text();
    let (_, node_lines) = tokenize_vtree_text(&fmt);
    for toks in &node_lines {
        if toks[0] == "I" {
            let id: usize = toks[1].parse().unwrap();
            let left: usize = toks[2].parse().unwrap();
            let right: usize = toks[3].parse().unwrap();
            assert!(left < id, "left {} >= parent {}", left, id);
            assert!(right < id, "right {} >= parent {}", right, id);
        }
    }

    // And it is still the same tree. The reader numbers the nodes it reads in
    // its own order, so the file it would write back is not the file it read —
    // what has to survive is the shape and the leaves.
    let loaded = Vtree::from_vtree_text(&fmt).expect("roundtrip parse failed");
    let (_, reloaded_lines) = tokenize_vtree_text(&loaded.to_vtree_text());
    assert_eq!(var_sets(&reloaded_lines), var_sets(&node_lines));
}

#[test]
fn test_vtree_format_all_vtree_types() {
    let num_vars = 7u32;
    for vtree in [
        Vtree::balanced(num_vars),
        Vtree::linear(num_vars),
        Vtree::random(num_vars, 42),
    ] {
        let fmt = vtree.to_vtree_text();
        let (n, node_lines) = tokenize_vtree_text(&fmt);
        assert_eq!(n, 2 * num_vars as usize - 1);
        assert_eq!(node_lines.len(), n);

        let ids: Vec<usize> = node_lines
            .iter()
            .map(|toks| toks[1].parse::<usize>().unwrap())
            .collect();
        assert_eq!(ids, (0..n).collect::<Vec<_>>());
    }
}

#[test]
fn test_vtree_format_single_var() {
    let vtree = Vtree::balanced(1);
    let fmt = vtree.to_vtree_text();
    let (n, node_lines) = tokenize_vtree_text(&fmt);
    assert_eq!(n, 1);
    assert_eq!(node_lines.len(), 1);
    assert_eq!(node_lines[0][0], "L");
    assert_eq!(node_lines[0][2], "1");
}

#[test]
fn test_vtree_format_deterministic() {
    let fmt1 = Vtree::random(8, 17).to_vtree_text();
    let fmt2 = Vtree::random(8, 17).to_vtree_text();
    assert_eq!(fmt1, fmt2);
}

#[test]
fn test_vtree_format_roundtrip_balanced() {
    for n in 1..=10 {
        let vtree = Vtree::balanced(n);
        let fmt = vtree.to_vtree_text();
        let loaded = Vtree::from_vtree_text(&fmt).expect("roundtrip parse failed");
        let fmt2 = loaded.to_vtree_text();
        assert_eq!(
            fmt, fmt2,
            "roundtrip failed for balanced vtree with {} vars",
            n
        );
    }
}

#[test]
fn test_vtree_format_roundtrip_linear() {
    for n in 1..=10 {
        let vtree = Vtree::linear(n);
        let fmt = vtree.to_vtree_text();
        let loaded = Vtree::from_vtree_text(&fmt).expect("roundtrip parse failed");
        let fmt2 = loaded.to_vtree_text();
        assert_eq!(
            fmt, fmt2,
            "roundtrip failed for linear vtree with {} vars",
            n
        );
    }
}

#[test]
fn test_vtree_format_roundtrip_random() {
    for seed in 0..5 {
        let vtree = Vtree::random(8, seed);
        let fmt = vtree.to_vtree_text();
        let loaded = Vtree::from_vtree_text(&fmt).expect("roundtrip parse failed");
        let fmt2 = loaded.to_vtree_text();
        assert_eq!(fmt, fmt2, "roundtrip failed for random vtree seed {}", seed);
    }
}

/// Malformed text is rejected, never a panic — and the rejection says which
/// part of the file is at fault, since the reader is the only thing that has
/// seen both the line and what it contradicts. Each case carries the substrings
/// its own message must contain.
#[test]
fn each_rejected_vtree_file_names_the_id_or_token_the_file_contradicts() {
    let cases: &[(&str, &str, &[&str])] = &[
        (
            "vtree 1\nL 5 1\n",
            "leaf id past the declared node count",
            &["5", "1 nodes"],
        ),
        (
            "vtree 1\nL 0 0\n",
            "variable 0 in a 1-based format",
            &["variable 0", "1-based"],
        ),
        ("vtree 0\n", "a header declaring no nodes", &["0 nodes"]),
        (
            "vtree 3\nL 0 1\nL 1 2\nI 2 0 9\n",
            "right child past the declared node count",
            &["right child 9", "3 nodes"],
        ),
        ("", "no header at all", &["empty"]),
        (
            "nodes 2\n",
            "a header that is not 'vtree N'",
            &["header", "vtree N"],
        ),
        ("vtree x\n", "a non-numeric node count", &["node count"]),
        (
            "vtree 2\nL 0 1\n",
            "a node the header declared but no line defines",
            &["node 1"],
        ),
        (
            "vtree 3\nL 0 1\nI 1 0 0\nI 2 1 0\n",
            "a node reached from two parents",
            &["node 0", "not a tree"],
        ),
        (
            "vtree 3\nL 0 1\nL 1 2\nI 2 2 0\n",
            "an internal node that is its own child",
            &["node 2", "not a tree"],
        ),
    ];
    for &(text, what, expected) in cases {
        let err = Vtree::from_vtree_text(text)
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

/// A vtree carries each variable on exactly one leaf, so a file naming one
/// twice describes no vtree. Reported by the reader, which has the two leaf ids
/// in hand: parsed instead, the tree reaches a consumer whose own leaf-count
/// check aborts on it far from the file that caused it.
#[test]
fn a_vtree_file_naming_one_variable_on_two_leaves_is_rejected() {
    let err = Vtree::from_vtree_text("vtree 3\nL 0 1\nL 1 1\nI 2 0 1\n")
        .map(|_| ())
        .expect_err("one variable on two leaves is not a vtree")
        .to_string();
    for want in ["leaves 0 and 1", "variable 1"] {
        assert!(
            err.contains(want),
            "the message must name the variable and both leaves ({want:?}), got: {err}",
        );
    }

    // The same shape over two distinct variables is the well-formed file.
    Vtree::from_vtree_text("vtree 3\nL 0 1\nL 1 2\nI 2 0 1\n")
        .expect("two leaves naming two variables is a vtree");
}

/// The writer numbers nodes by bottom-up position, which several rotations
/// drive further and further from the node array's own order. What the file
/// says has to stay the tree that wrote it, not the numbering it started from.
#[test]
fn a_sequence_of_rotations_still_writes_a_file_that_reads_back_as_the_same_tree() {
    let mut vtree = Vtree::balanced(8);
    let mut applied = 0;
    for i in 0..vtree.num_nodes() {
        let node = VtreeIdx(i as u32);
        let rotated = if i % 2 == 0 {
            rotate::rotate_left(&mut vtree, node)
        } else {
            rotate::rotate_right(&mut vtree, node)
        };
        applied += usize::from(rotated.is_some());
    }
    assert!(applied >= 3, "the fixture must rotate more than once");

    let loaded = Vtree::from_vtree_text(&vtree.to_vtree_text())
        .expect("what this writer writes, this reader reads");
    assert!(
        loaded.same_tree(&vtree),
        "the file must describe the tree that wrote it",
    );
}
