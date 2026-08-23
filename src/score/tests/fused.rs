//! The fused five-metric computation against the same metrics spelled out one
//! at a time. Two of those spellings reach the shared arithmetic this module
//! keeps to itself, which is why the pin lives beside `score` rather than in
//! the crate-root tree with the rest of the scoring tests.

use crate::cnf::CnfFormula;
use crate::cnf::{Reduced, ShowSet};
use crate::decompose::TreeDecomposition;
use crate::score::*;
use crate::tests::common::make_td;
use crate::tests::score_fixture::{fixture_formula, fixture_vtree, vtree_peak_context_width};
use crate::vtree::Vtree;

/// A tree decomposition of [`fixture_formula`]'s primal graph: the path of
/// bags `{0,1} — {1,2} — {2,3}`. Written out here rather than produced by
/// a decomposer so this file needs no tree-decomposition backend to test
/// arithmetic that has nothing to do with one.
fn fixture_td() -> TreeDecomposition {
    make_td(
        vec![vec![0, 1], vec![1, 2], vec![2, 3]],
        vec![(0, 1), (1, 2)],
        4,
    )
}

/// Projection-aware peak: max show-variable context width over vtree nodes, or `None`
/// when no show mask is present. `show_mask[vi]` = true marks a show
/// variable, in `formula`'s var space. This is the reference the fused
/// `peak_context_width_show` field is checked against.
fn vtree_peak_context_width_show(
    vtree: &Vtree,
    formula: &CnfFormula,
    show_mask: Option<&crate::cnf::ShowMask>,
) -> Option<u32> {
    show_mask.map(|mask| {
        vtree_context_width_per_node(vtree, formula, Some(mask))
            .into_iter()
            .max()
            .unwrap_or(0)
    })
}

/// Fused-vs-wrapper equivalence pin: `VtreeScores::compute` shares one
/// `clause_lca_counts` and one `clause_high_lca` scan across all five metrics,
/// so its fields must equal the five metrics computed individually.
/// Guards the fused core against divergence from the standalone spellings
/// forever (e.g. a metric's arithmetic edited in one place but not the other).
///
/// Run twice: once on the vtree a conversion realizes from
/// [`fixture_td`] — the shape this pin has always used — and once on
/// [`fixture_vtree`], whose five metrics are also asserted against values
/// worked out by hand in the crate-root scoring tests, so an equality between
/// two identically-broken implementations cannot pass unnoticed.
#[test]
fn compute_matches_individual_fns() {
    let formula = fixture_formula();
    let realized =
        crate::decompose::td_to_vtree_best(&fixture_td(), formula.num_vars, &formula, 1.0);

    for vtree in [realized, fixture_vtree()] {
        let nv = vtree.num_vars();
        let mask = ShowSet::<Reduced>::from_zero_based((0..nv).filter(|i| i % 2 == 0)).mask(nv);
        for show in [None, Some(&mask)] {
            let fused = VtreeScores::compute(&vtree, &formula, show).expect("covering vtree");
            assert_eq!(
                fused.clause_load_stddev,
                stddev_from_counts(&clause_lca_counts(&vtree, &formula)),
                "clause_load_stddev"
            );
            assert_eq!(
                fused.max_clause_load,
                vtree_max_clause_load(&vtree, &formula),
                "max_clause_load"
            );
            assert_eq!(
                fused.peak_context_width_all,
                vtree_peak_context_width(&vtree, &formula),
                "peak_context_width_all"
            );
            assert_eq!(
                fused.peak_context_width_show,
                vtree_peak_context_width_show(&vtree, &formula, show),
                "peak_context_width_show"
            );
            assert_eq!(
                fused.cost,
                vtree_cost(&vtree, &formula).expect("covering vtree"),
                "cost"
            );
        }
    }
}
