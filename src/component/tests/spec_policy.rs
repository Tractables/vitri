//! The two decisions this module makes about a spec before anything is built:
//! which spec a formula of a given size is actually constructed from, and
//! whether that spec reads the formula's graph and so may be split per
//! component. Both are settled inside `component`, so they are pinned here
//! rather than through the entry point that consults them.

use crate::component::{build_vtree, is_structural_spec, spec_for_size};
use crate::config::{ComponentPolicy, RunConfig};
use crate::decompose::SelectionCtx;
use crate::spec::parse_vtree_spec;
use crate::tests::common::chain_components;

/// Every advertised base name, in one place: the names the crate hands out are
/// the names a shell over it will offer, so each has to survive its own
/// grammar.
fn advertised_bases() -> Vec<String> {
    crate::spec::elimination_spec_names()
        .chain(crate::spec::decomposition_spec_names())
        .chain(crate::spec::baseline_spec_names())
        .chain(crate::spec::standalone_spec_names())
        .map(str::to_string)
        .collect()
}

/// minfill has no `/best` handler — the auto-upgrade must leave it bare
/// (it used to produce "minfill/best" → "Unknown vtree type" on ≤1000-var
/// formulas).
#[test]
fn spec_for_size_leaves_minfill_bare() {
    assert_eq!(spec_for_size(500, "minfill"), "minfill");
}

/// The upgrade fires on the families whose grammar honors `/best` and on no
/// other, so a small formula gets the ranked construction without the spec ever
/// naming a token the parse would then refuse.
#[test]
fn a_bare_decomposition_spec_on_a_small_formula_is_upgraded_to_best() {
    for base in ["goatd", "goatd-primal", "flowcutter-primal:200ms"] {
        assert_eq!(
            spec_for_size(500, base),
            format!("{base}/best"),
            "{base} ranks candidates, so a small formula gets the ranked build",
        );
    }
    for base in ["portfolio", "balanced", "minfill", "hypergraph-bisect"] {
        assert_eq!(
            spec_for_size(500, base),
            base,
            "{base} has no candidate list for /best to rank",
        );
    }
    for typed in ["goatd/best", "flowcutter-primal/shallow", "force/d=3"] {
        assert_eq!(
            spec_for_size(500, typed),
            typed,
            "a spec that already carries a suffix is honored as typed",
        );
    }
}

/// The upgrade invents a spec the caller never typed, so the one it invents has
/// to be one the grammar accepts — otherwise a legal `--vtree` string is
/// refused for a reason the caller cannot see in what they wrote.
#[test]
fn the_best_upgrade_only_produces_specs_the_grammar_accepts() {
    for base in advertised_bases() {
        let upgraded = spec_for_size(500, &base);
        crate::spec::validate_vtree_spec(&upgraded).unwrap_or_else(|e| {
            panic!("the upgrade of {base} produced {upgraded}, which the grammar refuses: {e}")
        });
    }
}

/// The size gate is inclusive at its own threshold, and above it the spec is
/// whatever the caller typed — including the bare form the smaller formula
/// would have had upgraded.
#[test]
fn a_formula_past_the_upgrade_size_keeps_the_spec_the_caller_typed() {
    assert_eq!(spec_for_size(1000, "goatd"), "goatd/best");
    assert_eq!(spec_for_size(1001, "goatd"), "goatd");
    // A spec carrying a suffix is untouched on either side of the boundary.
    assert_eq!(spec_for_size(1000, "goatd/best"), "goatd/best");
    assert_eq!(spec_for_size(1001, "goatd/best"), "goatd/best");
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

    let structural = build_vtree(&formula, &split_policy("minfill"), &SelectionCtx::plain())
        .expect("the structural spec must build");
    assert_eq!(
        structural.components.as_ref().map(Vec::len),
        Some(2),
        "the contrast: a spec that reads the graph does split",
    );
}
