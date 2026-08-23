//! What a component bundle writes, and how it numbers what it writes.
//!
//! Each component is renumbered into its own local space, so the manifest
//! has to state the local-to-reduced map — and everything keyed by a
//! variable id, the show set included, has to be remapped with it.
//! Anything written per vtree, such as a picture, has to appear for every
//! vtree rather than only the first.

use super::*;

use crate::tests::common::chain_components;

/// Two independent chains plus one variable that occurs in no clause: the
/// middle run of length one contributes no clause, only its variable.
/// Component A owns reduced vars 1..=5, component B owns 7..=11, and reduced
/// var 6 is free — so LOCAL and REDUCED ids DISAGREE for component B, which
/// is exactly the case a consumer gets wrong.
fn two_chains_with_a_free_var() -> CnfFormula {
    chain_components(&[5, 1, 5])
}

/// A single-component formula still gets a manifest — one identity entry
/// pointing at the top-level files, so the consumer has one code path.
///
/// The show set travels that path too. It is the branch that has nothing to
/// remap, which is exactly why it is worth pinning: "the local set is the
/// reduced set" is true here and false one branch over, so a change that made
/// the two branches share a derivation has to keep this one an identity rather
/// than quietly reindexing it.
#[test]
fn single_component_manifest_is_the_identity() {
    let formula = CnfFormula {
        num_vars: 3,
        clauses: vec![Clause::new(vec![
            Literal::new(VarId(0), true),
            Literal::new(VarId(1), false),
            Literal::new(VarId(2), true),
        ])],
    };
    let dir = Scratch::new("single");
    let built = VtreeBuild {
        vtree: Arc::new(Vtree::balanced(formula.num_vars)),
        components: None,
        selections: vec![crate::spec::SelectionRecord::default()],
        candidate_sets: Vec::new(),
    };
    let (m, paths) = write_components(
        dir.path(),
        &formula,
        &built,
        Some(&ShowSet::from_dimacs_ids(&[1, 3]).expect("valid ids")),
        ComponentWriteOptions::default(),
    )
    .expect("manifest must write");

    assert_eq!(m.components.len(), 1);
    let c = &m.components[0];
    assert_eq!(c.local_to_reduced_dimacs, vec![1, 2, 3]);
    assert_eq!(
        c.show_vars_local_dimacs.as_ref().map(|s| s.to_dimacs()),
        Some(vec![1, 3]),
        "one component means local numbering IS reduced numbering",
    );
    // ...and the entry points at the top-level file, so a consumer following the
    // manifest reads the same CNF the show set is written over.
    assert_eq!(c.cnf, crate::bundle::REDUCED_CNF_NAME);
    assert_eq!(c.vtree, crate::bundle::VTREE_NAME);
    assert!(
        paths.files.is_empty(),
        "no duplicate per-component files are written"
    );
    assert!(m.free_vars_reduced_dimacs.is_empty());
}

/// The dot option is per-VTREE, not per-component: every `.vtree` this
/// writer emits gets its picture beside it, reported in the same path list,
/// and the default emits none at all.
#[test]
fn a_dot_sits_beside_every_written_vtree() {
    let formula = two_chains_with_a_free_var();
    let cfg = RunConfig {
        vtree_spec: "minfill".to_string(),
        ..Default::default()
    };
    let built = build_vtree(&formula, &cfg, &SelectionCtx::plain()).expect("the vtree must build");

    let dir = Scratch::new("dot");
    let (_, paths) = write_components(
        dir.path(),
        &formula,
        &built,
        None,
        ComponentWriteOptions { dot: true },
    )
    .expect("components must write");

    let vtrees: Vec<PathBuf> = paths
        .files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "vtree"))
        .cloned()
        .collect();
    assert_eq!(vtrees.len(), 2, "one vtree per component");
    for v in &vtrees {
        let dot = v.with_extension("dot");
        assert!(dot.exists(), "{} has no .dot beside it", v.display());
        assert!(
            paths.files.contains(&dot),
            "a written file must be reported"
        );
        let text = std::fs::read_to_string(&dot).unwrap();
        assert!(text.starts_with("graph vtree {"), "{text}");
        // Annotated against the component's own CNF, not left as bare structure.
        assert!(text.contains("c="), "{text}");
    }

    let plain_dir = Scratch::new("no-dot");
    let (_, plain) = write_components(
        plain_dir.path(),
        &formula,
        &built,
        None,
        ComponentWriteOptions::default(),
    )
    .expect("components must write");
    assert!(
        !plain
            .files
            .iter()
            .any(|p| p.extension().is_some_and(|e| e == "dot")),
        "the default writes no picture",
    );
}

/// A projected instance's show set is remapped into each component's LOCAL
/// space — the same trap as `local_to_reduced_dimacs`, one level down.
#[test]
fn show_set_is_remapped_per_component() {
    let formula = two_chains_with_a_free_var();
    let cfg = RunConfig {
        vtree_spec: "minfill".to_string(),
        components: ComponentPolicy::Split,
        ..Default::default()
    };
    let built = build_vtree(&formula, &cfg, &SelectionCtx::plain()).expect("the vtree must build");
    assert!(built.components.is_some(), "two chains must split");

    // REDUCED show set {1, 8, 11}: local 1 of A, and locals 2 and 5 of B.
    let dir = Scratch::new("show");
    let (m, _) = write_components(
        dir.path(),
        &formula,
        &built,
        Some(&ShowSet::from_dimacs_ids(&[1, 8, 11]).expect("valid ids")),
        ComponentWriteOptions::default(),
    )
    .expect("components must write");
    assert_eq!(
        m.components[0]
            .show_vars_local_dimacs
            .as_ref()
            .map(|s| s.to_dimacs()),
        Some(vec![1]),
    );
    assert_eq!(
        m.components[1]
            .show_vars_local_dimacs
            .as_ref()
            .map(|s| s.to_dimacs()),
        Some(vec![2, 5]),
    );

    // ...and it is carried into the component's own DIMACS file.
    let cnf = std::fs::read_to_string(dir.path().join(&m.components[1].cnf)).unwrap();
    assert!(
        cnf.contains("c p show 2 5 0"),
        "component CNF must carry its local show line:\n{cnf}"
    );
}
