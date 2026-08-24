//! Root/ordering sweep: convert one tree decomposition many ways and keep the
//! lowest-scoring vtree.
//!
//! The roots tried are the first bag, the centroid, and the decomposition's
//! leaf bags; each is converted under several [`ItemOrdering`]s and scored with
//! [`vtree_cost`]. Past a handful of roots the sweep screens every root under
//! one ordering and re-converts only the best few. This is the entry a backend
//! holding both a decomposition and the formula it came from uses, rather than
//! `td_to_vtree_configured`, which has no formula to score against.
//!
//! The [`TdConversionMeta`] handed back carries the WINNING conversion's bag
//! metadata: the sweep builds one bag assignment per conversion, and only that
//! one describes the returned tree.

use std::sync::Arc;
use std::time::Instant;

use crate::cnf::CnfFormula;
use crate::score::{BUILT_FROM_THIS_FORMULA, vtree_cost};
use crate::vtree::Vtree;

use super::super::TreeDecomposition;
use super::super::best::BestBy;
use super::algo::{ConversionInput, find_centroid, td_to_vtree_with_root};
use super::config::ItemOrdering;
use super::meta::BagMetadata;

/// What a TD → vtree conversion produced BESIDE the tree, returned alongside it
/// so the two can never be paired up wrongly.
///
/// Consumers must still validate `meta.num_vars()` against the formula they
/// intend to use it with: a nested conversion over a sub-decomposition describes
/// a DIFFERENT variable space. Metadata only steers clause ORDER, never the
/// compiled function, so a mismatch costs scheduling quality, not correctness.
#[derive(Clone, Default)]
pub(crate) struct TdConversionMeta {
    /// Bag metadata of the WINNING (root, ordering) conversion — the one that
    /// produced the returned vtree, never a runner-up's. `None` when the vtree
    /// did not come from a TD conversion at all.
    pub meta: Option<Arc<BagMetadata>>,
}

/// Build the best vtree for a TD by sweeping every (root, ordering)
/// conversion of it and selecting the lowest `vtree_cost`.
///
/// `deadline` bounds the sweep, never its result: see
/// [`td_to_vtree_best_traced`].
pub(crate) fn td_to_vtree_best(
    td: &TreeDecomposition,
    num_vars: u32,
    formula: &CnfFormula,
    effort_scale: f64,
    deadline: Option<Instant>,
) -> Vtree {
    td_to_vtree_best_traced(td, num_vars, formula, effort_scale, deadline).0
}

/// [`td_to_vtree_best`] with the conversion's by-products — the two share one
/// root/ordering sweep rather than duplicating it.
///
/// `deadline` governs how many (root, ordering) conversions get scored, never
/// whether any does: it is tested between conversions and only once one has been
/// adopted, so an already-expired deadline still yields the sweep's first
/// candidate, a single O(n) build. The natural caller is a portfolio candidate
/// converting a decomposition it has just spent its budget building, and a
/// refusal there would discard that decomposition exactly when the surrounding
/// wall cap starts working. A conversion already under way runs to completion,
/// so a bounded call may overrun by one of them.
pub(crate) fn td_to_vtree_best_traced(
    td: &TreeDecomposition,
    num_vars: u32,
    formula: &CnfFormula,
    effort_scale: f64,
    deadline: Option<Instant>,
) -> (Vtree, TdConversionMeta) {
    // Precondition: a non-empty tree decomposition. A 0-variable formula has no
    // vtree to build — callers must short-circuit it before reaching here.
    assert!(
        !td.adj.is_empty(),
        "td_to_vtree_best: empty tree decomposition (num_vars={num_vars}); \
         callers must short-circuit 0-variable formulas before vtree construction",
    );
    // Fixed for every conversion below: the sweep varies the root and the
    // ordering and nothing else.
    let input = ConversionInput {
        td,
        num_vars,
        formula: Some(formula),
        effort_scale,
    };
    // The roots to try: first bag, centroid, and all TD leaf bags (degree 1).
    // Each root produces a different vtree shape. td_to_vtree_with_root is O(n)
    // and RF scoring is O(1), so trying many roots is cheap.
    let n_bags = td.adj.len();
    let mut roots: Vec<usize> = vec![0];
    let centroid = find_centroid(0, &td.adj);
    if centroid != 0 {
        roots.push(centroid);
    }
    // Add leaf bags (degree 1) — these often produce deep, narrow vtrees
    // that can be better for formulas with chain-like clause structure.
    for i in 0..n_bags {
        if td.adj[i].len() == 1 && !roots.contains(&i) {
            roots.push(i);
            if roots.len() >= 20 {
                break;
            }
        }
    }
    // Expensive orderings (ClauseSplit, HypergraphBisect) involve O(n²)
    // bisection that dominates runtime on large TDs. Skip them when num_vars > 2000.
    let cheap_orderings: &[ItemOrdering] = &[
        ItemOrdering::ChildrenFirst,
        ItemOrdering::ChildrenBySize,
        ItemOrdering::VariablesFirst,
        ItemOrdering::Reversed,
    ];
    let expensive_orderings = [ItemOrdering::ClauseSplit, ItemOrdering::HypergraphBisect];
    let use_expensive = num_vars <= 2000;

    // Each conversion is a (vtree, bag assignment) pair, so the winner keeps the
    // assignment that describes IT and every runner-up's is dropped with the
    // tree it described.
    type BestConversion = BestBy<(Vtree, BagMetadata), f64>;
    let mut best = BestConversion::new();

    // Run one (root, ordering) conversion, score it and put it in front of
    // `best`; the score comes back so a screening pass can rank roots by it.
    let offer = |best: &mut BestConversion, root: usize, ordering: ItemOrdering| -> f64 {
        let built = td_to_vtree_with_root(input, root, ordering);
        let score = vtree_cost(&built.0, formula).expect(BUILT_FROM_THIS_FORMULA);
        best.offer(built, score);
        score
    };

    // Whether the sweep is out of time. Something must already be adopted, which
    // is what makes the bound cost conversions rather than the vtree.
    let out_of_time =
        |best: &BestConversion| best.has_candidate() && crate::budget::expired(deadline);

    // The sequence one root gets. `rank` is its position in the order the roots
    // are tried, which is what confines the expensive orderings to the first
    // two of them. Reports whether the bound stopped it.
    let offer_root =
        |best: &mut BestConversion, root: usize, rank: usize, orderings: &[ItemOrdering]| -> bool {
            for &ordering in orderings {
                if out_of_time(best) {
                    return true;
                }
                offer(best, root, ordering);
            }
            if use_expensive && rank < 2 {
                for &ordering in &expensive_orderings {
                    if out_of_time(best) {
                        return true;
                    }
                    offer(best, root, ordering);
                }
            }
            false
        };

    // Above a handful of roots, screening beats sweeping: score every root under
    // one ordering, then spend the rest of the portfolio on the roots that
    // ordering liked. Below it, every root gets the whole sequence.
    let screen_top_n = 5;
    if roots.len() > screen_top_n {
        let mut root_scores: Vec<(usize, f64)> = Vec::with_capacity(roots.len());
        for &root in &roots {
            if out_of_time(&best) {
                break;
            }
            root_scores.push((root, offer(&mut best, root, ItemOrdering::ChildrenFirst)));
        }
        // Lower is better. The sort is stable, so equal-scoring roots keep the
        // order they were generated in.
        root_scores.sort_by(|a, b| a.1.total_cmp(&b.1));
        root_scores.truncate(screen_top_n);

        // ChildrenFirst is already offered for every root by the screen.
        let remaining_orderings: Vec<ItemOrdering> = cheap_orderings
            .iter()
            .copied()
            .filter(|&o| o != ItemOrdering::ChildrenFirst)
            .collect();
        for (rank, &(root, _)) in root_scores.iter().enumerate() {
            if offer_root(&mut best, root, rank, &remaining_orderings) {
                break;
            }
        }
    } else {
        for (rank, &root) in roots.iter().enumerate() {
            if offer_root(&mut best, root, rank, cheap_orderings) {
                break;
            }
        }
    }

    let ((vtree, meta), _) = best.into_best().expect("at least one root");
    let info = TdConversionMeta {
        meta: Some(Arc::new(meta)),
    };
    // A malformed TD (phantom vertices, inconsistent adjacency) can leak extra
    // leaves into the vtree.
    assert_eq!(
        vtree.num_leaves(),
        num_vars,
        "td_to_vtree_best produced a malformed vtree: {} leaves for a \
         {}-variable formula",
        vtree.num_leaves(),
        num_vars,
    );

    (vtree, info)
}
