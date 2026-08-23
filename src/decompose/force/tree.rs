//! From a layout to a tree.
//!
//! Two ways of turning placed points into a vtree: agglomerating them
//! along a Euclidean minimum spanning tree, and cutting them
//! recursively at the median of their principal axis.

use super::*;

// ---------------------------------------------------------------------------
// Euclidean MST → single-linkage merge tree
// ---------------------------------------------------------------------------

/// Exact Euclidean MST via Prim, O(d·n²) time and O(n) memory. Edges are
/// `(length, a, b)`.
pub(super) fn emst_prim(pts: &[Vec<f64>]) -> Vec<(f64, u32, u32)> {
    let n = pts.len();
    let mut in_tree = vec![false; n];
    let mut dist_to = vec![f64::INFINITY; n];
    let mut parent = vec![u32::MAX; n];
    dist_to[0] = 0.0;
    let mut edges = Vec::with_capacity(n.saturating_sub(1));
    for _ in 0..n {
        let mut u = usize::MAX;
        let mut best = f64::INFINITY;
        for (i, &done) in in_tree.iter().enumerate() {
            if !done && dist_to[i] < best {
                best = dist_to[i];
                u = i;
            }
        }
        if u == usize::MAX {
            break; // no reachable frontier (only on non-finite coordinates)
        }
        in_tree[u] = true;
        if parent[u] != u32::MAX {
            edges.push((dist_to[u], parent[u], u as u32));
        }
        for i in 0..n {
            if !in_tree[i] {
                let dd = dist(&pts[i], &pts[u]);
                if dd < dist_to[i] {
                    dist_to[i] = dd;
                    parent[i] = u as u32;
                }
            }
        }
    }
    edges
}

/// Deterministic grid-bucketed k-NN candidate graph for large `n`. Each point
/// gathers roughly `k` nearest neighbours through expanding cell rings. The
/// candidate set need not contain the true Euclidean MST — the union-find fold in
/// [`mst_tree`] closes any leftover components into one spanning structure — so
/// this affects vtree QUALITY only, never validity. Memory is O(n + incidence).
///
/// Above `d = 2` the spatial buckets use the FIRST TWO whitened axes, which carry
/// the most variance, while candidate edge lengths use the full `d`-dimensional
/// distance. That keeps the complexity class sub-quadratic at any dimension.
pub(super) fn emst_grid_candidates(pts: &[Vec<f64>]) -> Vec<(f64, u32, u32)> {
    let n = pts.len();
    let (mut minx, mut miny) = (f64::INFINITY, f64::INFINITY);
    let (mut maxx, mut maxy) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for q in pts {
        minx = minx.min(q[0]);
        miny = miny.min(q[1]);
        maxx = maxx.max(q[0]);
        maxy = maxy.max(q[1]);
    }
    let w = (maxx - minx).max(EPS);
    let h = (maxy - miny).max(EPS);
    let area = w * h;
    let mut cell = (2.0 * area / n as f64).sqrt();
    if !cell.is_finite() || cell <= EPS {
        cell = (w.max(h)).max(EPS); // all-coincident guard: one bucket
    }
    let cellx = |x: f64| ((x - minx) / cell).floor() as i64;
    let celly = |y: f64| ((y - miny) / cell).floor() as i64;

    let mut grid: FxHashMap<(i64, i64), Vec<u32>> = FxHashMap::default();
    for (i, q) in pts.iter().enumerate() {
        grid.entry((cellx(q[0]), celly(q[1])))
            .or_default()
            .push(i as u32);
    }
    let ncols = cellx(maxx) + 1;
    let nrows = celly(maxy) + 1;
    let max_ring = ncols.max(nrows).max(1);

    let mut edges = Vec::with_capacity(n * KNN_K);
    let mut cand: Vec<(f64, u32)> = Vec::new();
    for (i, pi) in pts.iter().enumerate() {
        cand.clear();
        let (cx, cy) = (cellx(pi[0]), celly(pi[1]));
        let mut ring: i64 = 0;
        let mut rings_after_hit = 0;
        loop {
            // Cells at Chebyshev distance exactly `ring` from (cx, cy).
            let visit = |gx: i64, gy: i64, cand: &mut Vec<(f64, u32)>| {
                if let Some(bucket) = grid.get(&(gx, gy)) {
                    for &j in bucket {
                        if j as usize != i {
                            cand.push((dist(&pts[j as usize], pi), j));
                        }
                    }
                }
            };
            if ring == 0 {
                visit(cx, cy, &mut cand);
            } else {
                for dx in -ring..=ring {
                    visit(cx + dx, cy - ring, &mut cand);
                    visit(cx + dx, cy + ring, &mut cand);
                }
                for dy in (-ring + 1)..ring {
                    visit(cx - ring, cy + dy, &mut cand);
                    visit(cx + ring, cy + dy, &mut cand);
                }
            }
            if cand.len() >= KNN_K {
                // One extra ring after first reaching k, for better edges.
                rings_after_hit += 1;
                if rings_after_hit >= 2 {
                    break;
                }
            }
            ring += 1;
            if ring > max_ring {
                break;
            }
        }
        cand.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        cand.truncate(KNN_K);
        for &(len, j) in &cand {
            edges.push((len, i as u32, j));
        }
    }
    edges
}

/// Candidate edge set for `w = co`, used at every size: grid-kNN Euclidean edges
/// together with all co-occurring variable pairs (per clause, for clauses no wider
/// than [`CO_CLAUSE_CAP`]), each Euclidean length divided by `1 + #clauses containing
/// both`. Member lists are reconstructed from the clause-contiguous incidence
/// layout. Memory is O(incidence + candidate pairs) — no quadratic hash probing
/// inside Prim.
pub(super) fn cooccurrence_candidates(pts: &[Vec<f64>], inc: &Incidence) -> Vec<(f64, u32, u32)> {
    let mut cooc: FxHashMap<(u32, u32), u32> = FxHashMap::default();
    let mut start = 0usize;
    for &size in &inc.sizes {
        let sz = size as usize;
        let members = &inc.var_of[start..start + sz];
        start += sz;
        if sz <= CO_CLAUSE_CAP {
            // Members are sorted by `build_incidence`, so i < j ⇒ members[i] < members[j].
            for (i, &mi) in members.iter().enumerate() {
                for &mj in &members[(i + 1)..] {
                    *cooc.entry((mi, mj)).or_insert(0) += 1;
                }
            }
        }
    }
    let mut edges = emst_grid_candidates(pts);
    for &(u, v) in cooc.keys() {
        edges.push((dist(&pts[u as usize], &pts[v as usize]), u, v));
    }
    for e in edges.iter_mut() {
        let key = if e.1 < e.2 { (e.1, e.2) } else { (e.2, e.1) };
        let co = cooc.get(&key).copied().unwrap_or(0);
        e.0 /= 1.0 + co as f64;
    }
    edges
}

/// Per-side aggregate carried up the top-down build: variable count, minimum
/// variable index (the tie-break), and Σx, from which the centroid-x is Σx / count.
#[derive(Clone, Copy)]
pub(super) struct SideStat {
    count: u32,
    min_var: u32,
    sum_x: f64,
}

/// Is side `a` the LEFT child relative to side `b` under the orientation rule?
/// `min_var` is unique per side, so every rule is a strict total order.
pub(super) fn side_is_left(orient: OrientRule, a: &SideStat, b: &SideStat) -> bool {
    match orient {
        OrientRule::X => {
            let ca = a.sum_x / a.count as f64;
            let cb = b.sum_x / b.count as f64;
            ca < cb || (ca == cb && a.min_var < b.min_var)
        }
        OrientRule::Small => a.count < b.count || (a.count == b.count && a.min_var < b.min_var),
        OrientRule::Big => a.count > b.count || (a.count == b.count && a.min_var < b.min_var),
    }
}

pub(super) fn side_stat(pts: &[Vec<f64>], verts: &[u32]) -> SideStat {
    let mut min_var = u32::MAX;
    let mut sum_x = 0.0;
    for &v in verts {
        min_var = min_var.min(v);
        sum_x += pts[v as usize][0];
    }
    SideStat {
        count: verts.len() as u32,
        min_var,
        sum_x,
    }
}

/// O(n) scratch shared by the whole top-down build, written only for the vertices of
/// the subset currently being split. Grouped so the recursion carries one borrow
/// rather than three.
pub(super) struct SplitScratch {
    /// Membership flag for the subset currently being split.
    in_subset: Vec<bool>,
    /// Parent pointer within the subset tree, for the size-based root rules.
    parent: Vec<u32>,
    /// Subtree size within the subset tree, for the size-based root rules.
    size: Vec<u32>,
}

impl SplitScratch {
    fn new(n: usize) -> Self {
        SplitScratch {
            in_subset: vec![false; n],
            parent: vec![0u32; n],
            size: vec![0u32; n],
        }
    }
}

/// The spanning structure the top-down build splits: the selected tree edges and the
/// adjacency over them (`adj[v]` lists `(neighbour, edge index)`).
pub(super) struct SpanTree {
    edges: Vec<(f64, u32, u32)>,
    adj: Vec<Vec<(u32, u32)>>,
}

/// Top-down MST tree-ifier: build a spanning tree over the candidate edges (Kruskal,
/// i.e. single-linkage merge order), then recursively SPLIT it per [`RootRule`],
/// orienting each internal node per [`OrientRule`]. `root = merge` splits at the
/// longest edge, which reproduces the union-find merge hierarchy exactly;
/// `balance` and `hybrid` pick a different split edge. Any leftover forest — the
/// grid-kNN and co-occurrence candidate sets need not span — is folded with the
/// same rule.
pub(super) fn mst_tree(
    pts: &[Vec<f64>],
    cfg: &ForceConfig,
    inc: &Incidence,
    nodes: &mut VtreeArena,
) -> VtreeIdx {
    let n = pts.len();

    // 1. Candidate edges. `w=co` routes through the candidate graph at all sizes.
    let mut edges = match cfg.weight {
        WeightRule::Euclid => {
            if n <= PRIM_LIMIT {
                emst_prim(pts)
            } else {
                emst_grid_candidates(pts)
            }
        }
        WeightRule::Co => cooccurrence_candidates(pts, inc),
    };
    edges.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));

    // 2. Kruskal selects the spanning-tree edges (ascending order = merge order).
    // Each selected edge keeps its ORIGINAL (len, a, b) orientation so the `merge`
    // rule's max-(len, a, b) split matches the last merge exactly.
    let mut uf: Vec<u32> = (0..n as u32).collect();
    let mut span = SpanTree {
        edges: Vec::with_capacity(n.saturating_sub(1)),
        adj: vec![Vec::new(); n],
    };
    for (len, a, b) in edges {
        let ra = find(&mut uf, a);
        let rb = find(&mut uf, b);
        if ra == rb {
            continue;
        }
        uf[rb as usize] = ra;
        let eidx = span.edges.len() as u32;
        span.edges.push((len, a, b));
        span.adj[a as usize].push((b, eidx));
        span.adj[b as usize].push((a, eidx));
    }

    // 3. Group vertices by final component (grid-kNN / co can leave a forest).
    let mut comp_verts: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    for v in 0..n as u32 {
        comp_verts.entry(find(&mut uf, v)).or_default().push(v);
    }

    // 4. Build each component top-down. Scratch is O(n), touched only per subset.
    let mut scratch = SplitScratch::new(n);
    let mut comps: Vec<(VtreeIdx, SideStat)> = Vec::with_capacity(comp_verts.len());
    for verts in comp_verts.into_values() {
        comps.push(build_component(pts, cfg, &span, verts, nodes, &mut scratch));
    }

    // 5. Fold components into one root under the same orientation rule, accumulator
    // LEFT. A single component — always the case on the Prim path — returns directly
    // and adds no fold node.
    comps.sort_by(|x, y| {
        if side_is_left(cfg.orient, &x.1, &y.1) {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
    });
    let mut node = comps[0].0;
    for &(cidx, _) in &comps[1..] {
        let idx = nodes.internal(node, cidx);
        node = idx; // pre-sorted, so the accumulator stays LEFT
    }
    node
}

/// Union-find root of `x` with path halving.
pub(super) fn find(uf: &mut [u32], mut x: u32) -> u32 {
    while uf[x as usize] != x {
        uf[x as usize] = uf[uf[x as usize] as usize];
        x = uf[x as usize];
    }
    x
}

/// Build one connected component's subtree top-down, iteratively over an explicit
/// stack — safe for the O(depth) recursion a merge-order chain can reach. Returns
/// its root [`VtreeIdx`] and aggregate.
pub(super) fn build_component(
    pts: &[Vec<f64>],
    cfg: &ForceConfig,
    span: &SpanTree,
    verts0: Vec<u32>,
    nodes: &mut VtreeArena,
    scratch: &mut SplitScratch,
) -> (VtreeIdx, SideStat) {
    enum Task {
        Enter(Vec<u32>),
        Combine,
    }
    let mut task = vec![Task::Enter(verts0)];
    let mut done: Vec<(VtreeIdx, SideStat)> = Vec::new();
    while let Some(t) = task.pop() {
        match t {
            Task::Enter(verts) => {
                if verts.len() == 1 {
                    let v = verts[0];
                    let idx = nodes.leaf(VarId(v));
                    done.push((
                        idx,
                        SideStat {
                            count: 1,
                            min_var: v,
                            sum_x: pts[v as usize][0],
                        },
                    ));
                } else {
                    let (left, right) = split_subset(pts, cfg, span, &verts, scratch);
                    // Combine runs after both child subproblems resolve (LIFO):
                    // Enter(left) is pushed last, so its result lands below
                    // Enter(right)'s.
                    task.push(Task::Combine);
                    task.push(Task::Enter(right));
                    task.push(Task::Enter(left));
                }
            }
            Task::Combine => {
                let (ri, rs) = done.pop().unwrap();
                let (li, ls) = done.pop().unwrap();
                let idx = nodes.internal(li, ri);
                done.push((
                    idx,
                    SideStat {
                        count: ls.count + rs.count,
                        min_var: ls.min_var.min(rs.min_var),
                        sum_x: ls.sum_x + rs.sum_x,
                    },
                ));
            }
        }
    }
    done.pop().unwrap()
}

/// Pick a split edge for `verts` per [`RootRule`], partition the subtree into its two
/// sides, and return them oriented (LEFT, RIGHT) per [`OrientRule`]. O(subset).
pub(super) fn split_subset(
    pts: &[Vec<f64>],
    cfg: &ForceConfig,
    span: &SpanTree,
    verts: &[u32],
    scratch: &mut SplitScratch,
) -> (Vec<u32>, Vec<u32>) {
    let tree_edges = &span.edges;
    let adj = &span.adj;
    for &v in verts {
        scratch.in_subset[v as usize] = true;
    }
    // Subset edges, deduplicated by counting each once at its smaller endpoint.
    let mut sub_edges: Vec<u32> = Vec::with_capacity(verts.len().saturating_sub(1));
    for &v in verts {
        for &(u, eidx) in &adj[v as usize] {
            if scratch.in_subset[u as usize] && v < u {
                sub_edges.push(eidx);
            }
        }
    }

    let split_eidx = match cfg.root {
        RootRule::Merge => {
            // Longest edge = max by (len, a, b) = the last-merged edge.
            *sub_edges
                .iter()
                .max_by(|&&x, &&y| {
                    let (lx, ax, bx) = tree_edges[x as usize];
                    let (ly, ay, by) = tree_edges[y as usize];
                    lx.total_cmp(&ly).then(ax.cmp(&ay)).then(bx.cmp(&by))
                })
                .unwrap()
        }
        RootRule::Balance | RootRule::Hybrid => {
            // Subtree sizes: root the subset tree at verts[0], iterative DFS.
            let m = verts.len() as u32;
            let root0 = verts[0];
            let mut order: Vec<u32> = Vec::with_capacity(verts.len());
            let mut stack = vec![root0];
            scratch.parent[root0 as usize] = root0;
            while let Some(v) = stack.pop() {
                order.push(v);
                for &(u, _) in &adj[v as usize] {
                    if scratch.in_subset[u as usize] && u != scratch.parent[v as usize] {
                        scratch.parent[u as usize] = v;
                        stack.push(u);
                    }
                }
            }
            for &v in &order {
                scratch.size[v as usize] = 1;
            }
            for &v in order.iter().rev() {
                let pv = scratch.parent[v as usize];
                if pv != v {
                    scratch.size[pv as usize] += scratch.size[v as usize];
                }
            }
            // Cost of splitting at edge e: max(|A|, |B|), where |A| is the child-side
            // size.
            let parent = &scratch.parent;
            let size = &scratch.size;
            let max_side = |eidx: u32| -> u32 {
                let (_, a, b) = tree_edges[eidx as usize];
                let child = if parent[a as usize] == b { a } else { b };
                let sc = size[child as usize];
                sc.max(m - sc)
            };
            // Hybrid restricts to edges at least half the component's longest.
            let len_of = |eidx: u32| tree_edges[eidx as usize].0;
            let max_len = sub_edges.iter().map(|&e| len_of(e)).fold(0.0f64, f64::max);
            let threshold = 0.5 * max_len;
            // Tie: min max_side, then longer edge, then smaller endpoint (a, b) key.
            let endpoint_key = |eidx: u32| {
                let (_, a, b) = tree_edges[eidx as usize];
                (a.min(b), a.max(b))
            };
            *sub_edges
                .iter()
                .filter(|&&e| cfg.root == RootRule::Balance || len_of(e) >= threshold)
                .min_by(|&&x, &&y| {
                    max_side(x)
                        .cmp(&max_side(y))
                        .then(len_of(y).total_cmp(&len_of(x)))
                        .then(endpoint_key(x).cmp(&endpoint_key(y)))
                })
                .unwrap()
        }
    };

    // Partition: walk from the split edge's `a` endpoint without crossing the split
    // edge. Membership flags flip back to false as side A is collected, which also
    // clears the scratch for the next call.
    let (_, ea, _eb) = tree_edges[split_eidx as usize];
    let mut side_a: Vec<u32> = Vec::new();
    let mut stack = vec![ea];
    scratch.in_subset[ea as usize] = false;
    side_a.push(ea);
    while let Some(v) = stack.pop() {
        for &(u, eidx) in &adj[v as usize] {
            if eidx != split_eidx && scratch.in_subset[u as usize] {
                scratch.in_subset[u as usize] = false;
                side_a.push(u);
                stack.push(u);
            }
        }
    }
    let mut side_b: Vec<u32> = Vec::new();
    for &v in verts {
        if scratch.in_subset[v as usize] {
            scratch.in_subset[v as usize] = false;
            side_b.push(v);
        }
    }

    let stat_a = side_stat(pts, &side_a);
    let stat_b = side_stat(pts, &side_b);
    if side_is_left(cfg.orient, &stat_a, &stat_b) {
        (side_a, side_b)
    } else {
        (side_b, side_a)
    }
}

// ---------------------------------------------------------------------------
// Recursive principal-axis median cut
// ---------------------------------------------------------------------------

/// Recursive principal-axis median cut. Split each subset at the median of its
/// projection onto the top principal axis; LEFT is the smaller-projection half, ties
/// broken by (projection, variable index). A median split halves the subset, which
/// bounds recursion depth at O(log n) even on all-coincident points.
pub(super) fn cut_tree(
    pts: &[Vec<f64>],
    d: usize,
    subset: &[u32],
    nodes: &mut VtreeArena,
) -> VtreeIdx {
    if subset.len() == 1 {
        let idx = nodes.leaf(VarId(subset[0]));
        return idx;
    }
    let m = subset.len();
    let mf = m as f64;
    // Subset mean (per coordinate).
    let mut mean = vec![0.0f64; d];
    for &v in subset {
        for (x, q) in mean.iter_mut().zip(pts[v as usize].iter()) {
            *x += q;
        }
    }
    for x in mean.iter_mut() {
        *x /= mf;
    }
    // Subset covariance → top principal axis.
    let denom = (mf - 1.0).max(1.0);
    let mut covn = vec![vec![0.0f64; d]; d];
    for &v in subset {
        let q = &pts[v as usize];
        for (i, row) in covn.iter_mut().enumerate() {
            let qi = q[i] - mean[i];
            for (j, x) in row.iter_mut().enumerate() {
                *x += qi * (q[j] - mean[j]);
            }
        }
    }
    for row in covn.iter_mut() {
        for x in row.iter_mut() {
            *x /= denom;
        }
    }
    // The top principal axis is the largest-eigenvalue one, last in ascending order.
    let axes = principal_axes(&covn);
    let axis = axes.last().unwrap();
    // Stable order by (projection, variable index).
    let mut centered = vec![0.0f64; d];
    let mut keyed: Vec<(f64, u32)> = subset
        .iter()
        .map(|&v| {
            for (c, (q, m)) in centered
                .iter_mut()
                .zip(pts[v as usize].iter().zip(mean.iter()))
            {
                *c = q - m;
            }
            (dot(&centered, axis), v)
        })
        .collect();
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let ordered: Vec<u32> = keyed.into_iter().map(|(_, v)| v).collect();

    let half = m / 2; // both halves non-empty for m ≥ 2
    let left = cut_tree(pts, d, &ordered[..half], nodes);
    let right = cut_tree(pts, d, &ordered[half..], nodes);
    nodes.internal(left, right)
}
