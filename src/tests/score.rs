use crate::cnf::Clause;
use crate::cnf::CnfFormula;
use crate::cnf::{Reduced, ShowSet};
use crate::score::*;
use crate::tests::common::lit;
use crate::tests::score_fixture::{fixture_formula, fixture_vtree};
use crate::vtree::VarId;

/// The five metrics of [`fixture_vtree`] over [`fixture_formula`], worked
/// out by hand. Without this, the equivalence pin beside `score` would still
/// pass on a fixture whose every metric came out zero.
///
/// Name the vtree's nodes `A` (spans `{v0,v1}`), `B` (spans `{v2,v3}`) and
/// `R` (the root). Each clause lands on the lowest node spanning all its
/// variables: c1 and c5 on `A`, c2 on `B`, c3 and c4 on `R`. So the per-node
/// clause loads are `A=2, B=1, R=2`, and every other node is 0.
///
/// * `max_clause_load` = 2, the largest of those.
/// * `clause_load_stddev` is taken over the loaded nodes only, i.e. over `2, 1, 2`:
///   mean 5/3, sample variance 1/3, so `sqrt(1/3)`.
/// * `peak_context_width_all`: every variable's widest-spanning clause has its meeting
///   point at `R` (v0 through c3, v1 through c4, v2 through c3, v3 through
///   c4), so each variable crosses exactly the one node between its leaf and
///   `R` — `A` for v0 and v1, `B` for v2 and v3. Both cuts are 2 wide.
/// * `peak_context_width_show` with only v0 and v2 shown counts one crossing at `A` and
///   one at `B`, so 1.
/// * `cost` = `max_load³ + Σ (left clauses × right clauses) + Σ load ×
///   ⌊log₂ leaves⌋` = `8 + (0·0 + 0·0 + 2·1) + (2·1 + 1·1 + 2·2)` = `8 + 2 +
///   7` = 17.
#[test]
fn fixture_metrics_match_hand_computation() {
    let formula = fixture_formula();
    let vtree = fixture_vtree();
    let show = ShowSet::<Reduced>::from_zero_based([0, 2]).mask(4);

    let stats = VtreeScores::compute(&vtree, &formula, Some(&show)).expect("covering vtree");
    assert_eq!(stats.max_clause_load, 2, "max_clause_load");
    assert!(
        (stats.clause_load_stddev - (1.0f64 / 3.0).sqrt()).abs() < 1e-12,
        "clause_load_stddev: {}",
        stats.clause_load_stddev
    );
    assert_eq!(stats.peak_context_width_all, 2, "peak_context_width_all");
    assert_eq!(
        stats.peak_context_width_show,
        Some(1),
        "peak_context_width_show"
    );
    assert_eq!(stats.cost, 17, "cost");
}

/// A vtree scored against a formula it has no leaves for is what the public
/// scoring entries advertise, so it has to come back as a `Mismatch` rather
/// than out of bounds: [`fixture_vtree`] carries four variables, the formula
/// here names six.
#[test]
fn scoring_a_formula_the_vtree_does_not_cover_is_a_mismatch() {
    let wider = CnfFormula {
        num_vars: 6,
        clauses: vec![Clause::new(vec![lit(0, true), lit(5, true)])],
    };
    let vtree = fixture_vtree();

    assert!(matches!(
        VtreeScores::compute(&vtree, &wider, None),
        Err(crate::error::VitriError::Mismatch { .. })
    ));
    assert!(matches!(
        vtree_cost(&vtree, &wider),
        Err(crate::error::VitriError::Mismatch { .. })
    ));
}

/// The declared variable space may be wider than the clauses use, and a vtree
/// covering everything the clauses NAME is still the right vtree for it — the
/// check reads the clauses precisely so a formula whose tail ids never occur is
/// scored rather than refused.
#[test]
fn a_formula_declaring_more_variables_than_it_uses_still_scores() {
    let mut wider = fixture_formula();
    wider.num_vars = 9;
    wider
        .clauses
        .push(Clause::new(vec![lit(1, true), lit(3, true)]));
    let vtree = fixture_vtree();

    VtreeScores::compute(&vtree, &wider, None).expect("no clause names a variable the vtree lacks");
    vtree_cost(&vtree, &wider).expect("no clause names a variable the vtree lacks");
}

/// A mismatch is a caller who paired the wrong two things, so the message has
/// to say which: the variable that could not be indexed, spelled the way the
/// caller's own file spells it, and how wide the vtree it was checked against
/// is.
#[test]
fn the_mismatch_message_names_the_dimacs_variable_the_vtree_lacks() {
    let wider = CnfFormula {
        num_vars: 6,
        clauses: vec![Clause::new(vec![lit(0, true), lit(5, true)])],
    };
    let err = VtreeScores::compute(&fixture_vtree(), &wider, None)
        .map(|_| ())
        .expect_err("the vtree has no leaf for that variable")
        .to_string();
    assert!(
        err.contains('6'),
        "the message must name the offending variable as the file spells it (6), got: {err}",
    );
    assert!(
        err.contains('4'),
        "the message must name how many variables the vtree indexes (4), got: {err}",
    );
}

/// A clause with no literals meets nowhere, so it lands on no node: every
/// metric reads the same with and without it, and the walk that reduces a
/// clause to its meeting point does not run out of literals to reduce.
#[test]
fn an_empty_clause_contributes_to_no_score() {
    let formula = fixture_formula();
    let mut with_empty = formula.clone();
    with_empty.clauses.push(Clause::new(Vec::new()));
    let vtree = fixture_vtree();
    let show = ShowSet::<Reduced>::from_zero_based([0, 2]).mask(4);

    assert_eq!(
        VtreeScores::compute(&vtree, &with_empty, Some(&show)).expect("covering vtree"),
        VtreeScores::compute(&vtree, &formula, Some(&show)).expect("covering vtree"),
    );
}

/// A unit clause has nothing to meet either, but it IS a clause: it lands on
/// its own variable's leaf and adds one to the load there. What it does not do
/// is widen a cut — no second variable is tied to it, so nothing crosses.
#[test]
fn a_unit_clause_loads_its_own_leaf_but_crosses_no_cut() {
    let formula = fixture_formula();
    let mut with_unit = formula.clone();
    with_unit.clauses.push(Clause::new(vec![lit(0, true)]));
    let vtree = fixture_vtree();
    let leaf = vtree.leaf_of(VarId(0));

    let before = vtree_clause_load_per_node(&vtree, &formula);
    let after = vtree_clause_load_per_node(&vtree, &with_unit);
    for node in 0..vtree.num_nodes() {
        let expected = before[node] + u32::from(node == leaf.idx());
        assert_eq!(
            after[node], expected,
            "node {node} carries the unit clause only if it is that variable's leaf",
        );
    }

    assert_eq!(
        VtreeScores::compute(&vtree, &with_unit, None)
            .expect("covering vtree")
            .peak_context_width_all,
        VtreeScores::compute(&vtree, &formula, None)
            .expect("covering vtree")
            .peak_context_width_all,
        "a unit clause ties no two variables together, so it widens no cut",
    );
}

/// With no clause to place there is no loaded node to take a spread over.
/// Reported as zero rather than the `NaN` a mean over nothing would give — a
/// `NaN` sorts unpredictably against every other candidate.
#[test]
fn a_formula_with_no_clauses_scores_zero_in_every_metric() {
    let empty = CnfFormula {
        num_vars: 4,
        clauses: Vec::new(),
    };
    let show = ShowSet::<Reduced>::from_zero_based([0, 2]).mask(4);
    let scores = VtreeScores::compute(&fixture_vtree(), &empty, Some(&show)).expect("covering");

    assert!(
        !scores.clause_load_stddev.is_nan(),
        "an empty formula must not score NaN",
    );
    assert_eq!(
        scores,
        VtreeScores {
            clause_load_stddev: 0.0,
            max_clause_load: 0,
            peak_context_width_all: 0,
            peak_context_width_show: Some(0),
            cost: 0,
        },
    );
}

/// A show mask states which variables are kept, so a mask that keeps none
/// reports a zero peak — the answer for a projection onto nothing — rather than
/// the `None` that means "this run is not projected at all". A mask stopping
/// short of the variable space hides the ids past its end for the same reason.
#[test]
fn an_all_hidden_show_mask_reports_a_zero_show_peak() {
    let formula = fixture_formula();
    let vtree = fixture_vtree();

    let none_shown = ShowSet::<Reduced>::empty().mask(4);
    assert_eq!(
        VtreeScores::compute(&vtree, &formula, Some(&none_shown))
            .expect("covering vtree")
            .peak_context_width_show,
        Some(0),
        "a mask that keeps nothing is a projection onto nothing, not an absent mask",
    );
    assert_eq!(
        VtreeScores::compute(&vtree, &formula, None)
            .expect("covering vtree")
            .peak_context_width_show,
        None,
        "the contrast: no mask at all is the non-projected reading",
    );

    // A mask covering only v0: v1 through v3 lie past its end and are hidden,
    // so the peak is v0's own crossing at the node spanning {v0, v1}.
    let short = ShowSet::<Reduced>::from_zero_based([0]).mask(1);
    assert_eq!(
        VtreeScores::compute(&vtree, &formula, Some(&short))
            .expect("covering vtree")
            .peak_context_width_show,
        Some(1),
        "a mask shorter than the variable space hides the ids it omits",
    );
}

/// The near-uniform verdict is read by two decisions that never see each
/// other — the bounded-variable-addition policy and the portfolio's candidate
/// gate — so what it measures, and that it is measured once, is worth pinning.
mod structure_profile {
    use crate::cnf::CnfFormula;
    use crate::preprocess::ArjunSbva;
    use crate::preprocess::arjun::arjun_sbva_skip;
    use crate::score::StructureProfile;

    fn formula(text: &str) -> CnfFormula {
        CnfFormula::from_dimacs(text.as_bytes())
            .expect("the fixture is well-formed DIMACS")
            .0
    }

    /// Every clause the same width, every variable the same number of
    /// occurrences: a cycle of binary clauses, which is the shape a
    /// graph-colouring encoding has.
    fn uniform() -> CnfFormula {
        formula("p cnf 4 4\n1 2 0\n2 3 0\n3 4 0\n4 1 0\n")
    }

    /// One variable in every clause and one clause far wider than the rest.
    fn skewed() -> CnfFormula {
        formula("p cnf 8 4\n1 2 0\n1 3 0\n1 4 5 6 7 8 0\n1 2 0\n")
    }

    #[test]
    fn a_formula_with_no_spread_at_all_measures_as_uniform() {
        let profile = StructureProfile::measure(&uniform());
        assert_eq!(profile.clause_width_cv, 0.0);
        assert_eq!(profile.var_occurrence_cv, 0.0);
        assert!(
            profile.coloring_like,
            "a formula with no dispersion is the extreme case of near-uniform",
        );
    }

    #[test]
    fn a_formula_with_one_hub_variable_and_one_wide_clause_measures_as_skewed() {
        let profile = StructureProfile::measure(&skewed());
        assert!(profile.clause_width_cv > 0.0);
        assert!(profile.var_occurrence_cv > 0.0);
        assert!(
            !profile.coloring_like,
            "widths of 2, 2, 6, 2 are not a near-uniform spread, got {profile:?}",
        );
    }

    /// A formula too small to have a spread reads as uniform rather than as
    /// undefined, which is the reading every consumer of this is written for.
    #[test]
    fn a_formula_with_a_single_clause_has_no_dispersion_to_report() {
        let profile = StructureProfile::measure(&formula("p cnf 3 1\n1 2 3 0\n"));
        assert_eq!(profile.clause_width_cv, 0.0);
        assert_eq!(profile.var_occurrence_cv, 0.0);
    }

    /// The pin: what a caller reads is what the bounded-variable-addition
    /// policy decides on, not a second measurement that agrees with it today.
    #[test]
    fn the_verdict_a_caller_reads_is_the_one_the_sbva_policy_acts_on() {
        for fixture in [uniform(), skewed()] {
            assert_eq!(
                arjun_sbva_skip(&fixture, ArjunSbva::Auto),
                StructureProfile::measure(&fixture).coloring_like,
                "the policy and the published profile disagree about {fixture:?}",
            );
        }
    }
}
