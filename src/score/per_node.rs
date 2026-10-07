//! The per-node tables every score is a reduction of.
//!
//! One pass over the (vtree, formula) pair produces a vector indexed by vtree
//! node: which clauses meet at each node, how wide its context is inside and
//! out, how many clauses cross it, what its subtree holds. [`super`] reduces
//! them to the cost, [`super::tables`] answers a ranker's feature by node from
//! the same tables, and neither builds a table of its own.

use crate::cnf::{Clause, CnfFormula};
use crate::vtree::{VarId, Vtree, VtreeIdx};

/// What every table below reads before it reads a clause: where each node's
/// subtree lies in a preorder numbering of the tree, and the node where each
/// clause's variables meet (the LCA of its literals' leaves).
///
/// Built once per (vtree, formula) pair and handed to every table a score
/// reads, so each clause's meeting point is found once rather than once per
/// table.
pub(crate) struct Layout {
    /// Each node's preorder number.
    entry: Vec<u32>,
    /// One past the largest preorder number in each node's subtree: node `t`
    /// holds the node numbered `at` exactly when `entry[t] <= at < exit[t]`.
    exit: Vec<u32>,
    /// Each clause's meeting point, by clause index; `None` for a clause with
    /// no literals, which meets nowhere.
    clause_lca: Vec<Option<VtreeIdx>>,
}

impl Layout {
    pub(crate) fn new(vtree: &Vtree, formula: &CnfFormula) -> Layout {
        let (entry, exit) = subtree_intervals(vtree);
        let clause_lca = formula
            .clauses()
            .iter()
            .map(|clause| meeting_point(vtree, &entry, &exit, clause))
            .collect();
        Layout {
            entry,
            exit,
            clause_lca,
        }
    }

    /// Each node's preorder number and the end of its subtree's range, as
    /// [`subtree_intervals`] returns them.
    pub(super) fn intervals(&self) -> (&[u32], &[u32]) {
        (&self.entry, &self.exit)
    }

    /// Whether `node`'s subtree holds the node numbered `at`.
    fn holds(&self, node: VtreeIdx, at: u32) -> bool {
        self.entry[node.idx()] <= at && at < self.exit[node.idx()]
    }

    /// Call `f` with the meeting point of each non-empty clause, in clause
    /// order.
    ///
    /// The one scan every clause table below is a reduction of. A clause with
    /// no literals meets nowhere and is skipped, so the clauses reported here
    /// are the non-empty ones, in the order the formula lists them.
    fn for_each_clause_lca(&self, mut f: impl FnMut(usize, VtreeIdx)) {
        for (clause_idx, lca) in self.clause_lca.iter().enumerate() {
            if let Some(lca) = *lca {
                f(clause_idx, lca);
            }
        }
    }

    /// The clause load per node: how many non-empty clauses meet there.
    pub(super) fn loads(&self, vtree: &Vtree) -> Vec<u32> {
        let mut clause_at = vec![0u32; vtree.num_nodes()];
        self.for_each_clause_lca(|_, lca| clause_at[lca.idx()] += 1);
        clause_at
    }

    /// The clauses meeting at each node, by clause index in formula order.
    pub(super) fn members(&self, vtree: &Vtree) -> Vec<Vec<usize>> {
        let mut clauses_at = vec![Vec::new(); vtree.num_nodes()];
        self.for_each_clause_lca(|clause_idx, lca| {
            clauses_at[lca.idx()].push(clause_idx);
        });
        clauses_at
    }
}

/// The node where a clause's variables meet; `None` for the empty clause.
///
/// The leaves of a set meet where the first of them in preorder first reaches
/// the last, so this is one walk up from the first leaf, testing each node's
/// interval against the last.
fn meeting_point(vtree: &Vtree, entry: &[u32], exit: &[u32], clause: &Clause) -> Option<VtreeIdx> {
    let mut leaves = clause.literals.iter().map(|lit| vtree.leaf_of(lit.var));
    let first = leaves.next()?;
    let (mut low, mut high) = (first, entry[first.idx()]);
    for leaf in leaves {
        let at = entry[leaf.idx()];
        if at < entry[low.idx()] {
            low = leaf;
        }
        high = high.max(at);
    }
    let mut node = low;
    while exit[node.idx()] <= high {
        node = vtree
            .node(node)
            .parent()
            .expect("the root's subtree holds every leaf");
    }
    Some(node)
}

/// For each non-empty clause, increment the count at the vtree node where the
/// clause's variables meet. Returns a vector of length `vtree.num_nodes()` with
/// clause counts per node.
///
/// A node's "clause load" is this count: the two names are the same number, one
/// from the scoring vocabulary and one from the picture `--dot` draws.
pub(crate) fn clause_lca_counts(vtree: &Vtree, formula: &CnfFormula) -> Vec<u32> {
    Layout::new(vtree, formula).loads(vtree)
}

/// The clause load per node together with the node each non-empty clause
/// landed on, in clause order.
///
/// For a caller that has to go back from an overloaded node to the clauses
/// sitting on it, which the counts alone cannot answer.
pub(crate) fn clause_lca_nodes(vtree: &Vtree, formula: &CnfFormula) -> (Vec<VtreeIdx>, Vec<u32>) {
    let layout = Layout::new(vtree, formula);
    let mut per_clause = Vec::with_capacity(formula.clauses().len());
    let mut clause_at = vec![0u32; vtree.num_nodes()];
    layout.for_each_clause_lca(|_, lca| {
        per_clause.push(lca);
        clause_at[lca.idx()] += 1;
    });
    (per_clause, clause_at)
}

/// Shallowest (closest-to-root) clause-LCA per variable; `None` = the var never
/// crosses a node boundary (only appears in unit/empty clauses). Shared by the
/// all-var and `keep`-restricted context-width metrics — the crossing structure
/// is identical; only the per-node accumulation differs.
pub(super) fn clause_high_lca(
    vtree: &Vtree,
    formula: &CnfFormula,
    layout: &Layout,
) -> Vec<Option<VtreeIdx>> {
    let n_vars = vtree.num_vars() as usize;
    let mut high_lca: Vec<Option<VtreeIdx>> = vec![None; n_vars];
    for (clause, lca) in formula.clauses().iter().zip(&layout.clause_lca) {
        if clause.literals.len() < 2 {
            continue; // unit/empty clause crosses no node boundary
        }
        let lca = lca.expect("a clause with two literals has an LCA");
        let lpos = vtree.topo_pos(lca);
        for lit in &clause.literals {
            let vi = lit.var.idx();
            let replace = match high_lca[vi] {
                Some(cur) => lpos > vtree.topo_pos(cur),
                None => true,
            };
            if replace {
                high_lca[vi] = Some(lca);
            }
        }
    }
    high_lca
}

/// The context-width walk over a `high_lca` table the caller already has, which
/// is what lets `VtreeScores::compute` count both widths off ONE shared table.
pub(super) fn context_width_from_high_lca(
    vtree: &Vtree,
    high_lca: &[Option<VtreeIdx>],
    show: Option<&crate::cnf::ShowMask>,
) -> Vec<u32> {
    let mut ctx = vec![0u32; vtree.num_nodes()];
    for (vi, &lca) in high_lca.iter().enumerate() {
        if let Some(mask) = show
            && !mask.as_slice().get(vi).copied().unwrap_or(false)
        {
            continue;
        }
        if let Some(l) = lca {
            let leaf = vtree.leaf_of(VarId::from_idx(vi));
            let mut cur = vtree.node(leaf).parent();
            while let Some(node) = cur {
                if node == l {
                    break; // reached the clause LCA — stop before it
                }
                ctx[node.idx()] += 1;
                cur = vtree.node(node).parent();
            }
        }
    }

    ctx
}

/// Context width per vtree node: the number of *distinct* variables in
/// `subtree(t)` that also appear in a clause crossing `t`'s boundary (a clause
/// whose LCA is a strict ancestor of `t`) — the inside end of the separator at
/// `t`. Unlike the `clause_load_*` metrics (which only count clauses bucketed
/// at their LCA), this measures how many variables leak across each split.
/// `2^ctx[t]` is not a bound on the diagram at `t` (a single inside variable
/// under the clauses `a ∨ c` and `¬a ∨ d` already has three subfunctions), and
/// the peak alone is a rough predictor of compile size; [`super::vtree_cost`]
/// reads it together with the outside end.
///
/// A variable `v` crosses node `t` iff `t` lies strictly between `leaf(v)` and
/// the *shallowest* clause-LCA among clauses containing `v` (shallowest = the
/// widest-spanning clause, so it gives the longest crossing segment). We find
/// that shallowest LCA per variable (closest to root = largest `topo_pos`),
/// then walk `leaf → ancestor`, incrementing each node on the segment.
///
/// Returns the per-node context-width array, length `vtree.num_nodes()`.
/// Cost: O(|clause literals| × vtree_depth).
///
/// `show` restricts the count to the shown variables, which is the binding cost
/// under PROJECTED counting: a hidden variable crossing a cut is ∃-forgotten
/// when its scope completes, collapsing that part of the frontier, whereas a
/// shown variable persists to the root. So a vtree whose wide cuts are dominated
/// by hidden variables compiles cheaply under ∃-forget even though its all-var
/// peak is large — and conversely a low all-var peak can hide a show-heavy
/// separator that blows up. The crossing structure is computed over ALL clauses
/// either way; only the per-node accumulation is filtered.
pub(crate) fn vtree_context_width_per_node(
    vtree: &Vtree,
    formula: &CnfFormula,
    show: Option<&crate::cnf::ShowMask>,
) -> Vec<u32> {
    let high_lca = clause_high_lca(vtree, formula, &Layout::new(vtree, formula));
    context_width_from_high_lca(vtree, &high_lca, show)
}

/// Outside context width per vtree node: the number of *distinct* variables
/// OUTSIDE `subtree(t)` that share a clause with a variable inside it — the
/// outside end of the same clauses [`vtree_context_width_per_node`] counts the
/// inside end of. The subfunctions the compiler can form at `t` are indexed by
/// an assignment to these variables.
///
pub(super) struct OutsideContextTables {
    pub(super) widths: Vec<u32>,
    pub(super) sibling_overlap: Vec<u32>,
}

/// Outside-context width per node, and each node's overlap with its sibling.
///
/// Per variable `v`: every node that contains a clause-mate of `v` but not `v`
/// itself, which is every node strictly below `lca(v, u)` on the path up from
/// `leaf(u)`, for each mate `u`. A stamp per variable keeps a node counted
/// once for `v` however many mates reach it and ends each walk at the first
/// node already stamped; a walk also ends at the first node containing `v`,
/// which the subtree intervals answer without touching `v`'s own path to the
/// root. The work is therefore the number of (node, variable) pairs marked plus
/// one pass over every clause per variable it contains, whatever the depth.
///
/// Both arrays have length `vtree.num_nodes()`. A leaf's width counts the
/// mates of its own variable.
pub(super) fn outside_context_tables(
    vtree: &Vtree,
    formula: &CnfFormula,
    layout: &Layout,
) -> OutsideContextTables {
    let n_vars = vtree.num_vars() as usize;
    let (pos, neg) = crate::cnf::occ::occurrence_lists(formula.clauses(), n_vars);
    let nn = vtree.num_nodes();
    let mut ctx_out = vec![0u32; nn];
    let mut sibling_overlap = vec![0u32; nn];
    // `stamp[t] == v` marks node `t` as counted for variable `v` by a mate's
    // walk, so the next mate to reach it stops there.
    let mut stamp: Vec<u32> = vec![u32::MAX; nn];
    // Unlike `stamp`, this marks only nodes where `v` was outside. It lets a
    // parent count variables outside both children without retaining one set
    // per node.
    let mut outside_stamp: Vec<u32> = vec![u32::MAX; nn];
    for (v, (in_pos, in_neg)) in pos.iter().zip(&neg).enumerate() {
        if in_pos.is_empty() && in_neg.is_empty() {
            continue;
        }
        let v_id = v as u32;
        // A node contains `v` exactly when its interval holds `v`'s leaf.
        let at = layout.entry[vtree.leaf_of(VarId::from_idx(v)).idx()];
        for &ci in in_pos.iter().chain(in_neg) {
            for lit in &formula.clauses()[ci].literals {
                if lit.var.idx() == v {
                    continue;
                }
                let mut cur = Some(vtree.leaf_of(lit.var));
                while let Some(node) = cur {
                    let i = node.idx();
                    if stamp[i] == v_id || layout.holds(node, at) {
                        break;
                    }
                    stamp[node.idx()] = v_id;
                    ctx_out[node.idx()] += 1;
                    outside_stamp[node.idx()] = v_id;
                    if let Some(parent) = vtree.node(node).parent() {
                        let (left, right) = vtree.children(parent);
                        let sibling = if node == left { right } else { left };
                        if outside_stamp[sibling.idx()] == v_id {
                            sibling_overlap[parent.idx()] += 1;
                        }
                    }
                    cur = vtree.node(node).parent();
                }
            }
        }
    }
    OutsideContextTables {
        widths: ctx_out,
        sibling_overlap,
    }
}

/// Crossing clauses per vtree node: the number of clauses with a variable
/// inside `subtree(t)` and one outside it, i.e. with at least two literals and
/// an LCA strictly above `t`. The third count the separator at `t` can be
/// measured by, beside the two variable counts.
///
/// Per clause: its LCA is stamped, then each literal's leaf walks up until it
/// reaches a node already stamped for this clause, counting the nodes it
/// passes. The nodes counted are exactly the union of the leaf-to-LCA paths
/// below the LCA, each once.
///
/// Returns the per-node array, length `vtree.num_nodes()`, read off the
/// clause meeting points in `layout`.
pub(super) fn crossing_clauses(vtree: &Vtree, formula: &CnfFormula, layout: &Layout) -> Vec<u32> {
    let nn = vtree.num_nodes();
    let mut cross = vec![0u32; nn];
    let mut stamp: Vec<usize> = vec![usize::MAX; nn];
    for (ci, (clause, lca)) in formula.clauses().iter().zip(&layout.clause_lca).enumerate() {
        if clause.literals.len() < 2 {
            continue;
        }
        let lca = lca.expect("a clause with two literals has an LCA");
        stamp[lca.idx()] = ci;
        for lit in &clause.literals {
            let mut cur = vtree.leaf_of(lit.var);
            while stamp[cur.idx()] != ci {
                stamp[cur.idx()] = ci;
                cross[cur.idx()] += 1;
                cur = vtree
                    .node(cur)
                    .parent()
                    .expect("a node below the clause LCA has a parent");
            }
        }
    }
    cross
}

/// What lies below each vtree node, over a precomputed clause-LCA count table.
pub(super) struct SubtreeTables {
    /// Clauses bucketed anywhere in the subtree, the node's own load included.
    pub(super) clauses: Vec<u64>,
    /// Leaves in the subtree; 1 at a leaf.
    pub(super) leaves: Vec<u32>,
    /// Edges from the node down to its deepest leaf; 0 at a leaf.
    pub(super) height: Vec<u32>,
}

/// Accumulate [`SubtreeTables`] in one bottom-up pass.
pub(super) fn subtree_tables(vtree: &Vtree, clause_at: &[u32]) -> SubtreeTables {
    let mut clauses = vec![0u64; vtree.num_nodes()];
    let mut leaves = vec![0u32; vtree.num_nodes()];
    let mut height = vec![0u32; vtree.num_nodes()];
    for t in vtree.bottomup() {
        let i = t.idx();
        if vtree.node(t).is_leaf() {
            clauses[i] = u64::from(clause_at[i]);
            leaves[i] = 1;
            continue;
        }
        let (left, right) = vtree.children(t);
        clauses[i] = u64::from(clause_at[i]) + clauses[left.idx()] + clauses[right.idx()];
        leaves[i] = leaves[left.idx()] + leaves[right.idx()];
        height[i] = 1 + height[left.idx()].max(height[right.idx()]);
    }
    SubtreeTables {
        clauses,
        leaves,
        height,
    }
}

/// Edges from the root to each node, indexed by node; the root is 0.
///
/// One top-down pass over the maintained topological order, which visits every
/// parent before its children when read backwards.
pub(super) fn node_depths(vtree: &Vtree) -> Vec<u32> {
    let mut depth = vec![0u32; vtree.num_nodes()];
    for node in vtree.bottomup().rev() {
        if !vtree.node(node).is_leaf() {
            let (left, right) = vtree.children(node);
            depth[left.idx()] = depth[node.idx()] + 1;
            depth[right.idx()] = depth[node.idx()] + 1;
        }
    }
    depth
}

pub(super) fn subtree_intervals(vtree: &Vtree) -> (Vec<u32>, Vec<u32>) {
    let mut entry = vec![0u32; vtree.num_nodes()];
    let mut exit = vec![0u32; vtree.num_nodes()];
    let mut next = 0u32;
    let mut stack = vec![(vtree.root(), false)];
    while let Some((node, leaving)) = stack.pop() {
        if leaving {
            exit[node.idx()] = next;
            continue;
        }
        entry[node.idx()] = next;
        next += 1;
        stack.push((node, true));
        if !vtree.node(node).is_leaf() {
            let (left, right) = vtree.children(node);
            stack.push((right, false));
            stack.push((left, false));
        }
    }
    (entry, exit)
}

/// What a load table says about the nodes carrying a load.
pub(crate) struct LoadStats {
    /// Mean load over the counted nodes, `0.0` when none is counted.
    pub(crate) mean: f64,
    /// Sample standard deviation of that load, `0.0` below two counted nodes.
    pub(crate) stddev: f64,
    /// How many nodes were counted.
    pub(crate) count: usize,
}

/// Summarize a per-node load table over the loaded nodes `keep` admits.
///
/// A node with no load is never counted: it is a node no clause chose, not a
/// node that carries an unusually light one, so counting it would pull the mean
/// toward zero and report a spread that is mostly the empty tree. `keep` narrows
/// that further for a caller reading only part of the tree.
pub(crate) fn load_stats(loads: &[u32], keep: impl Fn(VtreeIdx) -> bool) -> LoadStats {
    let mut sum: f64 = 0.0;
    let mut sum_sq: f64 = 0.0;
    let mut count: usize = 0;
    for (idx, &load) in loads.iter().enumerate() {
        if load > 0 && keep(VtreeIdx(idx as u32)) {
            sum += load as f64;
            sum_sq += (load as f64) * (load as f64);
            count += 1;
        }
    }

    if count == 0 {
        return LoadStats {
            mean: 0.0,
            stddev: 0.0,
            count: 0,
        };
    }

    let mean = sum / count as f64;
    let stddev = if count == 1 {
        0.0
    } else {
        ((sum_sq - sum * mean) / (count - 1) as f64).max(0.0).sqrt()
    };
    LoadStats {
        mean,
        stddev,
        count,
    }
}

/// Maximum clause load over a precomputed clause-LCA count table: the largest
/// number of clauses whose LCA is any single vtree node.
pub(super) fn max_from_counts(clause_at: &[u32]) -> u32 {
    clause_at.iter().copied().max().unwrap_or(0)
}

/// Standard deviation of clause loads across vtree nodes, over a precomputed
/// clause-LCA count table: for each node, the "clause load" is the number of
/// clauses whose LCA is that node. The canonical arithmetic, so
/// `VtreeScores::compute` and the pin that checks it against a separately
/// spelled-out computation cannot drift apart.
pub(super) fn stddev_from_counts(clause_at: &[u32]) -> f64 {
    load_stats(clause_at, |_| true).stddev
}
