//! End-to-end unit tests for goatd Config A.

use crate::decompose::goatd::graph::Graph;
use crate::decompose::goatd::preprocess::preprocess;
use crate::decompose::goatd::width_opt::Config;
use crate::decompose::{GraphKind, TreeDecomposition, build_primal_edges};
use crate::tests::td_fixture::{GraphAtItsWidth, assert_valid_td};

use super::width_opt::run_config;

fn run_primal(num_vars: u32, edges: &[(u32, u32)]) -> TreeDecomposition {
    run_config(
        GraphKind::Primal,
        num_vars,
        edges,
        num_vars,
        Config::MinFill,
        0,
    )
    .td
}

#[test]
fn triangle_is_width_two() {
    let edges = vec![(0, 1), (0, 2), (1, 2)];
    let td = run_primal(3, &edges);
    assert_eq!(td.treewidth(), 2);
    assert_valid_td(&td, 3, &edges);
}

#[test]
fn path_is_width_one() {
    let edges = vec![(0, 1), (1, 2), (2, 3), (3, 4)];
    let td = run_primal(5, &edges);
    assert_eq!(td.treewidth(), 1);
    assert_valid_td(&td, 5, &edges);
}

#[test]
fn cycle_four_is_width_two() {
    // C_4 has treewidth 2; requires one fill edge.
    let edges = vec![(0, 1), (1, 2), (2, 3), (3, 0)];
    let td = run_primal(4, &edges);
    assert_eq!(td.treewidth(), 2);
    assert_valid_td(&td, 4, &edges);
}

#[test]
fn three_tree_has_width_three() {
    // 3-tree on 5 vertices: start from K_4 {0,1,2,3}, then attach vertex 4
    // connected to three existing neighbours {0,1,2}.
    let edges = vec![
        (0, 1),
        (0, 2),
        (0, 3),
        (1, 2),
        (1, 3),
        (2, 3),
        (0, 4),
        (1, 4),
        (2, 4),
    ];
    let td = run_primal(5, &edges);
    assert_eq!(td.treewidth(), 3);
    assert_valid_td(&td, 5, &edges);
}

#[test]
fn disconnected_forest_covered() {
    let edges = vec![(0, 1), (2, 3)];
    let td = run_primal(4, &edges);
    assert_valid_td(&td, 4, &edges);
    assert!(td.treewidth() <= 1);
}

#[test]
fn preprocessing_preserves_covering() {
    // Series-reducible pentagon: preprocessing should completely eliminate
    // every vertex via series reductions.
    let edges = vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)];
    let graph = Graph::from_edges(5, &edges);
    let reduced = preprocess(graph);
    assert_eq!(reduced.prefix.bags.len(), 5);
    assert_eq!(reduced.graph.num_active, 0);
}

#[test]
fn small_cnf_round_trip() {
    use crate::cnf::{Clause, CnfFormula, Literal};
    use crate::vtree::VarId;
    let formula = CnfFormula {
        num_vars: 4,
        clauses: vec![
            Clause {
                literals: vec![
                    Literal {
                        var: VarId(0),
                        positive: true,
                    },
                    Literal {
                        var: VarId(1),
                        positive: false,
                    },
                ],
            },
            Clause {
                literals: vec![
                    Literal {
                        var: VarId(1),
                        positive: true,
                    },
                    Literal {
                        var: VarId(2),
                        positive: true,
                    },
                ],
            },
            Clause {
                literals: vec![
                    Literal {
                        var: VarId(2),
                        positive: false,
                    },
                    Literal {
                        var: VarId(3),
                        positive: true,
                    },
                ],
            },
        ],
    };
    let edges = build_primal_edges(&formula);
    let td = run_config(GraphKind::Primal, 4, &edges, 4, Config::MinFill, 0).td;
    assert_valid_td(&td, 4, &edges);

    // Go all the way through the public API too.
    let built =
        crate::decompose::goatd::vtree_from_goatd_best(&formula, 0, 1.0).expect("goatd build");
    assert_eq!(built.vtree.num_leaves(), 4);
}

#[test]
fn multi_seed_produces_valid_tds() {
    let edges = vec![(0, 1), (0, 2), (1, 3), (2, 3), (3, 4)];
    for seed in 0..4u64 {
        let td = run_config(GraphKind::Primal, 5, &edges, 5, Config::MinFill, seed).td;
        assert_valid_td(&td, 5, &edges);
    }
}

// Candidate = (label, width, total_bag_size, cost). The refined path keys
// on `(width, bagsize)`; the scored path keys on `(width, cost, bagsize)`.
// These pin that the two documented orderings disagree on the same input and
// that ties keep first-occurrence-wins.
use crate::decompose::best::select_first_min;
use crate::decompose::goatd::refined_select_key;

type Cand = (&'static str, u32, usize, u64);

/// The scored path's key, mirroring the tuple `best_vtree_over_schedule`
/// builds inline: lowest width wins, ties broken by cost then bag size.
fn scored_select_key(width: u32, bagsize: usize, cost: u64) -> (u64, u64, u64) {
    (width as u64, cost, bagsize as u64)
}

#[test]
fn refined_key_orders_by_width_then_bagsize() {
    // Same width; A must prefer the smaller bagsize regardless of cost.
    let cands: Vec<Cand> = vec![
        ("small_bag_high_qual", 5, 10, 100),
        ("big_bag_low_qual", 5, 20, 50),
    ];
    let winner = select_first_min(cands.iter().copied(), |&(_, w, b, _)| {
        refined_select_key(w, b)
    })
    .unwrap();
    assert_eq!(winner.0, "small_bag_high_qual");
}

#[test]
fn scored_key_orders_by_width_then_cost() {
    // Same input as the refined test; the scored path breaks the tie on
    // cost (2nd field), so it picks the OTHER candidate.
    let cands: Vec<Cand> = vec![
        ("small_bag_high_qual", 5, 10, 100),
        ("big_bag_low_qual", 5, 20, 50),
    ];
    let winner = select_first_min(cands.iter().copied(), |&(_, w, b, q)| {
        scored_select_key(w, b, q)
    })
    .unwrap();
    assert_eq!(winner.0, "big_bag_low_qual");
}

#[test]
fn select_first_min_breaks_ties_toward_first() {
    // Two candidates with identical refined keys: the FIRST listed wins,
    // matching `min_by_key`'s tie-break rule.
    let cands: Vec<Cand> = vec![("first", 7, 15, 1), ("second", 7, 15, 2)];
    let winner = select_first_min(cands.iter().copied(), |&(_, w, b, _)| {
        refined_select_key(w, b)
    })
    .unwrap();
    assert_eq!(winner.0, "first");
}

/// The refine-budget override's own spellings: absent and 0 both mean "take the
/// caller's share", a positive count is that budget, and anything else is an
/// error naming the variable rather than a silent fall back to the caller's.
#[test]
fn refine_budget_parsing() {
    use crate::decompose::goatd::refine_budget_ms;
    assert_eq!(refine_budget_ms(None), Ok(None));
    assert_eq!(refine_budget_ms(Some("0")), Ok(None));
    assert_eq!(refine_budget_ms(Some(" 2500 ")), Ok(Some(2500)));
    for bad in ["", "later", "-1", "2.5"] {
        let err = refine_budget_ms(Some(bad)).expect_err("{bad:?} should fail fast");
        assert!(
            err.to_string().contains("VITRI_GOATD_REFINE_BUDGET_MS"),
            "the error must name the variable, got: {err}"
        );
    }
}

/// The refined-best schedule budget resolves in ONE place
/// (`ModeConfig::compile_refined`) from two inputs: the explicit override, else
/// the caller's budget. Asserted at the config layer so it needs no schedule
/// run; `run_schedule` derives its soft deadline directly from this
/// `timeout_ms` (soft at `start+N`, hard at `start+2N`). The override now
/// arrives as a value rather than out of the process environment, so all three
/// cases are pinned here instead of one of them being skipped when the variable
/// happened to be set in the test process.
#[test]
fn compile_refined_resolves_the_schedule_budget() {
    use crate::decompose::goatd::{GoatdKnobs, ModeConfig};
    let knobs = |refine_budget_ms| GoatdKnobs { refine_budget_ms };
    assert_eq!(
        ModeConfig::compile_refined(Some(12_345), knobs(None)).timeout_ms,
        Some(12_345),
        "a caller budget must become the schedule's soft deadline",
    );
    assert_eq!(
        ModeConfig::compile_refined(None, knobs(None)).timeout_ms,
        None,
        "no override and no caller budget ⇒ run to completion, as before",
    );
    assert_eq!(
        ModeConfig::compile_refined(Some(12_345), knobs(Some(500))).timeout_ms,
        Some(500),
        "the explicit override wins over the caller's budget",
    );
}

/// The six smallest shapes a decomposer meets — one vertex, a path, a cycle, a
/// star, a complete graph and a forest with an isolated vertex — through every
/// elimination config there is.
///
/// The width bound is the shape's own treewidth, and no decomposition can go
/// below it, so an upper bound pins it exactly.
#[test]
fn every_elimination_config_decomposes_the_tiny_graph_family() {
    let shapes: &[GraphAtItsWidth] = &[
        ("one vertex", 1, &[], 0),
        ("a path", 5, &[(0, 1), (1, 2), (2, 3), (3, 4)], 1),
        ("a cycle", 4, &[(0, 1), (1, 2), (2, 3), (3, 0)], 2),
        ("a star", 4, &[(0, 1), (0, 2), (0, 3)], 1),
        (
            "a complete graph",
            4,
            &[(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)],
            3,
        ),
        ("a forest with an isolated vertex", 5, &[(0, 1), (2, 3)], 1),
    ];

    for &(shape, num_vars, edges, treewidth) in shapes {
        let weight = vec![1u32; num_vars as usize];
        let configs: [(&str, Config<'_>); 5] = [
            ("min-fill", Config::MinFill),
            ("min-degree", Config::MinDegree),
            ("nested dissection", Config::NestedDissMinCover),
            (
                "sampled min-fill",
                Config::MinFillSampleJW { weight: &weight },
            ),
            (
                "sampled min-degree",
                Config::MinDegreeSampleJW { weight: &weight },
            ),
        ];
        for (config_name, config) in configs {
            let td = run_config(GraphKind::Primal, num_vars, edges, num_vars, config, 0).td;
            assert_valid_td(&td, num_vars, edges);
            assert!(
                td.treewidth() <= treewidth,
                "{config_name} on {shape} must reach width {treewidth}, got {}",
                td.treewidth(),
            );
        }
    }
}

/// A sampling elimination breaks its ties by a seeded draw, so one seed has to
/// give one decomposition every time — otherwise a decomposition cannot be
/// reproduced from the spec that names it. Across seeds the draws differ, which
/// is what makes running several of them worth anything.
#[test]
fn a_seeded_sampling_elimination_repeats_its_decomposition() {
    // A 3x4 grid: symmetric enough that every step has a tie set to draw from,
    // and dense enough that preprocessing does not reduce it away first.
    let mut edges: Vec<(u32, u32)> = Vec::new();
    for row in 0..3u32 {
        for col in 0..4u32 {
            let v = row * 4 + col;
            if col + 1 < 4 {
                edges.push((v, v + 1));
            }
            if row + 1 < 3 {
                edges.push((v, v + 4));
            }
        }
    }
    let weight: Vec<u32> = (0..12u32).map(|v| v + 1).collect();
    let sampled = |seed: u64| {
        run_config(
            GraphKind::Primal,
            12,
            &edges,
            12,
            Config::MinFillSampleJW { weight: &weight },
            seed,
        )
        .td
    };

    for seed in [0u64, 7, 99] {
        assert_eq!(
            sampled(seed).bags,
            sampled(seed).bags,
            "seed {seed} must draw the same decomposition twice",
        );
    }

    let first = sampled(0);
    assert!(
        (1..24u64).any(|seed| sampled(seed).bags != first.bags),
        "the seeds must not all collapse onto one decomposition",
    );
}
