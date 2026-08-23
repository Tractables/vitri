//! The on-disk shape of `preprocess.json`, pinned byte for byte.
//!
//! The record is a published contract — a consumer parses these exact field
//! names and this exact variable-map encoding — so the guard is a literal, not a
//! set of structural assertions that would pass on a renamed field. The record
//! here is written by hand rather than produced by a run: what is under
//! test is the serialization, and a hand-written record can carry the map
//! entries a small fixture rarely produces (a `null` for a variable the
//! preprocessing introduced, a negative entry for a polarity flip, and each of the
//! three JSON types the original→reduced map uses).
//!
//! An intentional format change bumps `RECORD_FORMAT_TAG` — and updates this
//! expectation with it.

#[path = "../common/mod.rs"]
mod common;

use common::{full_record, sparse_record};
use vitri::bundle::{PreprocessRecord, RECORD_FORMAT_TAG};
use vitri::cnf::Mode;
use vitri::preprocess::{OriginalMap, OriginalTarget, VarMap};

const EXPECTED: &str = r#"{
  "format": "vitri-preprocess-v1",
  "mode": "pwmc",
  "count_lift_pow2": 3,
  "weight_lift": "7/8",
  "original_num_vars": 4,
  "reduced_to_original_dimacs": [
    3,
    null,
    -2
  ],
  "original_to_reduced_dimacs": [
    -3,
    3,
    false,
    null
  ],
  "forced_literals_original_dimacs": [
    -4
  ],
  "free_vars_original_dimacs": [
    2
  ],
  "unsat": false,
  "show_vars_reduced_dimacs": [
    1,
    3
  ],
  "reduced_weights": [
    {
      "literal": -1,
      "weight": "1/2"
    }
  ]
}"#;

#[test]
fn the_record_serializes_to_its_published_shape() {
    assert_eq!(full_record().to_json_string(), EXPECTED);
}

/// The three entry kinds are three different JSON types, which is what lets a
/// consumer tell them apart without a tag: a number for a reduced literal, a
/// boolean for a constant, `null` for a variable nothing constrains.
#[test]
fn the_original_map_writes_one_json_type_per_entry_kind() {
    let record = PreprocessRecord {
        format: RECORD_FORMAT_TAG.to_string(),
        mode: Mode::Compile,
        count_lift_pow2: 1,
        weight_lift: "1/1".to_string(),
        original_num_vars: 4,
        reduced_to_original_dimacs: VarMap::from_entries(vec![Some(1)]),
        original_to_reduced_dimacs: Some(OriginalMap::from_entries(vec![
            OriginalTarget::Literal(1),
            OriginalTarget::Constant(true),
            OriginalTarget::Constant(false),
            OriginalTarget::Free,
        ])),
        forced_literals_original_dimacs: vec![2, -3],
        free_vars_original_dimacs: vec![4],
        unsat: false,
        show_vars_reduced_dimacs: None,
        reduced_weights: None,
    };
    assert!(
        record.to_json_string().contains(
            "\"original_to_reduced_dimacs\": [\n    1,\n    true,\n    false,\n    null\n  ]"
        ),
        "{}",
        record.to_json_string(),
    );
}

/// The lists that are absent rather than empty on the wire: a consumer that
/// treats a missing key as an error must not see one where the chain simply had
/// nothing to report.
#[test]
fn empty_optional_lists_are_omitted_entirely() {
    let json = sparse_record().to_json_string();
    for absent in [
        "original_to_reduced_dimacs",
        "forced_literals_original_dimacs",
        "free_vars_original_dimacs",
        "show_vars_reduced_dimacs",
        "reduced_weights",
    ] {
        assert!(
            !json.contains(absent),
            "{absent} must be omitted, got:\n{json}"
        );
    }
    assert!(
        json.contains("\"reduced_to_original_dimacs\": [\n    1\n  ]"),
        "{json}"
    );
}
