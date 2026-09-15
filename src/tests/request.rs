//! [`crate::request`]: the request read from JSON, the bundle held in memory,
//! the summary, and the JSON answer.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::bundle;
use crate::cnf::Mode;
use crate::config::{ComponentPolicy, RunConfig};
use crate::decompose::SelectionCtx;
use crate::error::VitriError;
use crate::request::{
    CAPABILITIES_FORMAT, REQUEST_KEYS, RESULT_FORMAT, Request, capabilities, capabilities_json,
    prepare, prepare_json,
};
use crate::tests::common::{FULLY_RESOLVED, IRREDUCIBLE_5, REFUTED, Scratch, parse};

/// Two components of three variables each.
const TWO_COMPONENTS: &str = "p cnf 6 6\n1 2 3 0\n-1 -2 0\n-2 -3 0\n4 5 6 0\n-4 -5 0\n-5 -6 0\n";

/// One cheap elimination order, so what these assert is the request path rather
/// than which candidate the portfolio picked.
fn minfill() -> Request {
    Request {
        vtree: Some("minfill-primal".to_string()),
        ..Request::default()
    }
}

fn config_error(json: &str) -> String {
    match Request::from_json(json) {
        Err(VitriError::Config { reason }) => reason,
        other => panic!("{json} should be refused as a config error, got {other:?}"),
    }
}

/// Every file under `dir`, keyed by its `/`-separated path relative to `dir`.
fn files_under(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(at) = pending.pop() {
        for entry in std::fs::read_dir(&at).expect("the scratch directory reads") {
            let path = entry.expect("the entry reads").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let rel = path.strip_prefix(dir).expect("under the scratch directory");
                let rel = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                found.insert(rel, std::fs::read(&path).expect("the file reads"));
            }
        }
    }
    found
}

#[test]
fn every_request_key_is_accepted_and_null_is_the_same_as_absent() {
    for key in REQUEST_KEYS {
        assert_eq!(
            Request::from_json(&format!("{{\"{key}\": null}}")),
            Ok(Request::default()),
            "{key} is listed but not read",
        );
    }
    assert_eq!(Request::from_json("{}"), Ok(Request::default()));
}

#[test]
fn a_request_sets_exactly_the_fields_it_names() {
    let request = Request::from_json(
        r#"{"format": "vitri-request-v1", "mode": "pmc", "vtree": "minfill-primal",
            "budget_ms": 500, "components": "whole", "candidates": 3,
            "simplify": false, "arjun": false, "dot": true}"#,
    )
    .expect("a request using every key parses");
    let mut config = RunConfig::default();
    request.apply_to(&mut config);
    assert_eq!(config.mode, Some(Mode::Pmc));
    assert_eq!(config.vtree_spec, "minfill-primal");
    assert_eq!(config.budget_ms, Some(500));
    assert_eq!(config.components, ComponentPolicy::Whole);
    assert_eq!(config.candidates, 3);
    assert!(!config.stages.simplify);
    assert!(!config.stages.arjun);
    assert!(request.write_options().dot);

    let mut untouched = RunConfig::from_env_defaults().expect("the test environment is clean");
    let before = format!("{untouched:?}");
    Request::default().apply_to(&mut untouched);
    assert_eq!(
        format!("{untouched:?}"),
        before,
        "an empty request must leave the configuration it is applied to as it was",
    );
}

#[test]
fn a_malformed_request_is_refused_naming_what_is_wrong() {
    for (json, names) in [
        ("not json", "not JSON"),
        ("[1, 2]", "JSON object"),
        (r#"{"threads": 2}"#, "\"threads\""),
        (r#"{"format": "vitri-request-v2"}"#, "\"vitri-request-v2\""),
        (r#"{"mode": "count"}"#, "mc, wmc, pmc, pwmc or compile"),
        (r#"{"components": "some"}"#, "split or whole"),
        (r#"{"budget_ms": -1}"#, "non-negative integer"),
        (r#"{"candidates": 1.5}"#, "non-negative integer"),
        (r#"{"vtree": 3}"#, "string"),
        (r#"{"dot": "yes"}"#, "true or false"),
    ] {
        let reason = config_error(json);
        assert!(
            reason.contains(names),
            "the refusal of {json} should name {names:?}: {reason}"
        );
    }
}

#[test]
fn prepared_files_are_the_bundle_the_directory_writer_writes() {
    for dot in [false, true] {
        let request = Request {
            simplify: Some(false),
            arjun: Some(false),
            dot,
            ..minfill()
        };
        let prepared = prepare(TWO_COMPONENTS.as_bytes(), &request).expect("the run prepares");

        let (formula, meta) = parse(TWO_COMPONENTS);
        let mut config = RunConfig::default();
        request.apply_to(&mut config);
        let run = bundle::run(&formula, &meta, &config, &SelectionCtx::plain())
            .expect("the same run succeeds directly");
        let dir = Scratch::new("request-files");
        run.write_to_dir(dir.path(), request.write_options())
            .expect("the run writes");

        let in_memory: BTreeMap<String, Vec<u8>> = prepared
            .files
            .iter()
            .map(|f| (f.path.clone(), f.contents.clone()))
            .collect();
        assert_eq!(
            in_memory.keys().collect::<Vec<_>>(),
            files_under(dir.path()).keys().collect::<Vec<_>>(),
            "memory and disk must hold the same paths (dot = {dot})",
        );
        assert_eq!(
            in_memory,
            files_under(dir.path()),
            "memory and disk must hold the same bytes (dot = {dot})",
        );
        assert!(
            in_memory.keys().any(|p| p.starts_with("components/")),
            "the fixture must exercise the per-component files",
        );
        assert_eq!(
            in_memory.keys().any(|p| p.ends_with(".dot")),
            dot,
            "a .dot is written exactly when the request asks",
        );
        assert_eq!(
            prepared.summary.files,
            prepared
                .files
                .iter()
                .map(|f| f.path.clone())
                .collect::<Vec<_>>(),
            "the summary lists the files in the order they were written",
        );
    }
}

#[test]
fn the_summary_reports_the_run_it_describes() {
    let prepared = prepare(IRREDUCIBLE_5.as_bytes(), &minfill()).expect("the run prepares");
    let summary = &prepared.summary;
    assert_eq!(summary.format, RESULT_FORMAT);
    assert_eq!(summary.status, "built");
    assert_eq!(summary.request.vtree, "minfill-primal");
    assert_eq!(summary.request.mode, summary.mode);
    let (formula, _) = parse(IRREDUCIBLE_5);
    assert_eq!(summary.input.variables, formula.num_vars);
    assert_eq!(summary.input.clauses, formula.clauses.len());
    let vtree = summary.vtree.expect("a built run reports its vtree");
    assert_eq!(vtree.leaves, summary.reduced.variables);

    let record: Value = serde_json::from_slice(
        &prepared
            .files
            .iter()
            .find(|f| f.path == bundle::PREPROCESS_RECORD_NAME)
            .expect("the record is among the files")
            .contents,
    )
    .expect("the record is JSON");
    assert_eq!(record["count_lift_pow2"], summary.lift.count_lift_pow2);
    assert_eq!(record["weight_lift"], summary.lift.weight_lift.as_str());

    let json: Value = serde_json::from_str(&summary.to_json()).expect("the summary is JSON");
    assert_eq!(json["format"], RESULT_FORMAT);
    assert!(json["stages"].is_object());
    assert!(json["request"]["budget_ms"].is_null());
}

#[test]
fn a_run_without_a_vtree_reports_no_vtree_and_writes_none() {
    for (dimacs, status) in [(FULLY_RESOLVED, "fully_resolved"), (REFUTED, "refuted")] {
        let prepared = prepare(dimacs.as_bytes(), &minfill()).expect("the run prepares");
        assert_eq!(prepared.summary.status, status);
        assert!(prepared.summary.vtree.is_none());
        assert_eq!(
            prepared.summary.files,
            [bundle::REDUCED_CNF_NAME, bundle::PREPROCESS_RECORD_NAME],
            "a {status} run writes the reduced formula and the record only",
        );
    }
}

#[test]
fn prepare_json_answers_with_the_files_or_the_error_kind() {
    let answer: Value = serde_json::from_str(&prepare_json(
        IRREDUCIBLE_5.as_bytes(),
        r#"{"vtree": "minfill-primal"}"#,
    ))
    .expect("the answer is JSON");
    assert_eq!(answer["ok"], true);
    let listed: Vec<&str> = answer["summary"]["files"]
        .as_array()
        .expect("the summary lists files")
        .iter()
        .map(|p| p.as_str().expect("a path is a string"))
        .collect();
    for path in &listed {
        assert!(
            answer["files"][path].is_string(),
            "{path} is listed but its text is missing"
        );
    }
    assert_eq!(
        answer["files"].as_object().map(|files| files.len()),
        Some(listed.len())
    );

    for (dimacs, request, kind) in [
        (IRREDUCIBLE_5, r#"{"mode": "count"}"#, "config"),
        (
            IRREDUCIBLE_5,
            r#"{"vtree": "no-such-construction"}"#,
            "spec",
        ),
        ("p cnf 2 1\n1 x 0\n", "{}", "input"),
    ] {
        let answer: Value =
            serde_json::from_str(&prepare_json(dimacs.as_bytes(), request)).expect("JSON");
        assert_eq!(answer["ok"], false, "{request} over {dimacs:?}");
        assert_eq!(answer["error"]["kind"], kind, "{request} over {dimacs:?}");
        assert!(answer["error"]["message"].is_string());
    }
}

#[test]
fn calls_from_two_threads_both_complete_with_the_same_bundle() {
    let [a, b] = std::array::from_fn(|_| {
        std::thread::spawn(|| prepare(IRREDUCIBLE_5.as_bytes(), &minfill()))
    })
    .map(|handle| {
        handle
            .join()
            .expect("the thread finished")
            .expect("the run prepares")
    });
    assert_eq!(a.files, b.files);
}

#[test]
fn capabilities_list_the_vocabularies_the_request_accepts() {
    let caps = capabilities();
    assert_eq!(caps.format, CAPABILITIES_FORMAT);
    assert_eq!(caps.request_keys, REQUEST_KEYS);
    assert_eq!(caps.modes, Mode::names().collect::<Vec<_>>());
    assert_eq!(
        caps.components,
        ComponentPolicy::names().collect::<Vec<_>>()
    );
    assert!(caps.vtree_bases.iter().any(|b| b == caps.default_vtree));
    for mode in &caps.modes {
        assert!(Request::from_json(&format!("{{\"mode\": \"{mode}\"}}")).is_ok());
    }
    let json: Value = serde_json::from_str(&capabilities_json()).expect("JSON");
    assert_eq!(json["max_candidates"], caps.max_candidates);
}
