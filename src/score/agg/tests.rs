//! The aggregate ranker: the five reductions against the tables an offline fit
//! built, the model file's refusals, and what the score does to a pick.

use std::path::Path;

use super::{
    AGG_VAR, AggModel, AggScore, Aggregate, CONFLICTING, MARGIN_VAR, agg_score, conflict, gather,
    margin_from_value, round_robin,
};
use crate::cnf::CnfFormula;
use crate::score::tables::{FEATURE_NAMES, Feature, Tables};
use crate::score::vtree_cost;
use crate::vtree::Vtree;

/// Where the fixtures sit: four (CNF, vtree) pairs from three panels of the
/// offline study, and the aggregates that study's own pipeline computed for
/// them.
const DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/agg");

/// The eleven cost terms all at 1 and nothing else, which has to reproduce
/// [`vtree_cost`] exactly.
const COST_MODEL: &str = r#"{"kind": "agg-linear", "intercept": 0.0,
 "terms": {"tight": 1.0, "excess_half": 1.0, "clause_load_bits": 1.0,
           "high_load_25": 1.0, "chain_3_40": 1.0, "join_neg_half": 1.0,
           "directional_half": 1.0, "output_gap_16": 1.0, "extreme_chain_4": 1.0,
           "extreme_join_32": 1.0, "successor_guard": 1.0},
 "features": []}"#;

/// The same, plus the largest inside width over the tree at weight 1. The two
/// d1 candidates below are 1.28 apart on cost and 5 apart on that width, in
/// opposite directions, so this model picks the other one.
const FLIP_MODEL: &str = r#"{"kind": "agg-linear", "intercept": 0.0,
 "terms": {"tight": 1.0, "excess_half": 1.0, "clause_load_bits": 1.0,
           "high_load_25": 1.0, "chain_3_40": 1.0, "join_neg_half": 1.0,
           "directional_half": 1.0, "output_gap_16": 1.0, "extreme_chain_4": 1.0,
           "extreme_join_32": 1.0, "successor_guard": 1.0},
 "features": [{"column": "inside_width", "agg": "max",
               "mean": 0.0, "sd": 1.0, "weight": 1.0}]}"#;

fn model(text: &str) -> AggModel {
    AggModel::from_json(Path::new("model.json"), text).expect("the model loads")
}

/// The linear kind's score on a scorable pair.
fn linear(vtree: &Vtree, formula: &CnfFormula, model: &AggModel) -> f64 {
    agg_score(vtree, formula, model)
        .expect("the pair is scorable")
        .scalar()
        .expect("the linear kind scores a candidate on its own")
}

/// One fixture tree: `<panel>_<instance>_<component>_<rank>`, whose CNF is the
/// same name without the rank.
fn pair(tree: &str) -> (CnfFormula, Vtree) {
    let stem = tree
        .rsplit_once('_')
        .expect("a fixture name ends in its rank")
        .0;
    let file =
        std::fs::File::open(Path::new(DATA).join(format!("{stem}.cnf"))).expect("the CNF is there");
    let (formula, _) =
        CnfFormula::from_dimacs(std::io::BufReader::new(file)).expect("the fixture CNF parses");
    let text = std::fs::read_to_string(Path::new(DATA).join(format!("{tree}.vtree")))
        .expect("the vtree is there");
    let vtree = Vtree::from_vtree_text(&text).expect("the fixture vtree parses");
    (formula, vtree)
}

/// Every ported column, reduced every way, on one fixture tree, keyed the way
/// the reference file names them: `<agg>__<column>`.
fn aggregates(tree: &str) -> std::collections::HashMap<String, f64> {
    let (formula, vtree) = pair(tree);
    let columns: Vec<Feature> = FEATURE_NAMES.iter().map(|&(_, f)| f).collect();
    let tables = Tables::build(&vtree, &formula, true, true);
    let mut gathered = gather(&vtree, &tables, &columns);
    let mut out = std::collections::HashMap::new();
    for ((name, _), values) in FEATURE_NAMES.iter().zip(&mut gathered) {
        for (agg_name, agg) in super::AGGREGATE_NAMES {
            out.insert(format!("{agg_name}__{name}"), agg.of(values));
        }
    }
    out
}

/// The five reductions of all 38 ported columns, on four trees from three
/// panels, against the numbers the offline pipeline wrote for the same trees.
///
/// The reference file prints six significant digits, so the comparison is
/// relative at 1e-4 — two orders of magnitude looser than the file's own
/// precision, and tight enough that a wrong reduction, a wrong column or a node
/// counted that should not be shows up. The one that would: the four cut
/// columns have no row at the root in either implementation, and including a
/// zero there moves every `mean` by a percent.
#[test]
fn the_aggregates_match_the_tables_the_offline_fit_was_built_from() {
    let path = Path::new(DATA).join("expected_aggregates.tsv");
    let text = std::fs::read_to_string(&path).expect("the expected aggregates are there");
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("tree\tcolumn\tagg\texpected"),
        "the expected aggregates have their header"
    );
    let mut cache: Option<(String, std::collections::HashMap<String, f64>)> = None;
    let mut checked = 0usize;
    for line in lines {
        let mut fields = line.split('\t');
        let (tree, column, agg, expected) = (
            fields.next().expect("tree"),
            fields.next().expect("column"),
            fields.next().expect("agg"),
            fields.next().expect("expected"),
        );
        let expected: f64 = expected.parse().expect("the expected value is a number");
        if cache.as_ref().is_none_or(|(at, _)| at != tree) {
            cache = Some((tree.to_string(), aggregates(tree)));
        }
        let got = cache
            .as_ref()
            .expect("the tree was just computed")
            .1
            .get(&format!("{agg}__{column}"))
            .copied()
            .unwrap_or_else(|| panic!("{tree}: no {agg} of {column}"));
        let tolerance = 1e-4 * expected.abs().max(1.0);
        assert!(
            (got - expected).abs() <= tolerance,
            "{tree} {agg} of {column}: {got} against the table's {expected}",
        );
        checked += 1;
    }
    assert_eq!(checked, 4 * 38 * 5, "every column was reduced every way");
}

/// The eleven weights at 1 and no aggregate is the structural cost itself —
/// the terms the ranker reads are the addends the cost sums, not a second
/// spelling of them.
#[test]
fn the_eleven_terms_at_weight_one_are_the_structural_cost() {
    let cost_model = model(COST_MODEL);
    for tree in [
        "d1_mc2025_track1_145_comp010_rank00",
        "k1_mc2023_track1_064_comp004_rank00",
        "v1_mc2026_track1_109_comp074_rank00",
    ] {
        let (formula, vtree) = pair(tree);
        let cost = vtree_cost(&vtree, &formula).expect("the pair is scorable");
        let scored = linear(&vtree, &formula, &cost_model);
        assert_eq!(scored, cost, "{tree}");
    }
}

/// The score decides the pick, and it is not the cost's: on two candidates of
/// one component the cost prefers the first and this model the second.
#[test]
fn a_model_that_reads_one_column_picks_against_the_cost() {
    let flip = model(FLIP_MODEL);
    let (cheap, wide) = (
        "d1_mc2025_track1_145_comp010_rank00",
        "d1_mc2025_track1_145_comp010_rank01",
    );
    let (formula, cheap_vtree) = pair(cheap);
    let (_, wide_vtree) = pair(wide);
    let cost_of = |v: &Vtree| vtree_cost(v, &formula).expect("the pair is scorable");
    let agg_of = |v: &Vtree| linear(v, &formula, &flip);
    assert!(
        cost_of(&cheap_vtree) < cost_of(&wide_vtree),
        "the cost pick is the first candidate",
    );
    assert!(
        agg_of(&wide_vtree) < agg_of(&cheap_vtree),
        "the ranker's argmin is the second",
    );
}

/// Percentiles interpolate linearly between the neighbouring order statistics,
/// which is what the fit's `numpy.percentile` did.
#[test]
fn the_percentiles_interpolate_the_way_numpy_does() {
    let of = |agg: Aggregate, values: &[f64]| agg.of(&mut values.to_vec());
    // Four values: the 90th percentile sits at 0.9 * 3 = 2.7, between 3 and 4.
    assert!((of(Aggregate::P90, &[1.0, 2.0, 3.0, 4.0]) - 3.7).abs() < 1e-12);
    assert!((of(Aggregate::P99, &[1.0, 2.0, 3.0, 4.0]) - 3.97).abs() < 1e-12);
    // Order does not matter, and a single value is every percentile of itself.
    assert!((of(Aggregate::P90, &[4.0, 1.0, 3.0, 2.0]) - 3.7).abs() < 1e-12);
    assert_eq!(of(Aggregate::P99, &[2.5]), 2.5);
}

/// `lse` reads the strictly positive entries and nothing else, and is 0 when
/// none of them is positive — the convention the offline tables were built
/// with.
#[test]
fn the_log_sum_exp_counts_only_the_positive_entries() {
    let of = |values: &[f64]| Aggregate::Lse.of(&mut values.to_vec());
    assert!((of(&[1.0, 1.0]) - 2.0).abs() < 1e-12);
    assert!((of(&[1.0, 1.0, 0.0, -3.0]) - 2.0).abs() < 1e-12);
    assert_eq!(of(&[0.0, 0.0]), 0.0);
    assert_eq!(of(&[]), 0.0);
}

/// An aggregate over no entry at all is 0, for every reduction.
#[test]
fn an_aggregate_over_nothing_is_zero() {
    let mut nothing: [f64; 0] = [];
    for agg in [
        Aggregate::Max,
        Aggregate::Mean,
        Aggregate::P90,
        Aggregate::P99,
        Aggregate::Lse,
    ] {
        assert_eq!(agg.of(&mut nothing), 0.0, "{agg:?}");
    }
}

/// Every way a model file can be wrong is refused, and the message names the
/// field that is wrong.
#[test]
fn a_model_file_this_crate_cannot_evaluate_is_refused_by_field() {
    let feature = |column: &str, agg: &str, sd: &str, weight: &str| {
        format!(
            r#"{{"kind": "agg-linear", "intercept": 0.0, "terms": {{}},
                 "features": [{{"column": "{column}", "agg": "{agg}",
                                "mean": 0.0, "sd": {sd}, "weight": {weight}}}]}}"#
        )
    };
    let cases: [(String, &str); 7] = [
        ("not json at all".to_string(), "not an aggregate ranker"),
        (
            r#"{"kind": "agg-quadratic", "features": []}"#.to_string(),
            "agg-quadratic",
        ),
        (
            r#"{"kind": "agg-linear", "terms": {"tightness": 1.0}, "features": []}"#.to_string(),
            "tightness",
        ),
        (feature("wingspan", "max", "1.0", "1.0"), "wingspan"),
        (feature("inside_width", "median", "1.0", "1.0"), "median"),
        (feature("inside_width", "max", "0.0", "1.0"), "sd is 0"),
        (feature("inside_width", "max", "-1.0", "1.0"), "sd is -1"),
    ];
    for (text, named) in cases {
        let message = AggModel::from_json(Path::new("m.json"), &text)
            .err()
            .unwrap_or_else(|| panic!("{text} is refused"));
        assert!(message.contains(named), "{text}: {message}");
        assert!(message.contains("m.json"), "{text}: {message}");
    }
}

/// A term the file does not name enters at 0 rather than at 1: the ranker sums
/// what it was fitted with, and a missing weight is a term the fit dropped.
#[test]
fn a_term_the_file_does_not_name_is_weight_zero() {
    let none = model(r#"{"kind": "agg-linear", "features": []}"#);
    let (formula, vtree) = pair("k1_mc2023_track1_064_comp004_rank00");
    assert_eq!(linear(&vtree, &formula, &none), 0.0,);
}

/// The margin reads as a margin, is refused without a ranker to narrow, and is
/// refused when it is not one.
#[test]
fn the_margin_needs_a_ranker_and_has_to_be_a_margin() {
    assert_eq!(margin_from_value(None, false).expect("unset is fine"), None);
    assert_eq!(
        margin_from_value(Some(" 0.5 "), true).expect("a margin reads"),
        Some(0.5),
    );
    assert_eq!(
        margin_from_value(Some("0"), true).expect("zero is a margin"),
        Some(0.0),
    );
    let lonely = margin_from_value(Some("0.5"), false)
        .expect_err("a margin with no ranker is refused")
        .to_string();
    assert!(lonely.contains(MARGIN_VAR), "{lonely}");
    assert!(lonely.contains(AGG_VAR), "{lonely}");
    for bad in ["wide", "-1", "inf"] {
        let message = margin_from_value(Some(bad), true)
            .expect_err("not a margin")
            .to_string();
        assert!(message.contains(MARGIN_VAR), "{bad}: {message}");
    }
}

/// Set beside a variable that decides the same pick, the ranker is refused
/// naming both — one of the two would otherwise be doing nothing.
#[test]
fn the_ranker_beside_another_pick_variable_is_refused_naming_both() {
    conflict(|_| false).expect("nothing else set");
    for other in CONFLICTING {
        let message = conflict(|name| name == other)
            .expect_err("the combination is refused")
            .to_string();
        assert!(message.contains(AGG_VAR), "{other}: {message}");
        assert!(message.contains(other), "{other}: {message}");
    }
}

// ---------------------------------------------------------------------------
// The boosted kind
// ---------------------------------------------------------------------------

/// Two inputs, two trees: the first splits on the first input at 1.0, the
/// second is a bare leaf. Hand-walkable.
const TINY_BOOST: &str = r#"{"kind": "agg-pair-boost", "baseline": 0.5,
 "inputs": [{"term": "tight"}, {"column": "inside_width", "agg": "max"}],
 "trees": [{"nodes": [{"feature": 0, "threshold": 1.0, "left": 1, "right": 2},
                      {"value": -1.0}, {"value": 1.0}]},
           {"nodes": [{"value": 0.25}]}]}"#;

/// A boosted file is walked the way its node tables say: `<=` goes left, the
/// baseline and one leaf per tree are summed.
#[test]
fn a_boosted_file_is_walked_as_its_trees_say() {
    let m = model(TINY_BOOST);
    assert!(m.is_pairwise());
    assert_eq!(m.raw_pair(&[1.0, 7.0]), 0.5 - 1.0 + 0.25);
    assert_eq!(m.raw_pair(&[1.5, 7.0]), 0.5 + 1.0 + 0.25);
}

/// The round robin: each candidate's mean probability of being the larger
/// against each sibling, from the differences of their inputs; a lone
/// candidate scores 0, and the score falls with the first input here.
#[test]
fn the_round_robin_scores_a_candidate_against_each_sibling() {
    let m = model(TINY_BOOST);
    let sigmoid = |z: f64| 1.0 / (1.0 + (-z).exp());
    let a = [0.0, 0.0];
    let b = [2.0, 0.0];
    let c = [4.0, 0.0];
    let scores = round_robin(&m, &[&a, &b, &c]);
    // a - b = -2 and a - c = -4: both go left, raw -0.25.
    let left = sigmoid(-0.25);
    let right = sigmoid(1.75);
    assert!((scores[0] - left).abs() < 1e-12, "{scores:?}");
    // b - a = 2 (right), b - c = -2 (left).
    assert!(
        (scores[1] - (right + left) / 2.0).abs() < 1e-12,
        "{scores:?}"
    );
    assert!((scores[2] - right).abs() < 1e-12, "{scores:?}");
    assert!(scores[0] < scores[1] && scores[1] < scores[2]);
    assert_eq!(round_robin(&m, &[&a]), vec![0.0]);
}

/// On a real pair the boosted kind hands back its inputs — the cost addends by
/// name and the aggregates raw — one per entry of the file's input list, and
/// those numbers are the linear kind's own.
#[test]
fn the_boosted_kind_carries_the_inputs_the_linear_kind_sums() {
    let m = model(TINY_BOOST);
    let (formula, vtree) = pair("d1_mc2025_track1_145_comp010_rank00");
    let AggScore::Inputs(inputs) = agg_score(&vtree, &formula, &m).expect("scorable") else {
        panic!("the boosted kind carries inputs");
    };
    assert_eq!(inputs.len(), 2);
    let tight = model(r#"{"kind": "agg-linear", "terms": {"tight": 1.0}, "features": []}"#);
    assert_eq!(inputs[0], linear(&vtree, &formula, &tight));
    let width = model(
        r#"{"kind": "agg-linear", "features": [{"column": "inside_width", "agg": "max",
             "mean": 0.0, "sd": 1.0, "weight": 1.0}]}"#,
    );
    assert_eq!(inputs[1], linear(&vtree, &formula, &width));
}

/// The evaluator against the exporter's own numbers: a twenty-tree model fitted
/// offline over the 201 inputs, and one component's input matrix with the
/// scores the exporter computed from the same file.
#[test]
fn the_boosted_kind_reproduces_the_exporters_round_robin() {
    let text = std::fs::read_to_string(Path::new(DATA).join("boost_model.json"))
        .expect("the model is there");
    let m = AggModel::from_json(Path::new("boost_model.json"), &text).expect("it loads");
    let fixture: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(DATA).join("boost_fixture.json"))
            .expect("the fixture is there"),
    )
    .expect("the fixture parses");
    let number = |v: &serde_json::Value| v.as_f64().expect("a number");
    let matrix: Vec<Vec<f64>> = fixture["matrix"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row.as_array().expect("a row").iter().map(number).collect())
        .collect();
    let expected: Vec<f64> = fixture["scores"]
        .as_array()
        .expect("scores")
        .iter()
        .map(number)
        .collect();
    assert_eq!(matrix.len(), expected.len());
    assert_eq!(
        matrix[0].len(),
        fixture["inputs"].as_u64().expect("a count") as usize
    );
    let inputs: Vec<&[f64]> = matrix.iter().map(Vec::as_slice).collect();
    let scores = round_robin(&m, &inputs);
    for (at, (got, want)) in scores.iter().zip(&expected).enumerate() {
        assert!((got - want).abs() < 1e-9, "candidate {at}: {got} vs {want}");
    }
}

/// Every way a boosted file can be wrong is refused, and the message names the
/// field that is wrong.
#[test]
fn a_boosted_file_this_crate_cannot_evaluate_is_refused_by_field() {
    let file = |inputs: &str, trees: &str| {
        format!(
            r#"{{"kind": "agg-pair-boost", "baseline": 0.0, "inputs": {inputs}, "trees": {trees}}}"#
        )
    };
    let two = r#"[{"term": "tight"}, {"column": "inside_width", "agg": "max"}]"#;
    let leaf = r#"[{"nodes": [{"value": 0.0}]}]"#;
    let cases: [(String, &str); 9] = [
        (file("[]", leaf), "inputs is empty"),
        (file(two, "[]"), "trees is empty"),
        (file(r#"[{"term": "tightness"}]"#, leaf), "tightness"),
        (file(r#"[{"column": "wingspan", "agg": "max"}]"#, leaf), "wingspan"),
        (file(r#"[{"term": "tight", "column": "inside_width", "agg": "max"}]"#, leaf), "inputs[0]"),
        (
            file(two, r#"[{"nodes": [{"feature": 2, "threshold": 1.0, "left": 1, "right": 2}, {"value": 0.0}, {"value": 0.0}]}]"#),
            "feature 2 is out of range",
        ),
        (
            file(two, r#"[{"nodes": [{"feature": 0, "threshold": 1.0, "left": 1, "right": 5}, {"value": 0.0}]}]"#),
            "index the tree's 2 nodes",
        ),
        (
            file(two, r#"[{"nodes": [{"feature": 0, "threshold": 1.0, "left": 0, "right": 1}, {"value": 0.0}]}]"#),
            "after their parent",
        ),
        (
            r#"{"kind": "agg-pair-boost", "inputs": [{"term": "tight"}], "trees": [{"nodes": [{"value": 0.0}]}],
                "features": [{"column": "inside_width", "agg": "max", "mean": 0.0, "sd": 1.0, "weight": 1.0}]}"#
                .to_string(),
            "belong to",
        ),
    ];
    for (text, named) in cases {
        let message = AggModel::from_json(Path::new("m.json"), &text)
            .err()
            .unwrap_or_else(|| panic!("{text} is refused"));
        assert!(message.contains(named), "{text}: {message}");
        assert!(message.contains("m.json"), "{text}: {message}");
    }
}
