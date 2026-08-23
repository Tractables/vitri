//! The graph bisector on the smallest inputs it is handed.

use crate::decompose::multilevel_bisect::multilevel_bisect;
use crate::tests::td_fixture::NamedGraph;

/// Every vertex gets a side, both sides are used once there are two vertices to
/// share, and one seed gives one answer. The framework above descends into both
/// sides without checking either, so an empty side is not a split it can use.
#[test]
fn a_graph_bisection_fills_both_sides_on_the_tiny_shapes() {
    let shapes: &[NamedGraph] = &[
        ("no vertices", 0, &[]),
        ("one vertex", 1, &[]),
        ("two vertices", 2, &[(0, 1)]),
        ("a path", 6, &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)]),
        ("a star", 5, &[(0, 1), (0, 2), (0, 3), (0, 4)]),
        (
            "a complete graph",
            5,
            &[
                (0, 1),
                (0, 2),
                (0, 3),
                (0, 4),
                (1, 2),
                (1, 3),
                (1, 4),
                (2, 3),
                (2, 4),
                (3, 4),
            ],
        ),
        ("no edges at all", 5, &[]),
        (
            "two triangles",
            6,
            &[(0, 1), (1, 2), (0, 2), (3, 4), (4, 5), (3, 5)],
        ),
    ];

    for &(shape, n, edges) in shapes {
        let n = n as usize;
        let sides = multilevel_bisect(n, edges, 0.4, 0);
        assert_eq!(sides.len(), n, "{shape}: one side bit per vertex");
        assert!(
            sides.iter().all(|&s| s <= 1),
            "{shape}: a vertex is on side 0 or side 1, got {sides:?}",
        );
        if n >= 2 {
            assert!(
                sides.contains(&0) && sides.contains(&1),
                "{shape}: both sides must be used, got {sides:?}",
            );
        }
        assert_eq!(
            sides,
            multilevel_bisect(n, edges, 0.4, 0),
            "{shape}: one seed, one bisection",
        );
    }
}
