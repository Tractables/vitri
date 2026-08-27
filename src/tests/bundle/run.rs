//! The whole pipeline through its one entry point: what [`crate::bundle::run`]
//! produces, and what [`crate::bundle::VitriRun::write_to_dir`] leaves on disk.
//!
//! Every item under test is `pub`, so these live in the crate-root test tree
//! rather than beside the module.

use super::*;

use std::sync::Arc;

use crate::bundle::components::{COMPONENTS_DIR, COMPONENTS_JSON_NAME, ComponentWriteOptions};
use crate::component::{ComponentVtree, VtreeBuild};
use crate::decompose::SelectionCtx;
use crate::error::VitriError;
use crate::score::StructureProfile;
use crate::spec::SelectionRecord;
use crate::tests::common::{FULLY_RESOLVED, IRREDUCIBLE_5, REFUTED, make_formula};
use crate::vtree::{VarId, Vtree};

/// A wider instance of the same shape as [`IRREDUCIBLE_5`], for the case that
/// needs two runs whose output differs.
const IRREDUCIBLE_WIDER: &str = "p cnf 7 6\n1 2 0\n-1 3 0\n-2 -3 4 0\n2 3 -4 0\n4 5 0\n-5 6 7 0\n";

/// One cheap elimination order, so what these assert is the pipeline rather
/// than which candidate the portfolio happened to pick.
fn config() -> RunConfig {
    RunConfig {
        vtree_spec: "minfill-primal".to_string(),
        ..RunConfig::default()
    }
}

fn run_on(dimacs: &str) -> VitriRun {
    let (formula, meta) = parse(dimacs);
    run(&formula, &meta, &config(), &SelectionCtx::plain()).expect("the run must produce a bundle")
}

#[test]
fn run_and_frontend_session_prepare_the_same_result() {
    let (formula, meta) = parse(IRREDUCIBLE_5);
    let config = config();
    let selection = SelectionCtx::plain();

    let direct = run(&formula, &meta, &config, &selection).expect("run must succeed");
    let mut session = frontend(&formula, &meta, &config, &selection)
        .expect("the frontend session must be created");
    let prepared = session.prepare().expect("the session must prepare");

    assert_eq!(
        direct.preprocessed.reduced, prepared.preprocessed.reduced,
        "the convenience call and the session must export the same reduced formula",
    );
    assert_eq!(
        serde_json::to_value(&direct.preprocessed.record).expect("the direct record serializes"),
        serde_json::to_value(&prepared.preprocessed.record).expect("the session record serializes"),
        "the convenience call and the session must export the same record",
    );
    match (&direct.vtree, &prepared.vtree) {
        (RunVtree::Built(a), RunVtree::Built(b)) => assert!(
            a.vtree.same_tree(&b.vtree),
            "the convenience call and the session must build the same vtree",
        ),
        (RunVtree::FullyResolved, RunVtree::FullyResolved) => {}
        _ => panic!("the convenience call and the session disagreed on whether a vtree exists"),
    }
    assert_eq!(
        direct.source_profile, prepared.source_profile,
        "the convenience call and the session must report the same raw profile",
    );
}

#[test]
fn a_frontend_session_refuses_a_second_prepare() {
    let (formula, meta) = parse(IRREDUCIBLE_5);
    let config = config();
    let mut session = frontend(&formula, &meta, &config, &SelectionCtx::plain())
        .expect("the frontend session must be created");
    session.prepare().expect("the first prepare must succeed");

    let err = session
        .prepare()
        .expect_err("a second prepare must not repeat preprocessing");
    assert!(
        matches!(err, VitriError::Config { .. }),
        "reusing a one-attempt session is a caller error, got: {err:?}",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("FrontendSession::prepare") && msg.contains("at most once"),
        "the refusal must name the call and its one-attempt contract, got: {msg}",
    );
}

#[test]
fn a_run_owns_the_raw_formula_profile() {
    let (formula, meta) = parse(IRREDUCIBLE_WIDER);
    let measured = StructureProfile::measure(&formula);
    let deliberately_wrong = StructureProfile::from_coefficients(99.0, 99.0);
    assert_ne!(
        measured, deliberately_wrong,
        "the test profile must be wrong"
    );

    let mut selection = SelectionCtx::plain();
    selection.source_profile = Some(deliberately_wrong);
    let produced = run(&formula, &meta, &config(), &selection)
        .expect("the run must own and measure the raw formula's profile");

    assert_eq!(
        produced.source_profile, measured,
        "the full run must report its own raw-input measurement, not a caller's profile",
    );
}

#[test]
fn one_run_call_produces_the_bundle_and_the_vtree_over_what_it_left() {
    let produced = run_on(IRREDUCIBLE_5);
    let build = produced
        .built()
        .expect("an irreducible instance leaves variables to build a vtree over");
    assert_eq!(
        build.vtree.num_leaves(),
        produced.preprocessed.reduced.num_vars,
        "the vtree must span exactly the formula preprocessing left",
    );

    let dir = Scratch::new("run-both-halves");
    let paths = produced
        .write_to_dir(dir.path(), ComponentWriteOptions::default())
        .expect("the run must write");
    let vtree = paths
        .vtree
        .as_ref()
        .expect("a run that built a vtree names its files");
    for named in [
        &paths.bundle.reduced_cnf,
        &paths.bundle.record,
        &vtree.vtree,
        &vtree.components.paths.manifest,
    ] {
        assert!(
            named.exists(),
            "{} is named in the result but was not written",
            named.display(),
        );
    }
}

#[test]
fn a_fully_resolved_run_writes_the_bundle_and_names_no_vtree() {
    let produced = run_on(FULLY_RESOLVED);
    assert_eq!(
        produced.preprocessed.reduced.num_vars, 0,
        "the fixture must really resolve outright",
    );
    assert!(
        matches!(produced.vtree, RunVtree::FullyResolved),
        "a resolved instance reports an outcome, not a failure",
    );
    assert!(produced.built().is_none());
    assert!(
        produced.preprocessed.telemetry.simplify_ms.is_some(),
        "fully resolving the formula must retain the preprocessing telemetry",
    );

    let dir = Scratch::new("run-resolved");
    let paths = produced
        .write_to_dir(dir.path(), ComponentWriteOptions::default())
        .expect("the bundle must still be written");
    assert!(
        paths.vtree.is_none(),
        "there is no vtree half to name paths in",
    );
    assert!(paths.bundle.reduced_cnf.exists());
    assert!(paths.bundle.record.exists());
    for absent in [VTREE_NAME, COMPONENTS_JSON_NAME] {
        assert!(
            !dir.path().join(absent).exists(),
            "{absent} describes a vtree this run did not build",
        );
    }
}

/// The one flag combination that is not refused up front: asking for a picture
/// of a vtree an instance turns out not to need is an ordinary run that writes
/// no picture, because the resolved return precedes any picture.
#[test]
fn a_fully_resolved_run_asked_for_a_picture_writes_none() {
    let produced = run_on(FULLY_RESOLVED);
    let dir = Scratch::new("run-resolved-dot");
    let paths = produced
        .write_to_dir(dir.path(), ComponentWriteOptions { dot: true })
        .expect("asking for a picture must not fail the run");
    assert!(paths.vtree.is_none());
    let pictures: Vec<_> = std::fs::read_dir(dir.path())
        .expect("the bundle directory must exist")
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .filter(|n| {
            std::path::Path::new(n)
                .extension()
                .is_some_and(|x| x == "dot")
        })
        .collect();
    assert!(
        pictures.is_empty(),
        "no vtree was built, so there is nothing to picture, got {pictures:?}",
    );
}

/// A picture is the vtree file's sibling — the same stem with a `.dot`
/// extension — and it is written only when one was asked for.
#[test]
fn a_picture_is_written_as_the_vtree_files_sibling() {
    let produced = run_on(IRREDUCIBLE_5);

    let plain = Scratch::new("run-no-picture");
    let paths = produced
        .write_to_dir(plain.path(), ComponentWriteOptions::default())
        .expect("the run must write");
    assert_eq!(
        paths.vtree.as_ref().expect("a vtree was built").dot,
        None,
        "no picture was asked for, so none is named",
    );

    let with_dot = Scratch::new("run-picture");
    let paths = produced
        .write_to_dir(with_dot.path(), ComponentWriteOptions { dot: true })
        .expect("the run must write");
    let vtree = paths.vtree.as_ref().expect("a vtree was built");
    assert_eq!(
        vtree.dot.as_deref(),
        Some(vtree.vtree.with_extension("dot").as_path()),
        "the picture is the vtree file's sibling",
    );
    assert!(vtree.dot.as_ref().is_some_and(|p| p.exists()));
}

/// A refuted instance is an outcome the record states, not a failure: the run
/// still produces a bundle, and the synthetic contradiction it exports has
/// variables, so it still gets a vtree.
#[test]
fn a_refuted_run_records_the_refutation_and_still_exports_a_bundle() {
    let produced = run_on(REFUTED);
    assert!(
        produced.preprocessed.record.unsat,
        "preprocessing refuted the instance and must say so",
    );
    assert!(
        produced.built().is_some(),
        "the exported contradiction has variables, so it has a vtree",
    );
    assert!(
        produced.preprocessed.telemetry.simplify_ms.is_some(),
        "the early refutation path must retain the attempted phase telemetry",
    );
    let dir = Scratch::new("run-refuted");
    let paths = produced
        .write_to_dir(dir.path(), ComponentWriteOptions::default())
        .expect("a refuted run still writes its bundle");
    assert!(paths.bundle.reduced_cnf.exists());
}

/// The record and the CNF a run writes are the ones it is holding — the write
/// half reports what the run half decided, rather than a second derivation of
/// it.
#[test]
fn the_bundle_a_run_writes_reads_back_as_the_record_it_holds() {
    let produced = run_on(IRREDUCIBLE_5);
    let dir = Scratch::new("run-readback");
    let paths = produced
        .write_to_dir(dir.path(), ComponentWriteOptions::default())
        .expect("the run must write");

    let json = std::fs::read_to_string(&paths.bundle.record).expect("preprocess.json is readable");
    let read: PreprocessRecord = serde_json::from_str(&json).expect("preprocess.json must parse");
    let held = &produced.preprocessed.record;
    assert_eq!(read.mode, held.mode);
    assert_eq!(read.unsat, held.unsat);
    assert_eq!(read.count_lift_pow2, held.count_lift_pow2);
    assert_eq!(read.weight_lift, held.weight_lift);
    assert_eq!(read.original_num_vars, held.original_num_vars);

    let (reparsed, _) = parse(
        &std::fs::read_to_string(&paths.bundle.reduced_cnf).expect("reduced.cnf is readable"),
    );
    assert_eq!(
        reparsed, produced.preprocessed.reduced,
        "reduced.cnf must be the formula the run reduced to",
    );
}

/// A directory that already holds a bundle is written INTO: the bundle's own
/// files are replaced, and whatever else the caller keeps there is left alone.
#[test]
fn writing_a_run_into_a_directory_that_already_holds_a_bundle_replaces_it() {
    let dir = Scratch::new("run-rewrite");
    let target = dir.out("bundle");

    let first = run_on(IRREDUCIBLE_5);
    first
        .write_to_dir(&target, ComponentWriteOptions::default())
        .expect("the first run must write");
    let unrelated = target.join("notes.txt");
    std::fs::write(&unrelated, "kept").expect("a file of the caller's own");

    let second = run_on(IRREDUCIBLE_WIDER);
    let paths = second
        .write_to_dir(&target, ComponentWriteOptions::default())
        .expect("a second run into the same directory must write");

    let (reparsed, _) = parse(
        &std::fs::read_to_string(&paths.bundle.reduced_cnf).expect("reduced.cnf is readable"),
    );
    assert_eq!(
        reparsed, second.preprocessed.reduced,
        "the second run's formula must be what the file now holds",
    );
    assert_eq!(
        std::fs::read_to_string(&unrelated).expect("the caller's file is readable"),
        "kept",
        "a pre-existing directory is written into, not cleared",
    );
}

/// The writer takes the build and the formula as separate arguments, so a
/// caller can pair one with a formula it was not made from. That is a mistake in
/// the call rather than a broken invariant, so it is a [`VitriError::Mismatch`]
/// naming both counts — and it is refused before the component's own files land.
#[test]
fn a_build_from_another_formula_is_refused_before_its_component_files_are_written() {
    let formula = make_formula(
        10,
        vec![
            vec![1, -2],
            vec![2, -3],
            vec![3, -4],
            vec![4, -5],
            vec![6, -7],
            vec![7, -8],
            vec![8, -9],
            vec![9, -10],
        ],
    );
    let component = |vtree: Vtree, clauses: Vec<usize>, first: u32| ComponentVtree {
        vtree: Arc::new(vtree),
        clause_indices: clauses,
        local_to_outer: (first..first + 5).map(VarId).collect(),
    };
    let build = VtreeBuild {
        vtree: Arc::new(Vtree::balanced(10)),
        components: Some(vec![
            // One leaf short of the component it claims to describe.
            component(Vtree::balanced(4), vec![0, 1, 2, 3], 0),
            component(Vtree::balanced(5), vec![4, 5, 6, 7], 5),
        ]),
        selections: vec![SelectionRecord::default(), SelectionRecord::default()],
        candidate_sets: Vec::new(),
        limits: Default::default(),
        construction_ms: 0,
    };

    let dir = Scratch::new("run-mismatch");
    let err = build
        .write_to_dir(dir.path(), &formula, None, ComponentWriteOptions::default())
        .expect_err("a vtree that does not span its component must be refused");
    assert!(
        matches!(err, VitriError::Mismatch { .. }),
        "the arguments disagree, so this is a Mismatch, got: {err:?}",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("component 0") && msg.contains("4 leaves") && msg.contains("5 variables"),
        "the message must name the component and both counts, got: {msg}",
    );
    assert!(
        !dir.path().join(COMPONENTS_JSON_NAME).exists(),
        "no manifest may describe a build that was refused",
    );
    let component_files = std::fs::read_dir(dir.path().join(COMPONENTS_DIR))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(
        component_files, 0,
        "the refused component must leave no files behind",
    );
}

/// The whole-formula vtree is the other half of the same pairing: it and the
/// formula are separate arguments, so a build made over a different variable
/// count can be handed in. It spans the formula exactly or it is refused —
/// neither a leaf short nor a leaf over — and the refusal comes before the vtree
/// file or the manifest, so the caller's directory is left as it was.
#[test]
fn a_whole_formula_vtree_over_another_variable_count_is_refused_before_anything_is_written() {
    let formula = make_formula(5, vec![vec![1, -2], vec![2, -3], vec![3, -4], vec![4, -5]]);
    let whole = |leaves: u32| VtreeBuild {
        vtree: Arc::new(Vtree::balanced(leaves)),
        components: None,
        selections: Vec::new(),
        candidate_sets: Vec::new(),
        limits: Default::default(),
        construction_ms: 0,
    };

    for (leaves, tag) in [(4u32, "run-whole-short"), (6u32, "run-whole-over")] {
        let dir = Scratch::new(tag);
        let err = whole(leaves)
            .write_to_dir(dir.path(), &formula, None, ComponentWriteOptions::default())
            .expect_err("a vtree over another variable count must be refused");
        assert!(
            matches!(err, VitriError::Mismatch { .. }),
            "the arguments disagree, so this is a Mismatch, got: {err:?}",
        );
        let msg = err.to_string();
        assert!(
            msg.contains(&format!("{leaves} leaves")) && msg.contains("5 variables"),
            "the message must name both counts, got: {msg}",
        );
        for absent in [VTREE_NAME, COMPONENTS_JSON_NAME] {
            assert!(
                !dir.path().join(absent).exists(),
                "{absent} must not be written for a build that was refused",
            );
        }
    }
}

/// A component names its clauses by index into the formula it was cut from, so
/// a build made for another formula can name an index that formula does not
/// have. Refused, naming the index and how many clauses there are, before the
/// vtree file or the manifest is written.
#[test]
fn a_component_claiming_a_clause_outside_the_formula_is_refused_before_anything_is_written() {
    let formula = make_formula(4, vec![vec![1, 2], vec![3, 4]]);
    let build = VtreeBuild {
        vtree: Arc::new(Vtree::balanced(4)),
        components: Some(vec![ComponentVtree {
            vtree: Arc::new(Vtree::balanced(2)),
            clause_indices: vec![0, 5],
            local_to_outer: vec![VarId(0), VarId(1)],
        }]),
        selections: Vec::new(),
        candidate_sets: Vec::new(),
        limits: Default::default(),
        construction_ms: 0,
    };

    let dir = Scratch::new("run-clause-out-of-range");
    let err = build
        .write_to_dir(dir.path(), &formula, None, ComponentWriteOptions::default())
        .expect_err("a clause the formula does not have must be refused");
    assert!(
        matches!(err, VitriError::Mismatch { .. }),
        "the arguments disagree, so this is a Mismatch, got: {err:?}",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("clause 5") && msg.contains("2 clauses"),
        "the message must name the index and how many there are, got: {msg}",
    );
    for absent in [VTREE_NAME, COMPONENTS_JSON_NAME] {
        assert!(
            !dir.path().join(absent).exists(),
            "{absent} must not be written for a build that was refused",
        );
    }
}

/// A split is a partition, so no variable belongs to two components. Two
/// components claiming the same clause claim its variables twice over, and the
/// manifest would name one reduced id in two component maps. Refused, naming
/// both components and the variable they share, before anything is written.
#[test]
fn two_components_claiming_the_same_clause_are_refused_before_anything_is_written() {
    let formula = make_formula(4, vec![vec![1, 2], vec![3, 4]]);
    let component = |leaves: u32, clauses: Vec<usize>, outer: Vec<u32>| ComponentVtree {
        vtree: Arc::new(Vtree::balanced(leaves)),
        clause_indices: clauses,
        local_to_outer: outer.into_iter().map(VarId).collect(),
    };
    let build = VtreeBuild {
        vtree: Arc::new(Vtree::balanced(4)),
        components: Some(vec![
            component(2, vec![0], vec![0, 1]),
            component(4, vec![0, 1], vec![0, 1, 2, 3]),
        ]),
        selections: Vec::new(),
        candidate_sets: Vec::new(),
        limits: Default::default(),
        construction_ms: 0,
    };

    let dir = Scratch::new("run-shared-clause");
    let err = build
        .write_to_dir(dir.path(), &formula, None, ComponentWriteOptions::default())
        .expect_err("one variable cannot belong to two components");
    assert!(
        matches!(err, VitriError::Mismatch { .. }),
        "the arguments disagree, so this is a Mismatch, got: {err:?}",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("components 0 and 1") && msg.contains("variable 1"),
        "the message must name both components and the variable they share, got: {msg}",
    );
    for absent in [VTREE_NAME, COMPONENTS_JSON_NAME] {
        assert!(
            !dir.path().join(absent).exists(),
            "{absent} must not be written for a build that was refused",
        );
    }
}
