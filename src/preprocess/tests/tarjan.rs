use crate::preprocess::tarjan::*;

/// Build a representative map (node → min node in its SCC) from groups.
fn groups_to_rep(n: usize, groups: &[Vec<usize>]) -> Vec<usize> {
    let mut rep = vec![0usize; n];
    for group in groups {
        let min = *group.iter().min().unwrap();
        for &node in group {
            rep[node] = min;
        }
    }
    rep
}

#[test]
fn a_graph_with_no_nodes_has_no_components() {
    let groups = tarjan_scc_groups(0, &[]);
    assert!(groups.is_empty());
}

#[test]
fn single_node_no_edges() {
    let adj = vec![vec![]];
    let groups = tarjan_scc_groups(1, &adj);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0], vec![0]);
}

#[test]
fn two_nodes_no_cycle() {
    // 0 → 1, no back edge: two singleton SCCs
    let adj = vec![vec![1], vec![]];
    let groups = tarjan_scc_groups(2, &adj);
    assert_eq!(groups.len(), 2);
    // Every node appears exactly once
    let mut flat: Vec<usize> = groups.iter().flatten().copied().collect();
    flat.sort_unstable();
    assert_eq!(flat, vec![0, 1]);
}

#[test]
fn simple_cycle() {
    // 0 → 1 → 2 → 0: one SCC of size 3
    let adj = vec![vec![1], vec![2], vec![0]];
    let groups = tarjan_scc_groups(3, &adj);
    assert_eq!(groups.len(), 1);
    let mut members = groups[0].clone();
    members.sort_unstable();
    assert_eq!(members, vec![0, 1, 2]);
}

#[test]
fn a_node_that_only_points_into_a_cycle_is_its_own_component() {
    // Cycle: 0 → 1 → 2 → 0; tail: 3 → 0 (3 not in cycle)
    // Expected: one 3-node SCC {0,1,2} and one singleton {3}
    let adj = vec![vec![1], vec![2], vec![0], vec![0]];
    let groups = tarjan_scc_groups(4, &adj);
    assert_eq!(groups.len(), 2);

    let mut sizes: Vec<usize> = groups.iter().map(|g| g.len()).collect();
    sizes.sort_unstable();
    assert_eq!(sizes, vec![1, 3]);

    // The cycle SCC contains exactly {0, 1, 2}
    let cycle_scc: Vec<usize> = {
        let mut g = groups.iter().find(|g| g.len() == 3).unwrap().clone();
        g.sort_unstable();
        g
    };
    assert_eq!(cycle_scc, vec![0, 1, 2]);

    // The singleton is {3}
    let singleton = groups.iter().find(|g| g.len() == 1).unwrap();
    assert_eq!(singleton, &vec![3]);
}

#[test]
fn representative_map_cycle_plus_tail() {
    // Same graph: rep of {0,1,2} is 0; rep of {3} is 3.
    let adj = vec![vec![1], vec![2], vec![0], vec![0]];
    let groups = tarjan_scc_groups(4, &adj);
    let rep = groups_to_rep(4, &groups);
    assert_eq!(rep[0], 0);
    assert_eq!(rep[1], 0);
    assert_eq!(rep[2], 0);
    assert_eq!(rep[3], 3);
}

#[test]
fn two_separate_cycles() {
    // {0,1,2} and {3,4}: two independent cycles
    let adj = vec![vec![1], vec![2], vec![0], vec![4], vec![3]];
    let groups = tarjan_scc_groups(5, &adj);
    assert_eq!(groups.len(), 2);
    let mut flat: Vec<usize> = groups.iter().flatten().copied().collect();
    flat.sort_unstable();
    assert_eq!(flat, vec![0, 1, 2, 3, 4]);
}

#[test]
fn self_loop() {
    // Node 0 with a self-loop: still a singleton SCC
    let adj = vec![vec![0]];
    let groups = tarjan_scc_groups(1, &adj);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0], vec![0]);
}
