//! The optional candidate set: off by default, ranked when asked for.
//!
//! Requesting the runners-up must not change which vtree is selected —
//! selection is decided before the set exists — and must not cost anything
//! when it is not requested, which is checked as an observable rather than
//! assumed.

use super::*;

use crate::candidates::{CandidateRankMetric, CandidateSet};
use crate::tests::common::wide_component;

fn candidates_config(candidates: usize) -> RunConfig {
    RunConfig {
        vtree_spec: "portfolio".to_string(),
        budget_ms: Some(20_000),
        candidates,
        ..Default::default()
    }
}

/// The default bundle is exactly what it was before the candidate set existed: no
/// `candidates/` directory, no `vtree_candidates`, no rank metric. This is
/// the "zero cost by default" contract stated as an observable.
#[test]
fn no_candidate_set_is_emitted_by_default() {
    let formula = wide_component();
    let cfg = candidates_config(1);
    assert_eq!(cfg.candidates, RunConfig::default().candidates);
    let built = build_vtree(&formula, &cfg, &SelectionCtx::plain()).expect("the vtree must build");
    assert!(
        built.candidate_sets.iter().all(CandidateSet::is_empty),
        "no candidate set may be retained at the default width"
    );

    let dir = Scratch::new("no-candidates");
    let (m, paths) = write_components(
        dir.path(),
        &formula,
        &built,
        None,
        ComponentWriteOptions::default(),
    )
    .expect("must write");
    assert!(m.candidate_rank_metric.is_none());
    assert!(m.components.iter().all(|c| c.vtree_candidates.is_empty()));
    assert!(paths.candidates.is_empty());
    assert!(
        !dir.out(CANDIDATES_DIR).exists(),
        "no candidates/ directory is created"
    );
    // ...and it is absent from the JSON, not merely empty in it — a reader
    // of the previous format sees an unchanged document.
    let json = std::fs::read_to_string(&paths.manifest).unwrap();
    assert!(
        !json.contains("vtree_candidates"),
        "omitted, not emitted empty:\n{json}"
    );
    assert!(!json.contains("candidate_rank_metric"));
}

/// Asking for a candidate set emits one: ordered, entry 0 the selected vtree pointing
/// at the component's own files, every runner-up a real usable vtree in a
/// file of its own, and no two entries the same tree.
#[test]
fn a_requested_candidate_set_is_ranked_deduplicated_and_usable() {
    let formula = wide_component();
    let built = build_vtree(&formula, &candidates_config(4), &SelectionCtx::plain())
        .expect("the vtree must build");

    let dir = Scratch::new("candidates");
    let (m, paths) = write_components(
        dir.path(),
        &formula,
        &built,
        None,
        ComponentWriteOptions::default(),
    )
    .expect("must write");

    assert_eq!(
        m.candidate_rank_metric,
        Some(CandidateRankMetric::Cost),
        "a plain (non-projected) build ranks on the combined cost",
    );
    let set = &m.components[0].vtree_candidates;
    assert!(
        set.len() > 1,
        "the portfolio must retain more than the winner here"
    );
    assert!(
        set.len() <= 4,
        "the candidate set must honor the requested width"
    );

    // Entry 0 is the selected vtree and is NOT copied — it points at the
    // component's own files, which for a single component are the
    // whole-formula ones.
    assert_eq!(set[0].vtree, m.components[0].vtree);

    // The entries after the first ascend in the declared metric.
    assert!(
        set[1..]
            .windows(2)
            .all(|w| w[0].scores.cost <= w[1].scores.cost),
        "runners-up must be ordered by the declared metric, best first",
    );

    // Every runner-up is on disk, re-parses, and is a complete vtree over
    // this component's LOCAL space.
    let mut texts = vec![built.vtree.to_vtree_text()];
    for (i, e) in set.iter().enumerate().skip(1) {
        assert_eq!(
            e.vtree,
            format!("{CANDIDATES_DIR}/comp000.rank{i:02}.vtree")
        );
        let text = std::fs::read_to_string(dir.path().join(&e.vtree)).expect("runner-up on disk");
        let v = Vtree::from_vtree_text(&text).expect("runner-up parses");
        assert_eq!(
            v.num_leaves(),
            formula.num_vars,
            "a candidate is a COMPLETE vtree"
        );
        assert!(
            !e.built_by.is_empty(),
            "every entry names the spec(s) that produced it"
        );
        assert!(
            !texts.contains(&text),
            "structurally identical vtrees must be one entry"
        );
        texts.push(text);
    }
    assert_eq!(
        paths.candidates.len(),
        set.len() - 1,
        "one file per runner-up"
    );
}

/// Selection is decided before the candidate set exists and is unaffected by it: the
/// vtree a caller gets is the same whether or not one was requested.
#[test]
fn retaining_a_candidate_set_does_not_change_the_selected_vtree() {
    let formula = wide_component();
    let plain = build_vtree(&formula, &candidates_config(1), &SelectionCtx::plain())
        .expect("the vtree must build");
    let with_candidates = build_vtree(&formula, &candidates_config(4), &SelectionCtx::plain())
        .expect("the vtree must build");
    assert_eq!(
        plain.vtree.to_vtree_text(),
        with_candidates.vtree.to_vtree_text(),
        "asking for the runners-up must not change which candidate wins",
    );
}

/// The run config is where a caller states what construction may spend, and
/// both of those statements reach the portfolio: `candidates` decides how many
/// runners-up come back, and `deadline` bounds the construction.
#[test]
fn what_the_run_config_allows_reaches_the_portfolio() {
    let formula = wide_component();
    let retained = |b: &crate::component::VtreeBuild| {
        b.candidate_sets.first().map_or(0, |s| s.candidates.len())
    };

    let width = build_vtree(&formula, &candidates_config(4), &SelectionCtx::plain())
        .expect("the vtree must build");
    assert!(
        retained(&width) > 1,
        "the run config's candidate width must reach the portfolio",
    );

    // The same config with an already-spent deadline. The build still returns a
    // vtree — the portfolio gives its first candidate one short attempt rather
    // than skipping the whole catalog — but it obeys the deadline by leaving the
    // rest of the catalog unstarted, which a complete build never reports.
    let spent = RunConfig {
        deadline: Some(std::time::Instant::now()),
        ..candidates_config(1)
    };
    let bounded = build_vtree(&formula, &spent, &SelectionCtx::plain())
        .expect("a spent deadline must still hand back a vtree");
    assert!(
        !bounded.limits.skipped.is_empty(),
        "a spent deadline on the run config must bound the construction",
    );
}
