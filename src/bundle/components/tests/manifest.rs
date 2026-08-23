use crate::bundle::components::*;
use crate::cnf::CnfFormula;
use crate::component::build_vtree;
use crate::config::RunConfig;
use crate::decompose::SelectionCtx;
use crate::tests::common::{Scratch, chain_components};

#[test]
fn manifest_states_the_local_to_reduced_numbering() {
    // Two independent chains plus one variable that occurs in no clause: the
    // middle run of length one contributes no clause, only its variable.
    // Component A owns reduced vars 1..=5, component B owns 7..=11, and reduced
    // var 6 is free — so LOCAL and REDUCED ids DISAGREE for component B, which
    // is exactly the case a consumer gets wrong.
    let formula = chain_components(&[5, 1, 5]);
    let cfg = RunConfig {
        vtree_spec: "minfill".to_string(),
        ..Default::default()
    };
    let built = build_vtree(&formula, &cfg, &SelectionCtx::plain()).expect("the vtree must build");
    assert!(built.components.is_some(), "two chains must split");
    let whole = built.vtree.clone();

    let dir = Scratch::new("numbering");
    let (m, paths) = write_components(
        dir.path(),
        &formula,
        &built,
        None,
        ComponentWriteOptions::default(),
    )
    .expect("components must write");

    assert_eq!(m.components.len(), 2);
    // Reduced var 6 (1-based) occurs in no clause.
    assert_eq!(m.free_vars_reduced_dimacs, vec![6]);
    assert!(
        manifest_matches_vtree(&m, &whole),
        "components + free vars must cover the reduced space"
    );

    // Components sort by (clause count, min var): A first.
    let a = &m.components[0];
    let b = &m.components[1];
    assert_eq!(a.local_to_reduced_dimacs, vec![1, 2, 3, 4, 5]);
    // The case a consumer gets wrong: component B's LOCAL 1..=5 are REDUCED
    // 7..=11.
    assert_eq!(b.local_to_reduced_dimacs, vec![7, 8, 9, 10, 11]);

    // The emitted component CNF is really in the LOCAL space, and re-parses
    // to a formula whose vtree file has one leaf per variable.
    let cnf = std::fs::read_to_string(dir.path().join(&b.cnf)).unwrap();
    let (parsed, _) =
        CnfFormula::from_dimacs(std::io::Cursor::new(&cnf)).expect("component CNF parses");
    assert_eq!(parsed.num_vars as usize, b.local_to_reduced_dimacs.len());
    let max_lit = parsed
        .clauses
        .iter()
        .flat_map(|c| c.literals.iter())
        .map(|l| l.var.0 + 1)
        .max()
        .unwrap();
    assert!(
        max_lit as usize <= b.local_to_reduced_dimacs.len(),
        "component CNF must be renumbered into a dense local space",
    );
    assert_eq!(paths.files.len(), 4, "two files per component");
}
