//! The decision this module makes about a spec before anything is built:
//! whether it reads the formula's graph and so may be split per component.
//! Settled inside `component`, so it is pinned here rather than through the
//! entry point that consults it.
//!
//! The other spec decision a build makes — whether `best=auto` ranks
//! candidates — belongs to the grammar that owns the parameter, and is pinned
//! with it in `crate::tests::spec::conversion`.

use crate::component::{build_vtree, is_structural_spec};
use crate::config::{ComponentPolicy, RunConfig};
use crate::decompose::SelectionCtx;
use crate::spec::parse_vtree_spec;
use crate::tests::common::chain_components;

/// Every advertised base name, from the one list the crate hands out: the
/// names it offers are the names a shell over it will offer, so each has to
/// survive its own grammar.
fn advertised_bases() -> Vec<String> {
    crate::spec::vtree_spec_bases()
}

/// Splitting reads the formula's graph, and the baseline shapes read no graph
/// at all — they lay out the declared variable space by index. So a baseline
/// spec under a policy that asks for a split builds one vtree over the whole
/// formula, and every other base splits.
#[test]
fn a_non_structural_spec_never_splits_a_formula_into_components() {
    for base in advertised_bases() {
        let parsed = parse_vtree_spec(&base).expect("an advertised base must parse");
        let is_baseline = crate::spec::baseline_spec_names().any(|b| b == base);
        assert_eq!(
            is_structural_spec(&parsed),
            !is_baseline,
            "{base} splits into components exactly when it reads the formula's graph",
        );
    }

    // Two independent 35-var chains over outer vars 0..=34 and 35..=69 (>30
    // vars each, so both route through the full spec dispatch rather than the
    // tiny minfill path).
    let formula = chain_components(&[35, 35]);
    let split_policy = |spec: &str| RunConfig {
        vtree_spec: spec.to_string(),
        components: ComponentPolicy::Split,
        ..Default::default()
    };
    for base in crate::spec::baseline_spec_names() {
        let built = build_vtree(&formula, &split_policy(base), &SelectionCtx::plain())
            .unwrap_or_else(|e| panic!("{base} must build: {e}"));
        assert!(
            built.components.is_none(),
            "{base} reads no graph, so it spans the whole formula",
        );
        assert_eq!(built.vtree.num_leaves(), formula.num_vars);
    }

    let structural = build_vtree(
        &formula,
        &split_policy("minfill-primal"),
        &SelectionCtx::plain(),
    )
    .expect("the structural spec must build");
    assert_eq!(
        structural.components.as_ref().map(Vec::len),
        Some(2),
        "the contrast: a spec that reads the graph does split",
    );
}
