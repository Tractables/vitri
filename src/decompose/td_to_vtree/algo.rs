//! The tree-decomposition → vtree conversion itself: root each component of the
//! decomposition, assign every variable to exactly one bag, then build the
//! vtree bottom-up over the bags in reverse BFS order, handing each bag's
//! already-built child subtrees and its local leaves to a combiner.
//!
//! Every construction in this crate that starts from a decomposition ends up
//! here; the root/ordering sweep in `portfolio` drives it once per candidate.
//! The tree comes back paired with the [`BagMetadata`] of the same run.
//!
//! One variable, one bag, one leaf. A variable occurring in several bags is
//! assigned to a single one, so a bag's assigned-variable set and its bag-vertex
//! set are different things — separator-aware combiners need the second, which
//! is why it is tracked separately. Variables in no bag at all become top-level
//! leaves beside the component roots.

use crate::cnf::CnfFormula;
use crate::vtree::{VarId, Vtree, VtreeArena, VtreeIdx};

use super::super::TreeDecomposition;
use super::super::td_ops::{RootedForest, rooted_forest};
use super::super::td_parse::{cooccurrence_counts, primal_adjacency};
use super::combiners::{
    combine_boundary_adjacent, combine_clause_aware, combine_hypergraph_bisect,
    combine_into_balanced, combine_left_deep, combine_td_edge_aligned,
};
use super::config::{BagAssignment, ItemOrdering, TdRootStrategy, TdToVtreeConfig, VarOrderInBag};
use super::meta::BagMetadata;
use super::portfolio::TdConversionMeta;

/// Where the conversion roots each connected component of the tree
/// decomposition. The two entry points below differ only in this choice; every
/// other decision is a [`TdToVtreeConfig`] field.
pub(super) enum RootChoice {
    /// One root per component, picked by the strategy.
    ByStrategy(TdRootStrategy),
    /// Root the decomposition at this specific bag. Components that bag does not
    /// reach keep their lowest-index bag as root.
    Bag(usize),
}

/// What is being converted, as opposed to how: the decomposition, the variable
/// space it is converted into, the formula behind it when the caller has one,
/// and the effort the conversion may spend.
///
/// These four are fixed for a whole conversion — a sweep that converts one
/// decomposition a dozen ways varies only the root and the ordering, and
/// carrying the fixed part as one value is what makes that visible at each of
/// its call sites.
#[derive(Clone, Copy)]
pub(crate) struct ConversionInput<'a> {
    /// The decomposition to convert.
    pub td: &'a TreeDecomposition,
    /// The variable space of the vtree to build: its leaves cover `0..num_vars`.
    pub num_vars: u32,
    /// The formula the decomposition describes, when the caller still holds it.
    /// `None` leaves the clause-aware heuristics nothing to order by, and they
    /// fall back to the plain balanced combiner.
    pub formula: Option<&'a CnfFormula>,
    /// Effort multiplier for [`ItemOrdering::HypergraphBisect`], the one item
    /// ordering that spends a scalable budget.
    pub effort_scale: f64,
}

/// THE tree-decomposition→vtree conversion; every construction in this crate
/// that starts from a tree decomposition ends up here.
///
/// Returns the vtree together with the [`BagMetadata`] describing which bag each
/// variable was assigned to, paired in one value so a winning candidate's
/// metadata can never be mismatched with a different candidate's vtree.
pub(super) fn convert(
    input: ConversionInput<'_>,
    root: RootChoice,
    config: &TdToVtreeConfig,
) -> (Vtree, BagMetadata) {
    let ConversionInput {
        td,
        num_vars,
        formula,
        effort_scale,
    } = input;
    let n = td.bags.len();

    // `chosen.chain(0..n)`: the named root(s) go first and claim their
    // component; 0..n then supplies a root for every component they didn't
    // reach.
    let chosen = chosen_roots(td, root, n);
    let RootedForest {
        order,
        parent: parent_td,
        depth,
        component_roots,
    } = rooted_forest(&td.adj, chosen.iter().copied().chain(0..n));

    let var_bag = assign_var_bags(td, num_vars, &order, &depth, config.bag_assignment, formula);

    // Bag assignment is final here — build the TD metadata from the very arrays
    // the conversion just produced (no second assignment pass anywhere).
    let meta = BagMetadata::from_assignment(num_vars, &var_bag, &order, n, td.treewidth());

    // vars_at[t] = variables assigned to TD node t.
    let mut vars_at: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut in_any_bag = vec![false; num_vars as usize];
    for v in 0..num_vars as usize {
        if var_bag[v] != usize::MAX {
            vars_at[var_bag[v]].push(v as u32);
            in_any_bag[v] = true;
        }
    }

    // --- Step 2: clause-aware variable ordering within bags ----------------
    if config.var_order == VarOrderInBag::ClauseAffinity
        && let Some(formula) = formula
    {
        apply_clause_affinity_ordering(&mut vars_at, formula, num_vars);
    }

    // --- Step 3: build the vtree bottom-up ---------------------------------
    // Primal adjacency for the TD-edge-aligned combiner's clause-partner routing
    // (built once, only when that combiner is selected and a formula is present).
    let td_edge_primal_adj: Vec<Vec<u32>> = if config.item_ordering == ItemOrdering::TdEdgeAligned {
        formula
            .map(|f| primal_adjacency(f, num_vars))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut nodes = VtreeArena::new();
    let mut td_vtree_idx: Vec<Option<VtreeIdx>> = vec![None; n];
    // Variables in each TD subtree. Doubles as the size key for the
    // size-ordering strategies: a subtree's variable count IS its vtree
    // subtree's leaf count, which is what "by size" means here.
    let mut td_vars: Vec<Vec<u32>> = vec![Vec::new(); n];
    // The union of BAG vertices in each TD subtree (all vars *appearing* in the
    // subtree, not just those *assigned* leaves there). Only maintained for
    // TdEdgeAligned, which needs it to detect which branches reference a lifted
    // separator (a shared var assigned to an ancestor is absent from its
    // branches' assigned-var sets but present in their bag-vertex sets).
    let track_bag_vars = config.item_ordering == ItemOrdering::TdEdgeAligned;
    let mut td_bag_vars: Vec<Vec<u32>> = vec![Vec::new(); n];

    for &t in order.iter().rev() {
        // (vtree index, subtree variable count) per child subtree.
        let mut child_items: Vec<(VtreeIdx, usize)> = Vec::new();
        let mut child_var_sets: Vec<Vec<u32>> = Vec::new();
        // Parallel bag-vertex sets for each child subtree (TdEdgeAligned only).
        let mut child_bag_var_sets: Vec<Vec<u32>> = Vec::new();
        for &nb in &td.adj[t] {
            if nb != parent_td[t]
                && let Some(child_idx) = td_vtree_idx[nb]
            {
                child_items.push((child_idx, td_vars[nb].len()));
                child_var_sets.push(td_vars[nb].clone());
                if track_bag_vars {
                    child_bag_var_sets.push(td_bag_vars[nb].clone());
                }
            }
        }

        // Leaf nodes for the variables assigned to this TD node.
        let mut var_items: Vec<VtreeIdx> = Vec::new();
        for &v in &vars_at[t] {
            let idx = nodes.leaf(VarId(v));
            var_items.push(idx);
        }

        let items = assemble_items(config.item_ordering, &mut child_items, &var_items);

        // This subtree's variables: the children's, plus the ones assigned here.
        // The assignment gives each variable exactly one bag, so the parts are
        // disjoint and the length is the subtree's variable count.
        let mut all_vars: Vec<u32> = Vec::new();
        for cv in &child_var_sets {
            all_vars.extend_from_slice(cv);
        }
        all_vars.extend_from_slice(&vars_at[t]);
        td_vars[t] = all_vars;

        // Bag-vertex union of this subtree = this bag's vertices ∪ children's.
        if track_bag_vars {
            let mut bag_union: Vec<u32> = Vec::new();
            for &v in &td.bags[t].vertices {
                if (v as usize) < num_vars as usize {
                    bag_union.push(v);
                }
            }
            for cbv in &child_bag_var_sets {
                bag_union.extend_from_slice(cbv);
            }
            bag_union.sort_unstable();
            bag_union.dedup();
            td_bag_vars[t] = bag_union;
        }

        td_vtree_idx[t] = if items.is_empty() {
            None
        } else {
            let bag = BagItems {
                items: &items,
                child_items: &child_items,
                child_var_sets: &child_var_sets,
                child_bag_var_sets: &child_bag_var_sets,
                var_items: &var_items,
                vars_here: &vars_at[t],
                parent_bag: (parent_td[t] != usize::MAX)
                    .then(|| td.bags[parent_td[t]].vertices.as_slice()),
            };
            Some(combine_bag(
                &bag,
                config,
                formula,
                effort_scale,
                &td_edge_primal_adj,
                &mut nodes,
            ))
        };
    }

    // Top-level vtree roots: one per TD component, then the isolated variables.
    let mut top_items: Vec<VtreeIdx> = Vec::new();
    for &cr in &component_roots {
        if let Some(root_idx) = td_vtree_idx[cr] {
            top_items.push(root_idx);
        }
    }
    for (v, &bagged) in in_any_bag.iter().enumerate() {
        if !bagged {
            let idx = nodes.leaf(VarId(v as u32));
            top_items.push(idx);
        }
    }
    assert!(!top_items.is_empty(), "td_to_vtree: no variables found");

    let root = combine_into_balanced(&top_items, &mut nodes);
    (Vtree::from_nodes(nodes.into_nodes(), root, num_vars), meta)
}

/// The bag each component of `td` is rooted at, as `root` asks for it: the one
/// bag it names, or one per component chosen by its strategy.
fn chosen_roots(td: &TreeDecomposition, root: RootChoice, n: usize) -> Vec<usize> {
    match root {
        RootChoice::Bag(b) => vec![b],
        RootChoice::ByStrategy(strategy) => {
            // Lowest-index bag of each component, in component-discovery order.
            let roots = rooted_forest(&td.adj, 0..n).component_roots;
            match strategy {
                TdRootStrategy::FirstBag => roots,
                TdRootStrategy::Centroid => {
                    roots.iter().map(|&cr| find_centroid(cr, &td.adj)).collect()
                }
            }
        }
    }
}

/// The bag each variable is assigned to, `usize::MAX` for one no bag holds.
///
/// Exactly one bag per variable, which is what makes the subtree variable sets
/// the build then accumulates disjoint. `order` and `depth` come from the
/// rooted forest; `formula` breaks ties by clause co-occurrence where it can.
fn assign_var_bags(
    td: &TreeDecomposition,
    num_vars: u32,
    order: &[usize],
    depth: &[usize],
    assignment: BagAssignment,
    formula: Option<&CnfFormula>,
) -> Vec<usize> {
    let mut var_bag = vec![usize::MAX; num_vars as usize];
    match assignment {
        BagAssignment::Deepest => {
            // Pass 1: BFS order is shallowest-first, so the last write wins and
            // each variable lands in its deepest bag.
            let mut var_max_depth = vec![0usize; num_vars as usize];
            for &bag_idx in order {
                for &v in &td.bags[bag_idx].vertices {
                    if (v as usize) < num_vars as usize {
                        var_bag[v as usize] = bag_idx;
                        var_max_depth[v as usize] = depth[bag_idx];
                    }
                }
            }
            // Pass 2 (co-occurrence tie-break): among equal-depth bags, prefer
            // the one sharing the most clauses with the variable.
            if let Some(formula) = formula {
                apply_cooc_tiebreak(
                    &BagWalk { td, order, depth },
                    formula,
                    num_vars,
                    &mut var_bag,
                    &var_max_depth,
                );
            }
        }
        BagAssignment::Shallowest => {
            for bag in &td.bags {
                for &v in &bag.vertices {
                    if (v as usize) < num_vars as usize {
                        let cur = var_bag[v as usize];
                        if cur == usize::MAX || depth[bag.id] < depth[cur] {
                            var_bag[v as usize] = bag.id;
                        }
                    }
                }
            }
        }
    }
    var_bag
}

/// The items of one TD node, in the order `ordering` asks for.
///
/// `ClauseSplit`, `HypergraphBisect`, `LeftDeep`, `BoundaryAdjacent` and
/// `TdEdgeAligned` take the plain children-then-leaves order here — their own
/// combiners reorder later from the boundary/clause data. Sorting is applied to
/// `child_items` itself, since the combiners read that order too.
fn assemble_items(
    ordering: ItemOrdering,
    child_items: &mut [(VtreeIdx, usize)],
    var_items: &[VtreeIdx],
) -> Vec<VtreeIdx> {
    fn children_then_vars(
        child_items: &[(VtreeIdx, usize)],
        var_items: &[VtreeIdx],
    ) -> Vec<VtreeIdx> {
        let mut items: Vec<VtreeIdx> = child_items.iter().map(|(idx, _)| *idx).collect();
        items.extend_from_slice(var_items);
        items
    }

    match ordering {
        ItemOrdering::VariablesFirst => {
            let mut items = var_items.to_vec();
            items.extend(child_items.iter().map(|(idx, _)| *idx));
            items
        }
        ItemOrdering::Reversed => {
            let mut items: Vec<VtreeIdx> = child_items.iter().rev().map(|(idx, _)| *idx).collect();
            items.extend(var_items.iter().rev());
            items
        }
        ItemOrdering::ChildrenBySize => {
            child_items.sort_by_key(|(_, size)| *size);
            children_then_vars(child_items, var_items)
        }
        ItemOrdering::LargestFirst => {
            child_items.sort_by_key(|(_, size)| std::cmp::Reverse(*size));
            children_then_vars(child_items, var_items)
        }
        ItemOrdering::ChildrenFirst
        | ItemOrdering::ClauseSplit
        | ItemOrdering::HypergraphBisect
        | ItemOrdering::LeftDeep
        | ItemOrdering::BoundaryAdjacent
        | ItemOrdering::TdEdgeAligned => children_then_vars(child_items, var_items),
    }
}

/// One TD node's pieces, as the combiners read them.
struct BagItems<'a> {
    /// What to combine, in the order [`assemble_items`] produced.
    items: &'a [VtreeIdx],
    /// Each child subtree's vtree root and variable count, in the order
    /// [`assemble_items`] left them.
    child_items: &'a [(VtreeIdx, usize)],
    /// Each child subtree's variables, in TD adjacency order.
    child_var_sets: &'a [Vec<u32>],
    /// Each child subtree's bag-vertex union, in the same order. Empty unless
    /// the TD-edge-aligned combiner is running — the only one that reads it.
    child_bag_var_sets: &'a [Vec<u32>],
    /// One leaf per variable assigned to this bag.
    var_items: &'a [VtreeIdx],
    /// The variables those leaves carry, in the same order.
    vars_here: &'a [u32],
    /// The parent bag's vertices, or `None` at a component root.
    parent_bag: Option<&'a [u32]>,
}

/// Combine one TD node's items into a single vtree subtree, by the rule its
/// item ordering names. A combiner that needs clause structure falls back to
/// the plain balanced combine without a formula.
fn combine_bag(
    bag: &BagItems<'_>,
    config: &TdToVtreeConfig,
    formula: Option<&CnfFormula>,
    effort_scale: f64,
    td_edge_primal_adj: &[Vec<u32>],
    nodes: &mut VtreeArena,
) -> VtreeIdx {
    let items = bag.items;
    // Per-item variable sets, in `items` order, for the two combiners that
    // partition by clause structure. Children first, then one set per leaf.
    let item_vars_list = || -> Vec<Vec<u32>> {
        let mut list: Vec<Vec<u32>> = bag.child_var_sets.to_vec();
        for &v in bag.vars_here {
            list.push(vec![v]);
        }
        list
    };

    match config.item_ordering {
        ItemOrdering::ClauseSplit => match formula {
            Some(formula) => {
                let list = item_vars_list();
                debug_assert_eq!(
                    list.len(),
                    items.len(),
                    "item_vars_list len {} != items len {} (children={}, vars={})",
                    list.len(),
                    items.len(),
                    bag.child_var_sets.len(),
                    bag.vars_here.len()
                );
                combine_clause_aware(items, &list, formula, nodes)
            }
            None => combine_into_balanced(items, nodes),
        },
        ItemOrdering::HypergraphBisect => match formula {
            Some(formula) => {
                combine_hypergraph_bisect(items, &item_vars_list(), formula, effort_scale, nodes)
            }
            None => combine_into_balanced(items, nodes),
        },
        ItemOrdering::LeftDeep => combine_left_deep(items, nodes),
        ItemOrdering::BoundaryAdjacent => {
            // Interior/boundary split per `ItemOrdering::BoundaryAdjacent`.
            let is_boundary = |v: u32| -> bool { bag.parent_bag.is_some_and(|pb| pb.contains(&v)) };
            let mut interior: Vec<VtreeIdx> = bag.child_items.iter().map(|(idx, _)| *idx).collect();
            let mut boundary: Vec<VtreeIdx> = Vec::new();
            debug_assert_eq!(bag.var_items.len(), bag.vars_here.len());
            for (i, &leaf_idx) in bag.var_items.iter().enumerate() {
                if is_boundary(bag.vars_here[i]) {
                    boundary.push(leaf_idx);
                } else {
                    interior.push(leaf_idx);
                }
            }
            combine_boundary_adjacent(&interior, &boundary, nodes)
        }
        ItemOrdering::TdEdgeAligned if formula.is_some() => {
            let child_idxs: Vec<VtreeIdx> = bag.child_items.iter().map(|(idx, _)| *idx).collect();
            combine_td_edge_aligned(
                &child_idxs,
                bag.child_bag_var_sets,
                bag.var_items,
                bag.vars_here,
                td_edge_primal_adj,
                nodes,
            )
        }
        _ => combine_into_balanced(items, nodes),
    }
}

/// TD → vtree with every knob set explicitly, rooting each component by the
/// config's [`TdRootStrategy`], returning the run's [`BagMetadata`] beside the
/// tree.
///
/// The one implementation of the conversion; the public
/// [`td_to_vtree_configured`](super::td_to_vtree_configured) is this at the
/// baseline effort.
pub(crate) fn td_to_vtree_configured_traced(
    input: ConversionInput<'_>,
    config: &TdToVtreeConfig,
) -> (Vtree, TdConversionMeta) {
    let (vtree, meta) = convert(input, RootChoice::ByStrategy(config.root_strategy), config);
    let info = TdConversionMeta {
        meta: Some(std::sync::Arc::new(meta)),
    };
    (vtree, info)
}

/// Find the centroid of a tree rooted at `start`. The centroid is the node
/// that minimizes the maximum subtree size when the tree is rooted at it.
pub(super) fn find_centroid(start: usize, adj: &[Vec<usize>]) -> usize {
    // Precondition: a non-empty decomposition containing `start` — an empty
    // `adj` panics `visited[start]` below with index-out-of-bounds.
    debug_assert!(
        !adj.is_empty(),
        "find_centroid requires a non-empty decomposition"
    );
    let RootedForest { order, parent, .. } = rooted_forest(adj, [start]);

    let component_size = order.len();
    if component_size <= 2 {
        return start;
    }

    let mut subtree_size = vec![1usize; adj.len()];
    for &t in order.iter().rev() {
        if parent[t] != usize::MAX {
            subtree_size[parent[t]] += subtree_size[t];
        }
    }

    // Centroid: the node where max(subtree_child_sizes, component_size - subtree_size[node])
    // is minimized.
    let mut best_node = start;
    let mut best_max = component_size;
    for &t in &order {
        let mut max_part = component_size - subtree_size[t]; // "upward" partition
        for &nb in &adj[t] {
            // Every neighbour of a walked bag was walked with it, so a bag
            // other than the parent is a child.
            if nb != parent[t] {
                max_part = max_part.max(subtree_size[nb]);
            }
        }
        if max_part < best_max {
            best_max = max_part;
            best_node = t;
        }
    }
    best_node
}

/// A tree decomposition together with the BFS walk over its bags: the visit
/// order, and the depth the walk reached each bag at.
struct BagWalk<'a> {
    td: &'a TreeDecomposition,
    order: &'a [usize],
    depth: &'a [usize],
}

/// Co-occurrence tiebreak pass: for each variable, re-assign it to whichever
/// equal-depth bag holds the most of its primal-graph neighbours.
fn apply_cooc_tiebreak(
    walk: &BagWalk<'_>,
    formula: &CnfFormula,
    num_vars: u32,
    var_bag: &mut [usize],
    var_max_depth: &[usize],
) {
    let nv = num_vars as usize;
    let primal_adj: Vec<Vec<u32>> = primal_adjacency(formula, num_vars);

    // Scoring scratch: per-variable best in-bag co-occurrence count.
    let mut best_score_u: Vec<u32> = vec![0u32; nv];
    // Bool array marking which variables are in the current bag.
    let mut in_bag = vec![false; nv];

    for &bag_idx in walk.order {
        let d = walk.depth[bag_idx];
        let bag_vars: Vec<u32> = walk.td.bags[bag_idx]
            .vertices
            .iter()
            .copied()
            .filter(|&v| (v as usize) < nv)
            .collect();
        if bag_vars.is_empty() {
            continue;
        }
        for &v in &bag_vars {
            in_bag[v as usize] = true;
        }
        for &v in &bag_vars {
            let vi = v as usize;
            if d == var_max_depth[vi] {
                let score: u32 = primal_adj[vi]
                    .iter()
                    .filter(|&&u| in_bag[u as usize])
                    .count() as u32;
                // ">=" keeps BFS-last behavior on exact ties (last-write wins).
                if score >= best_score_u[vi] {
                    best_score_u[vi] = score;
                    var_bag[vi] = bag_idx;
                }
            }
        }
        for &v in &bag_vars {
            in_bag[v as usize] = false;
        }
    }
}

/// Greedy nearest-neighbor clause-affinity reordering of variables within
/// each bag. Variables that co-occur in more clauses are placed adjacently.
fn apply_clause_affinity_ordering(vars_at: &mut [Vec<u32>], formula: &CnfFormula, num_vars: u32) {
    let cooccur = cooccurrence_counts(formula, num_vars);

    for bag_vars in vars_at.iter_mut() {
        if bag_vars.len() <= 2 {
            continue;
        }
        let mut remaining: Vec<bool> = vec![true; bag_vars.len()];
        let mut ordered = Vec::with_capacity(bag_vars.len());

        let start_idx = (0..bag_vars.len())
            .max_by_key(|&i| {
                let v = bag_vars[i] as usize;
                let total: u32 = cooccur[v]
                    .iter()
                    .filter(|(u, _)| bag_vars.contains(u))
                    .map(|(_, c)| c)
                    .sum();
                total
            })
            .unwrap();
        remaining[start_idx] = false;
        ordered.push(bag_vars[start_idx]);

        while ordered.len() < bag_vars.len() {
            let last = *ordered.last().unwrap() as usize;
            let best = (0..bag_vars.len())
                .filter(|&i| remaining[i])
                .max_by_key(|&i| {
                    let v = bag_vars[i];
                    cooccur[last]
                        .iter()
                        .find(|(u, _)| *u == v)
                        .map(|(_, c)| *c)
                        .unwrap_or(0)
                })
                .unwrap();
            remaining[best] = false;
            ordered.push(bag_vars[best]);
        }
        *bag_vars = ordered;
    }
}

/// Build a vtree from a TD rooted at an explicit bag and item ordering — bag
/// assignment is always [`BagAssignment::Deepest`], and every other knob is at
/// its default, the root strategy included since the root is given directly.
pub(super) fn td_to_vtree_with_root(
    input: ConversionInput<'_>,
    root_bag: usize,
    ordering: ItemOrdering,
) -> (Vtree, BagMetadata) {
    let config = TdToVtreeConfig {
        bag_assignment: BagAssignment::Deepest,
        item_ordering: ordering,
        ..TdToVtreeConfig::default()
    };
    convert(input, RootChoice::Bag(root_bag), &config)
}
