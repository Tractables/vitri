//! The top-level balanced separator, packaged for a caller that wants a split
//! rather than a decomposition.
//!
//! [`super::compute_separator`] returns the separator alone; this adds what a
//! caller splitting a graph on it needs — the two sides. FlowCutter does not
//! report the partition through the search, so the sides are recovered by
//! flood-fill on `G \ S`.

/// Result of a single FlowCutter top-level separator computation.
///
/// `side_a` and `side_b` are disjoint from `separator` and from each other.
/// Their union plus `separator` covers every vertex in the input subgraph.
/// The sides are computed by flood-fill on G \ S: FlowCutter itself doesn't
/// expose the partition assignment through this FFI path, so we recover it
/// by BFS.  When G \ S has more than two connected components (possible on
/// near-disconnected subgraphs), components are greedily packed into the
/// smaller side.
#[derive(Debug, Clone)]
pub(crate) struct FcSeparatorResult {
    /// The balanced separator: vertices removed to split the graph into `side_a`/`side_b`.
    pub separator: Vec<u32>,
    /// One side of the graph with `separator` removed.
    pub side_a: Vec<u32>,
    /// The other side of the graph with `separator` removed.
    pub side_b: Vec<u32>,
}

/// Compute one FlowCutter top-level balanced separator of the graph given by
/// `edges` on `num_nodes` vertices.  Returns `None` if the separator is empty
/// or degenerate (one side empty after removing S).
///
/// `steps`/`iters`/`timeout_ms` match the original FFI shape: `timeout_ms == 0`
/// means step-budget only.
pub(crate) fn flowcutter_compute_separator(
    num_nodes: usize,
    edges: &[(u32, u32)],
    steps: i64,
    iters: i32,
    timeout_ms: i64,
) -> Option<FcSeparatorResult> {
    if num_nodes < 3 {
        return None;
    }

    let sep = super::compute_separator(num_nodes, edges, steps, iters, timeout_ms)?;

    if sep.is_empty() || sep.len() >= num_nodes {
        return None;
    }

    let (side_a, side_b) = split_sides_bfs(num_nodes, edges, &sep)?;
    Some(FcSeparatorResult {
        separator: sep,
        side_a,
        side_b,
    })
}

/// Flood-fill G \ S into connected components and pack components greedily
/// into two sides balanced by vertex count.  Returns `None` if either side
/// ends up empty.
fn split_sides_bfs(
    num_nodes: usize,
    edges: &[(u32, u32)],
    separator: &[u32],
) -> Option<(Vec<u32>, Vec<u32>)> {
    let in_sep = {
        let mut flag = vec![false; num_nodes];
        for &v in separator {
            flag[v as usize] = true;
        }
        flag
    };

    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); num_nodes];
    for &(u, v) in edges {
        if in_sep[u as usize] || in_sep[v as usize] {
            continue;
        }
        adj[u as usize].push(v);
        adj[v as usize].push(u);
    }

    let mut component_of = vec![u32::MAX; num_nodes];
    let mut components: Vec<Vec<u32>> = Vec::new();

    for start in 0..num_nodes {
        if in_sep[start] || component_of[start] != u32::MAX {
            continue;
        }
        let cid = components.len() as u32;
        let mut stack = vec![start as u32];
        let mut comp = Vec::new();
        component_of[start] = cid;
        while let Some(v) = stack.pop() {
            comp.push(v);
            for &nb in &adj[v as usize] {
                if component_of[nb as usize] == u32::MAX {
                    component_of[nb as usize] = cid;
                    stack.push(nb);
                }
            }
        }
        components.push(comp);
    }

    if components.is_empty() {
        return None;
    }

    // Pack components into two sides greedily (largest-first) to balance.
    components.sort_by_key(|c| std::cmp::Reverse(c.len()));
    let mut side_a: Vec<u32> = Vec::new();
    let mut side_b: Vec<u32> = Vec::new();
    for comp in components {
        if side_a.len() <= side_b.len() {
            side_a.extend(comp);
        } else {
            side_b.extend(comp);
        }
    }

    if side_a.is_empty() || side_b.is_empty() {
        return None;
    }

    side_a.sort_unstable();
    side_b.sort_unstable();
    Some((side_a, side_b))
}
