//! Validating is not enough: a spelling the validator accepts must reach a
//! builder arm and return a leaf-complete vtree, and one it rejects must
//! fail saying so. The gap between the two is where a dropped builder arm
//! hides, so every family is built here on a real formula.
//!
//! That formula is `chain_components(&[40])` throughout: one 40-variable
//! chain, past the tiny-formula shortcut, with a real edge set for the
//! structural constructions to work from. Its clause widths are uniform and
//! its variable occurrences nearly so, which is what opens the portfolio's
//! structure-gated candidates on it.

use super::*;

use crate::tests::common::{assert_covers_all_vars, chain_components};

/// The four lists this crate hands out are what a shell over it offers, so each
/// name has to survive its own grammar AND reach a builder arm. A name offered
/// but unbuildable is a `--vtree` value the tool advertises and then refuses.
#[test]
fn every_name_the_crate_advertises_validates_and_builds() {
    let formula = chain_components(&[40]);
    let advertised = elimination_spec_names()
        .chain(decomposition_spec_names())
        .chain(baseline_spec_names())
        .chain(standalone_spec_names());
    for spec in advertised {
        assert!(
            validate_vtree_spec(spec).is_ok(),
            "{spec} is advertised and must validate",
        );
        let vt = build_one_vtree_artifacts(BuildRequest {
            formula: &formula,
            spec: &parse_ok(spec),
            ctx: &SelectionCtx::plain(),
            limits: &BuildLimits::default(),
        })
        .unwrap_or_else(|e| panic!("{spec} is advertised and must build: {e}"))
        .vtree;
        assert_covers_all_vars(&vt, formula.num_vars, spec);
    }
}

#[test]
fn spec_dispatch_builds_the_force_specs() {
    let formula = chain_components(&[40]);
    for spec in ["force", "force:cut", "force:mst/d=3/fb=2", "force/seeds=2"] {
        let a = build_one_vtree_artifacts(BuildRequest {
            formula: &formula,
            spec: &parse_ok(spec),
            ctx: &SelectionCtx::plain(),
            limits: &BuildLimits::default(),
        })
        .unwrap_or_else(|e| panic!("{spec} must build: {e}"))
        .vtree;
        let b = build_one_vtree_artifacts(BuildRequest {
            formula: &formula,
            spec: &parse_ok(spec),
            ctx: &SelectionCtx::plain(),
            limits: &BuildLimits::default(),
        })
        .unwrap_or_else(|e| panic!("{spec} must build: {e}"))
        .vtree;
        assert_eq!(
            a.num_leaves(),
            formula.num_vars,
            "{spec} must build a leaf-complete vtree",
        );
        assert_eq!(
            a.to_vtree_text(),
            b.to_vtree_text(),
            "{spec} must build the same vtree every time",
        );
    }
}

/// Cross-run identity is deliberately NOT asserted: like every other
/// decomposition-based spec the bare form runs a wall-clock-bounded search,
/// and that is a property of the search rather than of these specs.
#[test]
fn spec_dispatch_builds_the_flowcutter_combiner_specs() {
    let formula = chain_components(&[40]);
    for spec in [
        "hybrid-flowcutter-incidence",
        "hybrid-flowcutter-incidence:20000,4steps",
        "flowcutter-incidence/td-edge/shallow/centroid",
    ] {
        let v = build_one_vtree_artifacts(BuildRequest {
            formula: &formula,
            spec: &parse_ok(spec),
            ctx: &SelectionCtx::plain(),
            limits: &BuildLimits::default(),
        })
        .unwrap_or_else(|e| panic!("{spec} must build: {e}"))
        .vtree;
        assert_covers_all_vars(&v, formula.num_vars, spec);
    }
}

/// An unrecognized base parses `Ok` — the unknown-spec error is the builder's to
/// report, with the list of valid types — but building it still fails by name.
#[test]
fn an_unknown_base_validates_and_fails_at_build() {
    assert!(validate_vtree_spec("nonsense").is_ok());
    let formula = CnfFormula {
        num_vars: 2,
        clauses: vec![Clause::new(vec![
            Literal::new(VarId(0), true),
            Literal::new(VarId(1), false),
        ])],
    };
    let err = build_one_vtree_artifacts(BuildRequest {
        formula: &formula,
        spec: &parse_ok("nonsense"),
        ctx: &SelectionCtx::plain(),
        limits: &BuildLimits::default(),
    })
    .map(|_| ())
    .expect_err("an unknown base cannot build")
    .to_string();
    assert!(
        err.contains("nonsense") && err.contains("unknown vtree type"),
        "the unknown-spec error must name the spec, got: {err}",
    );
}

/// One min-fill implementation, reached two ways: the `minfill` spec and the
/// entry `component` takes for a tiny component build the SAME vtree from the
/// same formula. A second min-fill grown beside the first — or the spec's
/// default seed drifting away from the internal one — shows up here as two
/// different trees.
#[test]
fn the_minfill_spec_is_the_internal_minfill() {
    let formula = chain_components(&[40]);
    let from_spec = build_one_vtree_artifacts(BuildRequest {
        formula: &formula,
        spec: &parse_ok("minfill"),
        ctx: &SelectionCtx::plain(),
        limits: &BuildLimits::default(),
    })
    .expect("the minfill spec must build")
    .vtree;
    let internal = crate::decompose::vtree_from_minfill(
        &formula,
        crate::decompose::INTERNAL_ELIMINATION_SEED,
        1.0,
    )
    .expect("the internal min-fill must build");
    assert_eq!(
        from_spec.to_vtree_text(),
        internal.vtree.to_vtree_text(),
        "the minfill spec and the internal min-fill entry must be one construction",
    );
}

/// A name in the table that no construction can run would otherwise validate
/// and then die "unknown vtree type" at build time.
#[test]
fn spec_dispatch_builds_every_elimination_spec() {
    let formula = chain_components(&[40]);
    for name in crate::decompose::elimination_spec_names() {
        for spec in [name.to_string(), format!("{name}-inc"), format!("{name}:7")] {
            let vt = build_one_vtree_artifacts(BuildRequest {
                formula: &formula,
                spec: &parse_ok(&spec),
                ctx: &SelectionCtx::plain(),
                limits: &BuildLimits::default(),
            })
            .unwrap_or_else(|e| panic!("{spec} must build: {e}"))
            .vtree;
            assert_eq!(
                vt.num_leaves(),
                formula.num_vars,
                "{spec} must build a leaf-complete vtree",
            );
        }
    }
}

/// The retired `goatd-elimination-<Config>` spelling is gone, not aliased: it
/// fails at build time naming the spec, and the message offers the vocabulary
/// that replaced it.
#[test]
fn the_retired_per_order_spelling_is_rejected() {
    let formula = chain_components(&[40]);
    for spec in [
        "goatd-elimination-MinFill",
        "goatd-elimination-MinDegree-inc:3",
    ] {
        let err = build_one_vtree_artifacts(BuildRequest {
            formula: &formula,
            spec: &parse_ok(spec),
            ctx: &SelectionCtx::plain(),
            limits: &BuildLimits::default(),
        })
        .map(|_| ())
        .expect_err("the retired spelling cannot build")
        .to_string();
        assert!(
            err.contains(spec) && err.contains("minfill") && err.contains("mindegree"),
            "{spec} must be refused with the replacement names, got: {err}",
        );
    }
}

/// Companion to the validator: every parameterless "simple" vtree the
/// validator accepts must actually BUILD. This is the guard that was missing
/// when `linear`/`reverse-linear`/`random` validated (NO_TOKEN) but hit no
/// arm in the builder and died "Unknown vtree type" on any
/// non-tiny formula — a validator⟺builder divergence (the no-silent-no-op rule). A dropped
/// builder arm re-surfaces here as the builder returning
/// [`crate::VitriError::Spec`] instead of a vtree.
#[test]
fn spec_dispatch_builds_all_simple_specs() {
    let formula = chain_components(&[40]);
    for spec in ["balanced", "linear", "reverse-linear", "random"] {
        assert!(
            validate_vtree_spec(spec).is_ok(),
            "{spec} must validate as a simple vtree",
        );
        let vt = build_one_vtree_artifacts(BuildRequest {
            formula: &formula,
            spec: &parse_ok(spec),
            ctx: &SelectionCtx::plain(),
            limits: &BuildLimits::default(),
        })
        .unwrap_or_else(|e| panic!("{spec} must build: {e}"))
        .vtree;
        assert_eq!(
            vt.num_leaves(),
            formula.num_vars,
            "{spec} must build a leaf-complete vtree",
        );
    }
}
