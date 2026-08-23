use crate::candidates::*;
use crate::score::VtreeScores;
use crate::vtree::Vtree;
use std::sync::Arc;

fn stats(clause_load_stddev: f64, peak: u32, cost: u64) -> VtreeScores {
    VtreeScores {
        clause_load_stddev,
        max_clause_load: 0,
        peak_context_width_all: peak,
        peak_context_width_show: None,
        cost,
    }
}

/// One entry of a `from_scored` input: the construction name, the tree it
/// built, and its scores.
fn entry(built_by: &str, vtree: Arc<Vtree>, scores: VtreeScores) -> ScoredVtree {
    ScoredVtree {
        built_by: built_by.to_string(),
        vtree,
        scores,
    }
}

/// Three distinct vtrees over the same variable count, so dedup has to look
/// at STRUCTURE rather than leaf count.
fn shapes() -> (Arc<Vtree>, Arc<Vtree>, Arc<Vtree>) {
    (
        Arc::new(Vtree::balanced(4)),
        Arc::new(Vtree::linear(4)),
        Arc::new(Vtree::balanced(4)),
    )
}

/// `keep <= 1` is the default — this is the property that makes it free.
#[test]
fn keep_one_retains_nothing() {
    let (a, b, _) = shapes();
    let scored = vec![
        entry("flowcutter-incidence", a.clone(), stats(1.0, 5, 10)),
        entry("flowcutter-primal", b, stats(2.0, 4, 20)),
    ];
    assert!(from_scored(scored, &a, CandidateRankMetric::ClauseLoadStddev, 1).is_empty());
}

/// Rank 0 is the SELECTED vtree even when another candidate scores better on
/// the ranking metric — the plain-MC adoption rules are not a pure argmin, so
/// pinning rank 0 to the metric would sometimes not be the emitted vtree.
#[test]
fn selected_is_rank_zero_even_when_not_the_metric_minimum() {
    let (a, b, _) = shapes();
    let scored = vec![
        entry("flowcutter-incidence", a.clone(), stats(9.0, 5, 10)),
        entry("flowcutter-primal", b, stats(1.0, 4, 20)),
    ];
    let set = from_scored(scored, &a, CandidateRankMetric::ClauseLoadStddev, 4);
    assert_eq!(set.candidates.len(), 2);
    assert_eq!(
        set.candidates[0].built_by,
        vec!["flowcutter-incidence".to_string()]
    );
    assert!(set.candidates[0].selected);
    assert!(!set.candidates[1].selected);
}

/// Two candidates that produced the SAME tree collapse into one entry naming
/// both, instead of shipping two files a consumer would have to diff to
/// discover are identical.
#[test]
fn identical_vtrees_collapse_into_one_entry_naming_both_specs() {
    let (a, b, a_again) = shapes();
    let scored = vec![
        entry("flowcutter-incidence", a.clone(), stats(1.0, 5, 10)),
        entry("flowcutter-primal", b, stats(2.0, 4, 20)),
        entry("goatd", a_again, stats(1.0, 5, 10)),
    ];
    let set = from_scored(scored, &a, CandidateRankMetric::ClauseLoadStddev, 8);
    assert_eq!(
        set.candidates.len(),
        2,
        "the two identical trees must be one entry"
    );
    assert_eq!(
        set.candidates[0].built_by,
        vec!["flowcutter-incidence".to_string(), "goatd".to_string()],
    );
}

#[test]
fn winner_is_matched_structurally_not_by_pointer() {
    let (a, b, a_again) = shapes();
    let scored = vec![
        entry("flowcutter-incidence", a, stats(9.0, 5, 10)),
        entry("flowcutter-primal", b, stats(1.0, 4, 20)),
    ];
    // `a_again` is a different allocation with the same shape as `flowcutter-incidence`.
    let set = from_scored(scored, &a_again, CandidateRankMetric::ClauseLoadStddev, 4);
    assert_eq!(
        set.candidates[0].built_by,
        vec!["flowcutter-incidence".to_string()]
    );
    assert!(set.candidates[0].selected);
}

/// Projected runs rank on peak context width, which reverses the order the
/// plain-MC stddev metric would give on the same scores.
#[test]
fn peak_metric_orders_by_context_width_not_stddev() {
    let (a, b, _) = shapes();
    let scored = vec![
        entry("flowcutter-incidence", a.clone(), stats(9.0, 3, 10)),
        entry("flowcutter-primal", b, stats(1.0, 7, 20)),
    ];
    // Winner is flowcutter-incidence; with only two candidates the interesting half is that
    // the metric token and the value function agree.
    let set = from_scored(scored, &a, CandidateRankMetric::PeakContextWidthAll, 4);
    assert_eq!(set.metric.as_str(), "peak_context_width_all");
    assert_eq!(
        set.candidates[1].built_by,
        vec!["flowcutter-primal".to_string()]
    );
}

#[test]
fn truncation_keeps_the_selected_vtree() {
    let (a, b, _) = shapes();
    let c: Arc<Vtree> = Arc::new(Vtree::linear_from_order(&[
        crate::vtree::VarId(3),
        crate::vtree::VarId(2),
        crate::vtree::VarId(1),
        crate::vtree::VarId(0),
    ]));
    let scored = vec![
        entry("flowcutter-incidence", b, stats(1.0, 4, 20)),
        entry("flowcutter-primal", c, stats(2.0, 4, 20)),
        entry("goatd", a.clone(), stats(9.0, 9, 90)),
    ];
    let set = from_scored(scored, &a, CandidateRankMetric::ClauseLoadStddev, 2);
    assert_eq!(set.candidates.len(), 2);
    assert!(set.candidates[0].selected);
    assert_eq!(set.candidates[0].built_by, vec!["goatd".to_string()]);
}

/// Four distinct shapes over the same four variables, for the tie chain below:
/// dedup is by structure, so a tie test needs one tree per entry that no other
/// entry can collapse into.
fn more_shapes() -> (Arc<Vtree>, Arc<Vtree>) {
    use crate::vtree::VarId;
    (
        Arc::new(Vtree::reverse_linear(4)),
        Arc::new(Vtree::linear_from_order(&[
            VarId(2),
            VarId(3),
            VarId(0),
            VarId(1),
        ])),
    )
}

/// The show peak is absent for a whole run at once, so under the show metric a
/// non-projected set has no show score to rank on and reads the all-variable
/// peak instead. The spreads here run opposite to the peaks, so a fallback onto
/// the wrong field would reverse the order.
#[test]
fn a_show_metric_falls_back_to_the_all_variable_peak_when_no_show_score_exists() {
    let (a, b, _) = shapes();
    let (c, _) = more_shapes();
    let scored = vec![
        entry("goatd", a.clone(), stats(1.0, 9, 10)),
        entry("flowcutter-primal", b, stats(2.0, 7, 20)),
        entry("flowcutter-incidence", c, stats(3.0, 3, 30)),
    ];
    let set = from_scored(scored, &a, CandidateRankMetric::PeakContextWidthShow, 8);

    let order: Vec<&str> = set
        .candidates
        .iter()
        .map(|c| c.built_by[0].as_str())
        .collect();
    assert_eq!(
        order,
        ["goatd", "flowcutter-incidence", "flowcutter-primal"],
        "the selected vtree first, then the all-variable peaks ascending",
    );
}

/// The whole ordering, one level at a time: the selected vtree, then the
/// ranking metric, then the spread, then the cost, then the construction name.
/// Every level exists because the one above it can tie, and a tie resolved by
/// input order would make an emitted candidate set depend on catalog order
/// rather than on the trees.
#[test]
fn equal_metric_values_break_ties_by_spread_then_cost_then_name() {
    let (a, b, _) = shapes();
    let (c, d) = more_shapes();
    // Every candidate has the same peak, so the metric ties for all of them and
    // each following level decides exactly one pair.
    let scored = vec![
        entry("d-cost", d, stats(2.0, 5, 60)),
        entry("z-selected", a.clone(), stats(9.0, 5, 900)),
        entry("a-cost", c, stats(2.0, 5, 10)),
        entry("b-spread", b, stats(1.0, 5, 50)),
    ];
    let set = from_scored(scored, &a, CandidateRankMetric::PeakContextWidthAll, 8);

    let order: Vec<&str> = set
        .candidates
        .iter()
        .map(|c| c.built_by[0].as_str())
        .collect();
    assert_eq!(
        order,
        ["z-selected", "b-spread", "a-cost", "d-cost"],
        "selected, then spread, then cost — the metric ties everywhere",
    );
    assert!(set.candidates[0].selected);
}

/// The last level: everything above it equal, and the name is what is left to
/// order two candidates by.
#[test]
fn candidates_equal_on_every_score_are_ordered_by_construction_name() {
    let (a, b, _) = shapes();
    let (c, _) = more_shapes();
    let scored = vec![
        entry("goatd", b, stats(2.0, 5, 60)),
        entry("flowcutter-primal", c, stats(2.0, 5, 60)),
    ];
    let set = from_scored(scored, &a, CandidateRankMetric::PeakContextWidthAll, 8);

    let order: Vec<&str> = set
        .candidates
        .iter()
        .map(|c| c.built_by[0].as_str())
        .collect();
    assert_eq!(order, ["flowcutter-primal", "goatd"]);
}

/// The manifest token is the whole of what a consumer gets back: it has to
/// round-trip for every metric, or reading `candidate_rank_metric` out of an
/// emitted manifest cannot recover which score the ranks were ordered by.
#[test]
fn every_metric_round_trips_through_its_manifest_token() {
    for m in [
        CandidateRankMetric::ClauseLoadStddev,
        CandidateRankMetric::PeakContextWidthShow,
        CandidateRankMetric::PeakContextWidthAll,
    ] {
        assert_eq!(CandidateRankMetric::parse(m.as_str()), Some(m));
    }
    assert_eq!(CandidateRankMetric::parse("cost"), None);
}
