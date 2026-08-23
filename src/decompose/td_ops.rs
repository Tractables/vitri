//! Tree-decomposition surgery: rooting a decomposition into a walkable
//! forest, projecting one onto a variable subset, and gluing two back together
//! at a shared separator.
//!
//! These operate on a decomposition alone — no formula, no vtree, no backend —
//! which is what earns them a module of their own: a new operation on a
//! decomposition belongs here, not inside whichever backend happened to need it
//! first. The type itself, and the graph views a decomposition is built from,
//! belong to [`super::td_parse`].
//!
//! Every function here that returns a decomposition preserves the running
//! intersection property (RIP): a variable's bags form a connected subtree of
//! the result whenever they did in the input. The per-function docs say how.

use std::collections::VecDeque;

use rustc_hash::FxHashSet;

use super::{TdBag, TreeDecomposition, local_index};

/// A decomposition rooted for a downward walk: what one breadth-first sweep
/// over the bag tree leaves behind.
pub(super) struct RootedForest {
    /// Bag indices in breadth-first order, so every bag follows its parent and
    /// precedes its children. Reversed, it is a leaves-first order.
    pub(super) order: Vec<usize>,
    /// Each bag's parent, [`NO_PARENT`] at a component root and at a bag no
    /// root reached.
    pub(super) parent: Vec<usize>,
    /// Each bag's distance from its component root; `0` at a bag the walk did
    /// not reach.
    pub(super) depth: Vec<usize>,
    /// The bag each component was entered at, in the order the walk entered
    /// them.
    pub(super) component_roots: Vec<usize>,
}

/// [`RootedForest::parent`] at a bag with no parent.
pub(super) const NO_PARENT: usize = usize::MAX;

/// Root a decomposition's bag tree at `roots` and walk it breadth-first.
///
/// A decomposition need not be connected — a projection that drops a separator
/// leaves several components behind — so this roots a forest rather than a
/// tree: `roots` is tried in order, and each entry that a previous one has not
/// already reached opens a new component. Ending `roots` with `0..n` therefore
/// says "these bags first, then whatever they missed", and starting from
/// `0..n` alone says "no preference": either way every bag is reached exactly
/// once.
pub(super) fn rooted_forest(
    adj: &[Vec<usize>],
    roots: impl IntoIterator<Item = usize>,
) -> RootedForest {
    let n = adj.len();
    let mut parent = vec![NO_PARENT; n];
    let mut depth = vec![0usize; n];
    let mut order = Vec::with_capacity(n);
    let mut visited = vec![false; n];
    let mut component_roots = Vec::new();
    let mut queue = VecDeque::new();
    for start in roots {
        if visited[start] {
            continue;
        }
        component_roots.push(start);
        visited[start] = true;
        queue.push_back(start);
        while let Some(t) = queue.pop_front() {
            order.push(t);
            for &nb in &adj[t] {
                if !visited[nb] {
                    visited[nb] = true;
                    parent[nb] = t;
                    depth[nb] = depth[t] + 1;
                    queue.push_back(nb);
                }
            }
        }
    }
    RootedForest {
        order,
        parent,
        depth,
        component_roots,
    }
}

/// Result of projecting a TD onto a variable subset.
pub(super) struct ProjectedTd {
    pub(super) td: TreeDecomposition,
    /// Mapping from local variable IDs (0..k) back to global variable IDs.
    pub(super) local_to_global: Vec<u32>,
}

/// Project a tree decomposition onto a variable subset, preserving global
/// variable IDs (no renumbering).  Bags still contain IDs from 0..num_vars.
///
/// Variables outside `keep_vars` are dropped from every bag, bags left empty are
/// contracted away, and each surviving bag is reattached to its nearest
/// surviving ancestor.
///
/// The running intersection property is preserved: if a variable appears in bags
/// B_i and B_j in the original TD, it appears in all bags on the path between
/// them; dropping a variable removes it from a connected subtree of bags, so
/// what remains for each retained variable is still a connected subtree.
///
/// Returns `None` when every projected bag would be empty.
pub(super) fn project_td_keeping_global_ids(
    td: &TreeDecomposition,
    keep_vars: &FxHashSet<u32>,
    num_vars: u32,
) -> Option<TreeDecomposition> {
    let n = td.bags.len();
    if n == 0 {
        return None;
    }

    let projected: Vec<Vec<u32>> = td
        .bags
        .iter()
        .map(|bag| {
            bag.vertices
                .iter()
                .copied()
                .filter(|v| keep_vars.contains(v))
                .collect()
        })
        .collect();

    let non_empty: Vec<usize> = (0..n).filter(|&i| !projected[i].is_empty()).collect();
    if non_empty.is_empty() {
        return None;
    }

    let mut old_to_new = vec![None; n];
    for (new_i, &old_i) in non_empty.iter().enumerate() {
        old_to_new[old_i] = Some(new_i);
    }

    let new_count = non_empty.len();
    let mut new_adj: Vec<Vec<usize>> = vec![Vec::new(); new_count];

    let parent_in_td = rooted_forest(&td.adj, 0..n).parent;

    for &old_i in &non_empty {
        let new_i = old_to_new[old_i].unwrap();
        let mut ancestor = parent_in_td[old_i];
        while ancestor != NO_PARENT {
            if let Some(new_j) = old_to_new[ancestor] {
                new_adj[new_i].push(new_j);
                new_adj[new_j].push(new_i);
                break;
            }
            ancestor = parent_in_td[ancestor];
        }
    }

    let new_bags: Vec<TdBag> = non_empty
        .iter()
        .enumerate()
        .map(|(new_id, &old_id)| {
            let mut verts = projected[old_id].clone();
            verts.sort_unstable();
            TdBag {
                id: new_id,
                vertices: verts,
            }
        })
        .collect();

    Some(TreeDecomposition {
        kind: td.kind,
        bags: new_bags,
        adj: new_adj,
        num_vars,
    })
}

/// Project a tree decomposition onto a variable subset and renumber the result
/// into a local variable space.
///
/// Thin wrapper over [`project_td_keeping_global_ids`], which does the bag
/// filtering, empty-bag contraction and tree rebuild. The only extra work here
/// is the relabelling: `keep_vars` sorted ascending becomes local ids `0..k`, so
/// the returned decomposition numbers its variables `0..k` and
/// `local_to_global` maps them back.
pub(super) fn project_td(
    td: &TreeDecomposition,
    keep_vars: &FxHashSet<u32>,
) -> Option<ProjectedTd> {
    let mut sorted_vars: Vec<u32> = keep_vars.iter().copied().collect();
    sorted_vars.sort_unstable();
    let global_to_local = local_index(&sorted_vars);

    // Still in the global space at this point, so it keeps the source TD's
    // variable count; the relabelling below moves it into the local one.
    let mut projected = project_td_keeping_global_ids(td, keep_vars, td.num_vars)?;

    // Relabelling is order-preserving (a local id is the rank of its global id
    // in `sorted_vars`), so bags that came back sorted by global id stay sorted.
    for bag in &mut projected.bags {
        for v in &mut bag.vertices {
            *v = global_to_local[&*v];
        }
    }
    projected.num_vars = sorted_vars.len() as u32;

    Some(ProjectedTd {
        td: projected,
        local_to_global: sorted_vars,
    })
}

// ---------------------------------------------------------------------------
// Separator glue
// ---------------------------------------------------------------------------

/// Find one bag that contains every variable in `sep_vars`, augmenting the TD
/// if necessary.  Returns the index of that bag.
///
/// Strategy: pick the bag with the largest intersection with `sep_vars`, then
/// for each missing `v ∈ sep_vars`, BFS from a bag that contains `v` to the
/// anchor bag and add `v` to every bag on the path.  This preserves the
/// running intersection property (RIP): `v`'s original bag-subtree is extended
/// by a connected path of bags, all containing `v`.  A `v` whose bag is in
/// another component has no such path, so the two components are joined by one
/// edge first — see the branch below.
///
/// Bag widths may grow by up to `|sep_vars \ anchor_bag|` in the worst case;
/// the refinement's `(width, total_bag_size)` guard catches cases where this
/// growth wipes out the benefit of the cut.
fn augment_for_separator(td: &mut TreeDecomposition, sep_vars: &[u32]) -> Option<usize> {
    if td.bags.is_empty() {
        return None;
    }

    let sep_set: FxHashSet<u32> = sep_vars.iter().copied().collect();

    let anchor = (0..td.bags.len()).max_by_key(|&i| {
        td.bags[i]
            .vertices
            .iter()
            .filter(|v| sep_set.contains(v))
            .count()
    })?;

    for &v in sep_vars {
        if td.bags[anchor].vertices.contains(&v) {
            continue;
        }
        let src = (0..td.bags.len()).find(|&i| td.bags[i].vertices.contains(&v));
        match src {
            Some(src) if src != anchor => match bag_path_bfs(&td.adj, src, anchor) {
                Some(path) => {
                    for &b in &path {
                        if !td.bags[b].vertices.contains(&v) {
                            td.bags[b].vertices.push(v);
                        }
                    }
                }
                None => {
                    // `v`'s bag is in another component, so there is no path of
                    // bags to carry it along and writing it into both ends
                    // would leave its bags disconnected. One edge joins the two
                    // components first: between two components it can close no
                    // cycle, and it disconnects no variable's bags, so `v` then
                    // travels it the way it would any other edge.
                    td.adj[src].push(anchor);
                    td.adj[anchor].push(src);
                    td.bags[anchor].vertices.push(v);
                }
            },
            _ => {
                td.bags[anchor].vertices.push(v);
            }
        }
    }

    for bag in td.bags.iter_mut() {
        bag.vertices.sort_unstable();
        bag.vertices.dedup();
    }

    Some(anchor)
}

/// Shortest path between two bag indices in the TD tree.  Returns the
/// sequence of bag indices from `src` to `dst` inclusive, or `None` when the
/// two are in different components and no path exists.
fn bag_path_bfs(adj: &[Vec<usize>], src: usize, dst: usize) -> Option<Vec<usize>> {
    if src == dst {
        return Some(vec![src]);
    }
    let parent = rooted_forest(adj, [src]).parent;
    if parent[dst] == NO_PARENT {
        return None;
    }
    let mut path = vec![dst];
    let mut x = dst;
    while x != src {
        x = parent[x];
        path.push(x);
    }
    path.reverse();
    Some(path)
}

/// Glue two tree decompositions at a shared separator.
///
/// Both `td_a` and `td_b` must have already been projected to retain every
/// variable in `sep_vars` (the caller enforces this by passing `side ∪ sep`
/// as the keep-set to `project_td_keeping_global_ids`).  Global variable IDs
/// are preserved across both inputs.
///
/// The glued TD gets a new bag-0 containing exactly `sep_vars`, with the
/// anchor bag from each side attached as a neighbor.  Each side's bags are
/// augmented (if needed) so their anchor contains every `sep` variable,
/// preserving RIP for the glued tree.
pub(super) fn glue_at_separator(
    mut td_a: TreeDecomposition,
    mut td_b: TreeDecomposition,
    sep_vars: &[u32],
    num_vars: u32,
) -> Option<TreeDecomposition> {
    let anchor_a = augment_for_separator(&mut td_a, sep_vars)?;
    let anchor_b = augment_for_separator(&mut td_b, sep_vars)?;

    let mut sep_sorted: Vec<u32> = sep_vars.to_vec();
    sep_sorted.sort_unstable();
    sep_sorted.dedup();
    let sep_bag = TdBag {
        id: 0,
        vertices: sep_sorted,
    };

    let mut bags: Vec<TdBag> = Vec::with_capacity(1 + td_a.bags.len() + td_b.bags.len());
    bags.push(sep_bag);

    let a_offset = 1;
    for (i, bag) in td_a.bags.iter().enumerate() {
        bags.push(TdBag {
            id: a_offset + i,
            vertices: bag.vertices.clone(),
        });
    }

    let b_offset = a_offset + td_a.bags.len();
    for (i, bag) in td_b.bags.iter().enumerate() {
        bags.push(TdBag {
            id: b_offset + i,
            vertices: bag.vertices.clone(),
        });
    }

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); bags.len()];

    for (i, nbs) in td_a.adj.iter().enumerate() {
        for &nb in nbs {
            if nb > i {
                adj[a_offset + i].push(a_offset + nb);
                adj[a_offset + nb].push(a_offset + i);
            }
        }
    }
    for (i, nbs) in td_b.adj.iter().enumerate() {
        for &nb in nbs {
            if nb > i {
                adj[b_offset + i].push(b_offset + nb);
                adj[b_offset + nb].push(b_offset + i);
            }
        }
    }

    adj[0].push(a_offset + anchor_a);
    adj[a_offset + anchor_a].push(0);
    adj[0].push(b_offset + anchor_b);
    adj[b_offset + anchor_b].push(0);

    Some(TreeDecomposition {
        kind: td_a.kind,
        bags,
        adj,
        num_vars,
    })
}
