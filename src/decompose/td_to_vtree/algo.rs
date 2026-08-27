//! The tree-decomposition → vtree conversion itself: root each component of the
//! decomposition, assign every variable to exactly one bag, then build the
//! vtree bottom-up over the bags in reverse BFS order, handing each bag's
//! already-built child subtrees and its local leaves to a combiner.
//!
//! Every construction in this crate that starts from a decomposition ends up
//! here; the search in `search` drives it once per reading. The tree comes back
//! paired with the [`BagMetadata`] of the same run.
//!
//! One variable, one bag, one leaf. A variable occurring in several bags is
//! assigned to a single one, so a bag's assigned-variable set and its bag-vertex
//! set are different things — separator-aware combiners need the second, which
//! is why it is tracked separately. Variables in no bag at all become top-level
//! leaves beside the component roots.

use crate::cnf::CnfFormula;
use crate::vtree::{VarId, Vtree, VtreeArena, VtreeIdx};

use super::super::TreeDecomposition;
use super::super::td_parse::primal_adjacency;
use super::combiners::{combine_edge_aligned, combine_hypergraph_bisect, combine_into_balanced};
use super::meta::BagMetadata;
use super::reading::{Binarization, FixedReading, Place, RootPick};

/// What is being converted, as opposed to how: the decomposition, the variable
/// space it is converted into, the formula behind it when the caller has one,
/// and the effort the conversion may spend.
///
/// These four are fixed for a whole conversion — a search that reads one
/// decomposition a dozen ways varies only the reading, and carrying the fixed
/// part as one value is what makes that visible at each of its call sites.
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
    /// Effort multiplier for [`Binarization::Hypergraph`], the one binarization that spends a
    /// scalable budget.
    pub effort_scale: f64,
}

/// ONE reading of a tree decomposition, built.
///
/// Every construction in this crate that starts from a decomposition ends up
/// here, once per reading the search in [`super::search`] reaches.
///
/// Returns the vtree together with the [`BagMetadata`] describing which bag each
/// variable was assigned to, paired in one value so a winning reading's
/// metadata can never be mismatched with a different reading's vtree.
pub(super) fn convert_one(
    input: ConversionInput<'_>,
    reading: FixedReading,
) -> (Vtree, BagMetadata) {
    let ConversionInput {
        td,
        num_vars,
        formula,
        effort_scale,
    } = input;
    let n = td.bags().len();

    // `chosen.chain(0..n)`: the named root(s) go first and claim their
    // component; 0..n then supplies a root for every component they didn't
    // reach.
    let chosen = root_bags(td, reading.root);
    let forest = td
        .rooted_forest(chosen.iter().copied())
        .expect("conversion roots are bag indices");
    let order = forest.order();
    let parent_td = forest.parents();
    let depth = forest.depths();
    let component_roots = forest.component_roots();

    let var_bag = assign_var_bags(td, num_vars, order, depth, reading.place, formula);

    // Bag assignment is final here — build the TD metadata from the very arrays
    // the conversion just produced (no second assignment pass anywhere).
    let meta = BagMetadata::from_assignment(num_vars, &var_bag, order, n, td.treewidth());

    // vars_at[t] = variables assigned to TD node t.
    let mut vars_at: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut in_any_bag = vec![false; num_vars as usize];
    for v in 0..num_vars as usize {
        if var_bag[v] != usize::MAX {
            vars_at[var_bag[v]].push(v as u32);
            in_any_bag[v] = true;
        }
    }

    // --- Step 2: build the vtree bottom-up ---------------------------------
    // Primal adjacency for the edge-aligned binarization's clause-partner routing
    // (built once, only when it is selected and a formula is present).
    let edge_primal_adj: Vec<Vec<u32>> = if reading.binarize == Binarization::Edge {
        formula
            .map(|f| primal_adjacency(f, num_vars))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut nodes = VtreeArena::new();
    let mut td_vtree_idx: Vec<Option<VtreeIdx>> = vec![None; n];
    // Variables in each TD subtree: a subtree's variable count IS its vtree
    // subtree's leaf count.
    let mut td_vars: Vec<Vec<u32>> = vec![Vec::new(); n];
    // The union of BAG vertices in each TD subtree (all vars *appearing* in the
    // subtree, not just those *assigned* leaves there). Only maintained for
    // [`Binarization::Edge`], which needs it to detect which branches reference a lifted
    // separator (a shared var assigned to an ancestor is absent from its
    // branches' assigned-var sets but present in their bag-vertex sets).
    let track_bag_vars = reading.binarize == Binarization::Edge;
    let mut td_bag_vars: Vec<Vec<u32>> = vec![Vec::new(); n];

    for &t in order.iter().rev() {
        // (vtree index, subtree variable count) per child subtree.
        let mut child_items: Vec<(VtreeIdx, usize)> = Vec::new();
        let mut child_var_sets: Vec<Vec<u32>> = Vec::new();
        // Parallel bag-vertex sets for each child subtree ([`Binarization::Edge`] only).
        let mut child_bag_var_sets: Vec<Vec<u32>> = Vec::new();
        for &nb in &td.adjacency()[t] {
            if Some(nb) != parent_td[t]
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

        // Children then leaves, which is the order every binarization reads: the two
        // that reorder do it in their own combiner, off clause structure this
        // list cannot carry.
        let mut items: Vec<VtreeIdx> = child_items.iter().map(|(idx, _)| *idx).collect();
        items.extend_from_slice(&var_items);

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
            for &v in td.bags()[t].vertices() {
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
            };
            Some(combine_bag(
                &bag,
                reading.binarize,
                formula,
                effort_scale,
                &edge_primal_adj,
                &mut nodes,
            ))
        };
    }

    // Top-level vtree roots: one per TD component, then the isolated variables.
    let mut top_items: Vec<VtreeIdx> = Vec::new();
    for &cr in component_roots {
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
///
/// Also what the search reads to tell two root strategies apart before spending
/// a build on each.
pub(super) fn root_bags(td: &TreeDecomposition, root: RootPick) -> Vec<usize> {
    // Lowest-index bag of each component, in component-discovery order.
    let first = || {
        td.rooted_forest(0..td.bags().len())
            .expect("all generated roots are bag indices")
            .component_roots()
            .to_vec()
    };
    match root {
        RootPick::Leaf(bag) => vec![bag],
        RootPick::First => first(),
        RootPick::Centroid => first().iter().map(|&cr| find_centroid(td, cr)).collect(),
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
    place: Place,
    formula: Option<&CnfFormula>,
) -> Vec<usize> {
    let mut var_bag = vec![usize::MAX; num_vars as usize];
    match place {
        Place::Deep => {
            // Pass 1: BFS order is shallowest-first, so the last write wins and
            // each variable lands in its deepest bag.
            let mut var_max_depth = vec![0usize; num_vars as usize];
            for &bag_idx in order {
                for &v in td.bags()[bag_idx].vertices() {
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
        Place::Shallow => {
            for (bag_idx, bag) in td.bags().iter().enumerate() {
                for &v in bag.vertices() {
                    if (v as usize) < num_vars as usize {
                        let cur = var_bag[v as usize];
                        if cur == usize::MAX || depth[bag_idx] < depth[cur] {
                            var_bag[v as usize] = bag_idx;
                        }
                    }
                }
            }
        }
    }
    var_bag
}

/// One TD node's pieces, as the combiners read them.
struct BagItems<'a> {
    /// What to combine: the child subtrees, then this bag's own leaves.
    items: &'a [VtreeIdx],
    /// Each child subtree's vtree root and variable count, in TD adjacency
    /// order.
    child_items: &'a [(VtreeIdx, usize)],
    /// Each child subtree's variables, in TD adjacency order.
    child_var_sets: &'a [Vec<u32>],
    /// Each child subtree's bag-vertex union, in the same order. Empty unless
    /// [`Binarization::Edge`] is running — the only binarization that reads it.
    child_bag_var_sets: &'a [Vec<u32>],
    /// One leaf per variable assigned to this bag.
    var_items: &'a [VtreeIdx],
    /// The variables those leaves carry, in the same order.
    vars_here: &'a [u32],
}

/// Combine one TD node's items into a single vtree subtree, by the rule `binarize`
/// names. A binarization that needs clause structure falls back to the plain balanced
/// combine without a formula.
fn combine_bag(
    bag: &BagItems<'_>,
    binarize: Binarization,
    formula: Option<&CnfFormula>,
    effort_scale: f64,
    edge_primal_adj: &[Vec<u32>],
    nodes: &mut VtreeArena,
) -> VtreeIdx {
    let items = bag.items;
    match (binarize, formula) {
        (Binarization::Hypergraph, Some(formula)) => {
            // Per-item variable sets, in `items` order: children first, then one
            // set per leaf.
            let mut item_vars: Vec<Vec<u32>> = bag.child_var_sets.to_vec();
            for &v in bag.vars_here {
                item_vars.push(vec![v]);
            }
            debug_assert_eq!(
                item_vars.len(),
                items.len(),
                "item_vars len {} != items len {} (children={}, vars={})",
                item_vars.len(),
                items.len(),
                bag.child_var_sets.len(),
                bag.vars_here.len()
            );
            combine_hypergraph_bisect(items, &item_vars, formula, effort_scale, nodes)
        }
        (Binarization::Edge, Some(_)) => {
            let child_idxs: Vec<VtreeIdx> = bag.child_items.iter().map(|(idx, _)| *idx).collect();
            combine_edge_aligned(
                &child_idxs,
                bag.child_bag_var_sets,
                bag.var_items,
                bag.vars_here,
                edge_primal_adj,
                nodes,
            )
        }
        _ => combine_into_balanced(items, nodes),
    }
}

/// Find the centroid of a tree rooted at `start`. The centroid is the node
/// that minimizes the maximum subtree size when the tree is rooted at it.
pub(super) fn find_centroid(td: &TreeDecomposition, start: usize) -> usize {
    let adj = td.adjacency();
    // Precondition: a non-empty decomposition containing `start` — an empty
    // `adj` panics `visited[start]` below with index-out-of-bounds.
    debug_assert!(
        !adj.is_empty(),
        "find_centroid requires a non-empty decomposition"
    );
    let forest = td
        .rooted_forest([start])
        .expect("centroid start is a bag index");
    let all_order = forest.order();
    let parent = forest.parents();
    let component_size = all_order
        .iter()
        .skip(1)
        .position(|&bag| parent[bag].is_none())
        .map_or(all_order.len(), |offset| offset + 1);
    let order = &all_order[..component_size];

    if component_size <= 2 {
        return start;
    }

    let mut subtree_size = vec![1usize; adj.len()];
    for &t in order.iter().rev() {
        if let Some(parent) = parent[t] {
            let child_size = subtree_size[t];
            subtree_size[parent] += child_size;
        }
    }

    // Centroid: the node where max(subtree_child_sizes, component_size - subtree_size[node])
    // is minimized.
    let mut best_node = start;
    let mut best_max = component_size;
    for &t in order {
        let mut max_part = component_size - subtree_size[t]; // "upward" partition
        for &nb in &adj[t] {
            // Every neighbour of a walked bag was walked with it, so a bag
            // other than the parent is a child.
            if Some(nb) != parent[t] {
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
        let bag_vars: Vec<u32> = walk.td.bags()[bag_idx]
            .vertices()
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
