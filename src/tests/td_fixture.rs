//! The tree decompositions the construction tests run over, and the checks
//! every one of them makes on a decomposition.
//!
//! These are read from both test trees — the crate-root one and the
//! beside-module trees under `decompose` — so they live here rather than in any
//! one of them: a decomposition is valid or not on the same terms whichever
//! backend produced it, and one implementation of that judgement is what keeps
//! two backends from being held to different standards.

use crate::decompose::TreeDecomposition;
use crate::tests::common::make_td;

/// A named graph a decomposition test runs over: how many vertices it has, and
/// which pairs of them are joined.
pub(crate) type NamedGraph = (&'static str, u32, &'static [(u32, u32)]);

/// A named graph carrying its own treewidth, for the tests that hold a
/// decomposition of it to that bound.
pub(crate) type GraphAtItsWidth = (&'static str, u32, &'static [(u32, u32)], u32);

/// Whether the bags form one tree rather than several.
pub(crate) fn is_connected(td: &TreeDecomposition) -> bool {
    if td.bags.is_empty() {
        return true;
    }
    let mut seen = vec![false; td.bags.len()];
    let mut stack = vec![0usize];
    seen[0] = true;
    let mut count = 1;
    while let Some(bag) = stack.pop() {
        for &next in &td.adj[bag] {
            if !seen[next] {
                seen[next] = true;
                count += 1;
                stack.push(next);
            }
        }
    }
    count == td.bags.len()
}

/// The running intersection property: for every variable in the decomposition,
/// the bags holding it form a connected subtree.
///
/// The property everything downstream depends on — a variable whose bags are
/// disconnected has no single place in the vtree the conversion builds — so it
/// is asserted from one implementation here rather than per construction.
pub(crate) fn assert_rip(td: &TreeDecomposition) {
    for (var, holders) in bags_per_var(td) {
        if holders.len() <= 1 {
            continue;
        }
        let start = holders[0];
        let mut visited = vec![false; td.bags.len()];
        visited[start] = true;
        let mut queue = std::collections::VecDeque::from([start]);
        let mut reached = 1usize;
        while let Some(bag) = queue.pop_front() {
            for &next in &td.adj[bag] {
                if !visited[next] && td.bags[next].vertices.contains(&var) {
                    visited[next] = true;
                    reached += 1;
                    queue.push_back(next);
                }
            }
        }
        assert_eq!(
            reached,
            holders.len(),
            "running intersection violated for variable {var}: bags {holders:?} are not \
             connected through bags holding it",
        );
    }
}

/// Everything a tree decomposition of a graph must satisfy: every vertex is in
/// some bag, every edge's two endpoints share one, and the running intersection
/// holds.
pub(crate) fn assert_valid_td(td: &TreeDecomposition, num_vars: u32, edges: &[(u32, u32)]) {
    let mut seen = vec![false; num_vars as usize];
    for bag in &td.bags {
        for &v in &bag.vertices {
            if (v as usize) < seen.len() {
                seen[v as usize] = true;
            }
        }
    }
    for (v, &in_bag) in seen.iter().enumerate() {
        assert!(in_bag, "vertex {v} is in no bag");
    }

    for &(u, v) in edges {
        assert!(
            td.bags
                .iter()
                .any(|b| b.vertices.contains(&u) && b.vertices.contains(&v)),
            "edge ({u}, {v}) is covered by no bag",
        );
    }

    assert_rip(td);
}

/// Which bags hold each variable, in bag order.
fn bags_per_var(td: &TreeDecomposition) -> std::collections::BTreeMap<u32, Vec<usize>> {
    let mut per_var: std::collections::BTreeMap<u32, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (i, bag) in td.bags.iter().enumerate() {
        for &v in &bag.vertices {
            let holders = per_var.entry(v).or_default();
            if holders.last() != Some(&i) {
                holders.push(i);
            }
        }
    }
    per_var
}

/// A three-bag path decomposition of six variables, sharing two variables
/// across the first join and one across the second.
pub(crate) fn make_test_td() -> TreeDecomposition {
    make_td(
        vec![vec![0, 1, 2], vec![1, 2, 3], vec![3, 4, 5]],
        vec![(0, 1), (1, 2)],
        6,
    )
}
