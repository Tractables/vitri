//! Reading a bundle back: every type the two JSON artifacts are built from
//! parses from what it wrote.
//!
//! A consumer parses `preprocess.json` and `components.json` into THESE types
//! rather than into a hand-written copy that would drift from them, so the round
//! trip is as much a part of the published contract as the field names are. The
//! comparison is on the re-serialized JSON: the records deliberately carry no
//! `PartialEq`, and a value dropped or re-encoded on the way in shows up there
//! anyway.
//!
//! The cases that are not a plain "write it, read it": the entries the writer
//! OMITS (an absent key must arrive as the empty list or `None`, not as a parse
//! error), and [`OriginalTarget`], whose encoding is untagged and whose two
//! impls are therefore hand-written on both sides.

#[path = "../common/mod.rs"]
mod common;

use common::{full_record, sparse_record};
use vitri::bundle::PreprocessRecord;
use vitri::bundle::components::{
    COMPONENTS_FORMAT_TAG, CandidateEntry, ComponentEntry, ComponentsManifest, SelectionEntry,
    TreeDecompositionSummary,
};
use vitri::candidates::CandidateRankMetric;
use vitri::cnf::ShowSet;
use vitri::preprocess::{OriginalMap, OriginalTarget, VarMap};
use vitri::score::VtreeScores;

fn scores(show: Option<u32>) -> VtreeScores {
    VtreeScores {
        clause_load_stddev: 1.25,
        max_clause_load: 7,
        peak_context_width_all: 11,
        peak_context_width_show: show,
        cost: 4096,
    }
}

/// A manifest with a candidate set on one component and none on the other, so
/// both the written and the omitted spelling of `vtree_candidates` are parsed —
/// and a selection block with a tree-decomposition summary beside one without,
/// which are the two spellings of `selection`.
fn manifest() -> ComponentsManifest {
    ComponentsManifest {
        format: COMPONENTS_FORMAT_TAG.to_string(),
        free_vars_reduced_dimacs: vec![9],
        candidate_rank_metric: Some(CandidateRankMetric::PeakContextWidthShow),
        components: vec![
            ComponentEntry {
                local_to_reduced_dimacs: vec![1, 3, 5],
                show_vars_local_dimacs: Some(ShowSet::from_dimacs_ids(&[2]).expect("valid ids")),
                selection: Some(SelectionEntry {
                    winning_spec: "goatd".to_string(),
                    tree_decomposition: Some(TreeDecompositionSummary {
                        num_bags: 12,
                        treewidth: 4,
                    }),
                }),
                cnf: "components/comp001.cnf".to_string(),
                vtree: "components/comp001.vtree".to_string(),
                vtree_candidates: vec![
                    CandidateEntry {
                        built_by: vec!["goatd-minfill".to_string(), "goatd-mindeg".to_string()],
                        vtree: "components/comp001.vtree".to_string(),
                        scores: scores(Some(4)),
                    },
                    CandidateEntry {
                        built_by: vec!["flowcutter".to_string()],
                        vtree: "candidates/comp001-rank1.vtree".to_string(),
                        scores: scores(None),
                    },
                ],
            },
            ComponentEntry {
                local_to_reduced_dimacs: vec![2, 4],
                show_vars_local_dimacs: None,
                selection: Some(SelectionEntry {
                    winning_spec: "force".to_string(),
                    tree_decomposition: None,
                }),
                cnf: "components/comp002.cnf".to_string(),
                vtree: "components/comp002.vtree".to_string(),
                vtree_candidates: Vec::new(),
            },
        ],
    }
}

/// A manifest from a run that asked for no candidate set and whose vtree was
/// loaded rather than selected: the rank metric and the selection block are both
/// absent, which is those fields' `null`-free spelling.
fn manifest_without_candidates() -> ComponentsManifest {
    ComponentsManifest {
        format: COMPONENTS_FORMAT_TAG.to_string(),
        free_vars_reduced_dimacs: Vec::new(),
        candidate_rank_metric: None,
        components: vec![ComponentEntry {
            local_to_reduced_dimacs: vec![1, 2],
            show_vars_local_dimacs: None,
            selection: None,
            cnf: "reduced.cnf".to_string(),
            vtree: "vtree.vtree".to_string(),
            vtree_candidates: Vec::new(),
        }],
    }
}

#[test]
fn a_record_parses_from_what_it_wrote() {
    for record in [full_record(), sparse_record()] {
        let written = record.to_json_string();
        let read: PreprocessRecord =
            serde_json::from_str(&written).expect("the record must parse from its own output");
        assert_eq!(read.to_json_string(), written);
    }
}

#[test]
fn a_manifest_parses_from_what_it_wrote() {
    for m in [manifest(), manifest_without_candidates()] {
        let written = serde_json::to_string_pretty(&m).expect("a manifest serializes");
        let read: ComponentsManifest =
            serde_json::from_str(&written).expect("the manifest must parse from its own output");
        assert_eq!(
            serde_json::to_string_pretty(&read).expect("a manifest serializes"),
            written
        );
    }
}

/// The untagged map: a number, a boolean and `null` are three variants, and the
/// reader has to tell them apart by JSON type alone.
#[test]
fn each_original_map_entry_kind_parses_from_its_own_json_type() {
    for (json, expect) in [
        ("-3", OriginalTarget::Literal(-3)),
        ("3", OriginalTarget::Literal(3)),
        ("true", OriginalTarget::Constant(true)),
        ("false", OriginalTarget::Constant(false)),
        ("null", OriginalTarget::Free),
    ] {
        let parsed: OriginalTarget =
            serde_json::from_str(json).unwrap_or_else(|e| panic!("{json} must parse: {e}"));
        assert_eq!(parsed, expect, "for {json}");
    }
    let whole: OriginalMap = serde_json::from_str("[-3, 3, true, false, null]").expect("parses");
    assert_eq!(
        whole,
        OriginalMap::from_entries(vec![
            OriginalTarget::Literal(-3),
            OriginalTarget::Literal(3),
            OriginalTarget::Constant(true),
            OriginalTarget::Constant(false),
            OriginalTarget::Free,
        ])
    );
}

/// A fourth JSON type is not a fourth variant, and the error says which one
/// arrived — the encoding has no tag to blame it on.
#[test]
fn a_map_entry_of_any_other_type_is_refused() {
    let err = serde_json::from_str::<OriginalTarget>("\"1\"")
        .expect_err("a string is not one of the three kinds");
    assert!(err.to_string().contains("string"), "{err}");
}

/// The newtype maps promise a bare array on the wire, which has to survive the
/// read as well as the write.
#[test]
fn the_variable_maps_parse_as_bare_arrays() {
    let map: VarMap<vitri::cnf::Reduced, vitri::cnf::Original> =
        serde_json::from_str("[3, null, -2]").expect("parses");
    assert_eq!(map, VarMap::from_entries(vec![Some(3), None, Some(-2)]));
}
