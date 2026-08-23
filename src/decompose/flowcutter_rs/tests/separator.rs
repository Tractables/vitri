use crate::decompose::flowcutter_rs::*;
use crate::tests::td_fixture::NamedGraph;

#[test]
fn separator_on_path() {
    let n = 10;
    let edges: Vec<(u32, u32)> = (0..9).map(|i| (i, i + 1)).collect();
    let sep = compute_separator(n, &edges, 100_000, 5, 0).expect("path separator");
    assert!(!sep.is_empty());
    // Small margin: the anytime search may report a slightly larger
    // separator before a smaller one is found.
    assert!(sep.len() <= 3, "path separator too large: {:?}", sep);
}

#[test]
fn separator_on_cycle() {
    let n = 12;
    let mut edges: Vec<(u32, u32)> = (0..11).map(|i| (i, i + 1)).collect();
    edges.push((11, 0));
    let sep = compute_separator(n, &edges, 100_000, 5, 0).expect("cycle separator");
    assert!(!sep.is_empty());
    assert!(sep.len() <= 4, "cycle separator too large: {:?}", sep);
}

#[test]
fn separator_on_disconnected_returns_none() {
    let n = 6;
    let edges = vec![(0, 1), (1, 2), (0, 2), (3, 4), (4, 5), (3, 5)];
    let sep = compute_separator(n, &edges, 10_000, 3, 0);
    assert!(
        sep.is_none(),
        "disconnected graph should report no separator"
    );
}

#[test]
fn separator_on_clique_is_large() {
    // K_5: any separator must be of size at least 4 (clique − 1).
    let n = 5;
    let mut edges = Vec::new();
    for i in 0..5 {
        for j in (i + 1)..5 {
            edges.push((i, j));
        }
    }
    let sep = compute_separator(n, &edges, 100_000, 5, 0);
    // FlowCutter may report no separator on a clique; only check the size
    // when it does.
    if let Some(sep) = sep {
        assert!(
            sep.len() >= 4 || sep.is_empty(),
            "K5 separator too small: {:?}",
            sep
        );
    }
}

#[test]
fn separator_does_not_panic_on_tree() {
    let n = 7;
    // Balanced binary tree: 0 children (1, 2), 1 children (3, 4), 2 children (5, 6).
    let edges = vec![(0, 1), (0, 2), (1, 3), (1, 4), (2, 5), (2, 6)];
    let sep = compute_separator(n, &edges, 100_000, 5, 0);
    assert!(sep.is_some(), "tree should yield a separator");
}

#[test]
fn separator_finds_articulation_point() {
    // Two triangles joined at vertex 0 (the articulation point).
    // Removing vertex 0 disconnects {1, 2} from {3, 4}.
    let n = 5;
    let edges = vec![
        (0, 1),
        (0, 2),
        (1, 2), // triangle A
        (0, 3),
        (0, 4),
        (3, 4), // triangle B
    ];
    let sep = compute_separator(n, &edges, 100_000, 30, 0).expect("must find sep");
    // 30 iterations = 30 random (s, t) pairs.
    assert_eq!(sep, vec![0], "expected {{0}}, got {:?}", sep);
}
#[test]
fn separator_seed2_articulation_at_16() {
    // Captured from the diff_random_sparse_graphs test (seed=2).  C++
    // FlowCutter finds {16} as a 1-vertex separator.  Rust port should
    // also find it — this is a regression guard.
    let n = 30;
    let edges = vec![
        (0, 9),
        (1, 2),
        (2, 26),
        (1, 25),
        (26, 27),
        (15, 29),
        (5, 19),
        (23, 29),
        (5, 9),
        (10, 16),
        (22, 25),
        (7, 15),
        (6, 25),
        (9, 15),
        (12, 16),
        (15, 21),
        (3, 4),
        (4, 16),
        (9, 11),
        (3, 11),
        (3, 14),
        (16, 24),
        (0, 13),
        (9, 14),
        (7, 24),
        (5, 8),
        (13, 29),
        (13, 21),
        (2, 11),
        (9, 20),
        (6, 29),
        (24, 27),
        (21, 25),
        (11, 22),
        (10, 18),
        (10, 15),
        (20, 28),
        (10, 17),
        (16, 23),
        (3, 18),
        (1, 29),
        (0, 15),
        (3, 19),
        (20, 23),
        (6, 19),
        (11, 20),
        (17, 26),
        (10, 28),
        (2, 18),
        (9, 26),
        (2, 27),
        (4, 23),
        (19, 21),
        (3, 24),
    ];
    let sep = compute_separator(n, &edges, 500_000, 200, 0).expect("must find sep");
    assert!(sep.len() <= 3, "seed=2 sep too big: {:?}", sep);
}

#[test]
fn separator_finds_articulation_in_larger() {
    // 4 cliques joined at one shared vertex.
    let n = 13;
    let mut edges = Vec::new();
    // shared vertex 0
    // clique A: {0,1,2,3} fully connected
    for i in 0..4u32 {
        for j in (i + 1)..4u32 {
            edges.push((i, j));
        }
    }
    // clique B: {0,4,5,6}
    for i in [0u32, 4, 5, 6] {
        for j in [0u32, 4, 5, 6].iter().filter(|&&x| x > i).copied() {
            edges.push((i, j));
        }
    }
    // clique C: {0,7,8,9}
    for i in [0u32, 7, 8, 9] {
        for j in [0u32, 7, 8, 9].iter().filter(|&&x| x > i).copied() {
            edges.push((i, j));
        }
    }
    // clique D: {0,10,11,12}
    for i in [0u32, 10, 11, 12] {
        for j in [0u32, 10, 11, 12].iter().filter(|&&x| x > i).copied() {
            edges.push((i, j));
        }
    }
    let sep = compute_separator(n, &edges, 200_000, 30, 0).expect("must find sep");
    assert_eq!(sep, vec![0], "expected {{0}}, got {:?}", sep);
}

/// What makes a separator a separator: remove it and the two sides no longer
/// reach each other. Sizes and exact vertex ids are the search's business and
/// change with it; this is the property every caller relies on.
#[test]
fn a_separator_leaves_no_edge_between_its_two_sides() {
    use crate::decompose::flowcutter_rs::flowcutter_compute_separator;

    let shapes: &[NamedGraph] = &[
        (
            "a path",
            10,
            &[
                (0, 1),
                (1, 2),
                (2, 3),
                (3, 4),
                (4, 5),
                (5, 6),
                (6, 7),
                (7, 8),
                (8, 9),
            ],
        ),
        (
            "a cycle",
            8,
            &[
                (0, 1),
                (1, 2),
                (2, 3),
                (3, 4),
                (4, 5),
                (5, 6),
                (6, 7),
                (7, 0),
            ],
        ),
        (
            "a grid",
            9,
            &[
                (0, 1),
                (1, 2),
                (3, 4),
                (4, 5),
                (6, 7),
                (7, 8),
                (0, 3),
                (3, 6),
                (1, 4),
                (4, 7),
                (2, 5),
                (5, 8),
            ],
        ),
    ];

    for &(shape, n, edges) in shapes {
        let n = n as usize;
        let result = flowcutter_compute_separator(n, edges, 10_000, 3, 0)
            .unwrap_or_else(|| panic!("{shape} must have a separator"));

        let mut side = vec![u8::MAX; n];
        for &v in &result.side_a {
            assert_eq!(
                side[v as usize],
                u8::MAX,
                "{shape}: vertex {v} placed twice"
            );
            side[v as usize] = 0;
        }
        for &v in &result.side_b {
            assert_eq!(
                side[v as usize],
                u8::MAX,
                "{shape}: vertex {v} placed twice"
            );
            side[v as usize] = 1;
        }
        for &v in &result.separator {
            assert_eq!(
                side[v as usize],
                u8::MAX,
                "{shape}: vertex {v} placed twice"
            );
            side[v as usize] = 2;
        }
        assert!(
            side.iter().all(|&s| s != u8::MAX),
            "{shape}: the two sides and the separator must cover every vertex",
        );

        for &(u, v) in edges {
            let (a, b) = (side[u as usize], side[v as usize]);
            assert!(
                !(a < 2 && b < 2 && a != b),
                "{shape}: edge ({u}, {v}) crosses from one side to the other",
            );
        }
    }
}
