//! Canonical iterative Tarjan SCC implementation.

/// Iterative Tarjan's SCC on a directed graph of `n` nodes with adjacency list `adj`.
///
/// `adj[v]` is the list of outgoing neighbours of node `v`.
/// Returns SCCs as groups of node indices, in reverse topological order; within
/// each group, nodes are in pop order from the internal stack — not a canonical
/// order, so a caller needing e.g. the minimum element must search the group.
pub(super) fn tarjan_scc_groups(n: usize, adj: &[Vec<usize>]) -> Vec<Vec<usize>> {
    struct Frame {
        node: usize,
        next_edge: usize,
        is_init: bool,
    }

    let mut index_counter: usize = 0;
    let mut tarjan_stack: Vec<usize> = Vec::new();
    let mut on_stack = vec![false; n];
    let mut index = vec![usize::MAX; n];
    let mut lowlink = vec![0usize; n];
    let mut result: Vec<Vec<usize>> = Vec::new();

    let mut call_stack: Vec<Frame> = Vec::new();

    for start in 0..n {
        if index[start] != usize::MAX {
            continue;
        }

        call_stack.push(Frame {
            node: start,
            next_edge: 0,
            is_init: true,
        });

        while let Some(frame) = call_stack.last_mut() {
            let node = frame.node;

            if frame.is_init {
                index[node] = index_counter;
                lowlink[node] = index_counter;
                index_counter += 1;
                tarjan_stack.push(node);
                on_stack[node] = true;
                frame.is_init = false;
            }

            if frame.next_edge < adj[node].len() {
                let w = adj[node][frame.next_edge];
                frame.next_edge += 1;

                if index[w] == usize::MAX {
                    call_stack.push(Frame {
                        node: w,
                        next_edge: 0,
                        is_init: true,
                    });
                } else if on_stack[w] {
                    lowlink[node] = lowlink[node].min(index[w]);
                }
                continue;
            }

            if lowlink[node] == index[node] {
                let mut scc = Vec::new();
                loop {
                    let w = tarjan_stack.pop().unwrap();
                    on_stack[w] = false;
                    scc.push(w);
                    if w == node {
                        break;
                    }
                }
                result.push(scc);
            }

            let finished_lowlink = lowlink[node];
            call_stack.pop();
            if let Some(parent) = call_stack.last_mut() {
                lowlink[parent.node] = lowlink[parent.node].min(finished_lowlink);
            }
        }
    }

    result
}
