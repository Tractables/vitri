use crate::preprocess::tarjan::*;

/// The components of the graph `adj` lists the out-neighbours of.
fn components(adj: &[Vec<usize>]) -> Vec<Vec<usize>> {
    tarjan_scc_groups(adj.len(), |v| adj[v].as_slice())
}

/// Build a representative map (node → min node in its SCC) from groups; a
/// node in no group is its own.
fn groups_to_rep(n: usize, groups: &[Vec<usize>]) -> Vec<usize> {
    let mut rep: Vec<usize> = (0..n).collect();
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
    assert!(components(&[]).is_empty());
}

#[test]
fn a_node_without_edges_is_in_no_group() {
    assert!(components(&[vec![]]).is_empty());
}

#[test]
fn an_edge_without_a_way_back_forms_no_group() {
    // 0 → 1, no back edge: each node is a component of its own.
    assert!(components(&[vec![1], vec![]]).is_empty());
}

#[test]
fn simple_cycle() {
    // 0 → 1 → 2 → 0: one SCC of size 3
    let groups = components(&[vec![1], vec![2], vec![0]]);
    assert_eq!(groups.len(), 1);
    let mut members = groups[0].clone();
    members.sort_unstable();
    assert_eq!(members, vec![0, 1, 2]);
}

#[test]
fn a_node_that_only_points_into_a_cycle_is_left_out_of_its_group() {
    // Cycle: 0 → 1 → 2 → 0; tail: 3 → 0 (3 not in cycle)
    let groups = components(&[vec![1], vec![2], vec![0], vec![0]]);
    assert_eq!(groups.len(), 1);
    let mut cycle = groups[0].clone();
    cycle.sort_unstable();
    assert_eq!(cycle, vec![0, 1, 2]);
}

#[test]
fn representative_map_cycle_plus_tail() {
    // Same graph: rep of {0,1,2} is 0; rep of {3} is 3.
    let groups = components(&[vec![1], vec![2], vec![0], vec![0]]);
    let rep = groups_to_rep(4, &groups);
    assert_eq!(rep, vec![0, 0, 0, 3]);
}

#[test]
fn two_separate_cycles() {
    // {0,1,2} and {3,4}: two independent cycles
    let groups = components(&[vec![1], vec![2], vec![0], vec![4], vec![3]]);
    assert_eq!(groups.len(), 2);
    let mut flat: Vec<usize> = groups.iter().flatten().copied().collect();
    flat.sort_unstable();
    assert_eq!(flat, vec![0, 1, 2, 3, 4]);
}

#[test]
fn a_self_loop_forms_no_group() {
    // Node 0 with a self-loop is still a component of its own.
    assert!(components(&[vec![0]]).is_empty());
}
