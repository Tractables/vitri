//! Conversion parameters: the keys that say how a vtree is read off a
//! decomposition, and the `force` axes typed off the same parse.
//!
//! Each key sets exactly one field and leaves the rest at their defaults.

use super::*;

/// Every conversion value the grammar has, each written on its own, with the
/// whole resulting config compared against the default with exactly one field
/// moved. A value that set a second field — or the wrong one — would otherwise
/// build a different vtree without saying so.
#[test]
fn every_conversion_value_sets_its_own_field_and_only_that() {
    let with = |f: fn(&mut TdToVtreeConfig)| {
        let mut cfg = TdToVtreeConfig::default();
        f(&mut cfg);
        cfg
    };
    let cases: [(&str, TdToVtreeConfig); 15] = [
        (
            "assign=shallow",
            with(|c| c.bag_assignment = BagAssignment::Shallowest),
        ),
        (
            "assign=deep",
            with(|c| c.bag_assignment = BagAssignment::Deepest),
        ),
        (
            "td-root=centroid",
            with(|c| c.root_strategy = TdRootStrategy::Centroid),
        ),
        (
            "td-root=first-bag",
            with(|c| c.root_strategy = TdRootStrategy::FirstBag),
        ),
        (
            "var-order=affinity",
            with(|c| c.var_order = VarOrderInBag::ClauseAffinity),
        ),
        (
            "var-order=natural",
            with(|c| c.var_order = VarOrderInBag::Natural),
        ),
        (
            "order=children-first",
            with(|c| c.item_ordering = ItemOrdering::ChildrenFirst),
        ),
        (
            "order=vars-first",
            with(|c| c.item_ordering = ItemOrdering::VariablesFirst),
        ),
        (
            "order=children-by-size",
            with(|c| c.item_ordering = ItemOrdering::ChildrenBySize),
        ),
        (
            "order=clause-split",
            with(|c| c.item_ordering = ItemOrdering::ClauseSplit),
        ),
        (
            "order=left-deep",
            with(|c| c.item_ordering = ItemOrdering::LeftDeep),
        ),
        (
            "order=largest-first",
            with(|c| c.item_ordering = ItemOrdering::LargestFirst),
        ),
        (
            "order=hypergraph-bisect",
            with(|c| c.item_ordering = ItemOrdering::HypergraphBisect),
        ),
        (
            "order=boundary-adjacent",
            with(|c| c.item_ordering = ItemOrdering::BoundaryAdjacent),
        ),
        (
            "order=td-edge",
            with(|c| c.item_ordering = ItemOrdering::TdEdgeAligned),
        ),
    ];
    for (param, expected) in cases {
        let spec = format!("flowcutter-primal:{param}");
        let p = parse_ok(&spec);
        assert_eq!(
            p.td_config, expected,
            "{param} must set its own field and leave every other at its default",
        );
    }
}

/// Every value on every `force` axis, not one per axis: a value that parsed but
/// landed on the wrong setting embeds the wrong layout in a vtree the caller
/// asked for by name.
#[test]
fn every_force_axis_value_in_the_grammar_is_accepted_and_typed() {
    use crate::decompose::{ClauseWeight, ForceConfig, InitMode, OrientRule, RootRule, WeightRule};

    let force_cfg = |spec: &str| match parse_ok(spec).param {
        SpecParam::Force(cfg) => cfg,
        _ => panic!("{spec} is a force spec"),
    };
    /// The axis key, the value written after it, and the reading that says the
    /// value landed on its own axis rather than anywhere else.
    type Case = (&'static str, &'static str, fn(&ForceConfig) -> bool);

    let cases: &[Case] = &[
        ("root", "merge", |c| c.root == RootRule::Merge),
        ("root", "balance", |c| c.root == RootRule::Balance),
        ("root", "hybrid", |c| c.root == RootRule::Hybrid),
        ("orient", "x", |c| c.orient == OrientRule::X),
        ("orient", "small", |c| c.orient == OrientRule::Small),
        ("orient", "big", |c| c.orient == OrientRule::Big),
        ("weights", "euclid", |c| c.weight == WeightRule::Euclid),
        ("weights", "co", |c| c.weight == WeightRule::Co),
        ("clause-weight", "uniform", |c| {
            c.clause_weight == ClauseWeight::Uniform
        }),
        ("clause-weight", "short", |c| {
            c.clause_weight == ClauseWeight::Short
        }),
        ("dim", "2", |c| c.dim == 2),
        ("dim", "3", |c| c.dim == 3),
        ("dim", "4", |c| c.dim == 4),
        ("init", "rand", |c| c.init == InitMode::Rand),
        ("init", "force1d", |c| c.init == InitMode::Force1d),
    ];
    for &(axis, value, holds) in cases {
        let spec = format!("force:{axis}={value}");
        assert!(
            holds(&force_cfg(&spec)),
            "{spec} must land on the {axis} axis",
        );
    }
    // The two numeric axes, over the whole range each row admits.
    for fb in 0..=8u8 {
        let spec = format!("force:feedback={fb}");
        assert_eq!(force_cfg(&spec).fb, fb, "{spec} sets the feedback rounds");
    }
    for restarts in 1..=16u8 {
        let spec = format!("force:restarts={restarts}");
        assert_eq!(
            force_cfg(&spec).seeds,
            restarts,
            "{spec} sets the restart count",
        );
    }
}

#[test]
fn a_spec_with_no_conversion_parameter_returns_the_default_config() {
    let p = parse_ok("flowcutter-primal");
    assert_eq!(p.base, "flowcutter-primal");
    assert_eq!(p.td_config, TdToVtreeConfig::default());
}

#[test]
fn best_and_a_conversion_parameter_are_typed_off_the_one_parse() {
    let p = parse_ok("flowcutter-primal:assign=shallow,best=off");
    assert_eq!(p.base, "flowcutter-primal");
    assert_eq!(p.td_config.bag_assignment, BagAssignment::Shallowest);
    assert!(!p.use_best);
}

#[test]
fn td_root_and_var_order_are_read_together() {
    let p = parse_ok("flowcutter-primal:td-root=centroid,var-order=affinity");
    assert_eq!(p.td_config.root_strategy, TdRootStrategy::Centroid);
    assert_eq!(p.td_config.var_order, VarOrderInBag::ClauseAffinity);
}

/// `best=auto` is the default, and it is the SIZE rule: a formula under the
/// threshold ranks the candidates its family builds, a larger one converts the
/// one decomposition. Pinned because the rule used to be applied by rewriting
/// the spec string, where it was invisible to anyone reading the spec.
#[test]
fn best_defaults_to_the_size_rule_and_an_explicit_value_overrides_it() {
    let resolved = |spec: &str, num_vars: u32| {
        let mut p = parse_ok(spec);
        p.resolve_best(num_vars);
        p.use_best
    };
    for base in [
        "flowcutter-primal",
        "flowcutter-incidence",
        "goatd-incidence",
    ] {
        assert!(
            resolved(base, BEST_AUTO_MAX_VARS),
            "{base} ranks candidates at the size the rule covers",
        );
        assert!(
            !resolved(base, BEST_AUTO_MAX_VARS + 1),
            "{base} converts one decomposition past that size",
        );
        // Written out, the size no longer decides.
        let on = format!("{base}:best=on");
        let off = format!("{base}:best=off");
        assert!(resolved(&on, BEST_AUTO_MAX_VARS + 1), "{on} is on");
        assert!(!resolved(&off, BEST_AUTO_MAX_VARS), "{off} is off");
    }
    // A budget the family reads does not state a conversion, so the rule still
    // applies; a conversion parameter does state one, and turns the rule off.
    assert!(resolved(
        "flowcutter-primal:budget=200ms",
        BEST_AUTO_MAX_VARS
    ));
    assert!(!resolved(
        "flowcutter-primal:assign=shallow",
        BEST_AUTO_MAX_VARS
    ));
    // Step-budgeted mode has no candidate list to rank at any size.
    assert!(!resolved(
        "flowcutter-primal:budget=900steps",
        BEST_AUTO_MAX_VARS
    ));
    // A family that builds one configuration never ranks anything.
    for base in [
        "minfill-primal",
        "hypergraph-bisect",
        "balanced",
        "portfolio",
    ] {
        assert!(
            !resolved(base, BEST_AUTO_MAX_VARS),
            "{base} builds one vtree, so there is nothing to rank",
        );
    }
}

/// The `force` axes come back TYPED off the one parse, defaults included, so the
/// constructor never re-reads the spec string. Pinned per axis: a parameter that
/// parsed but set the wrong field would otherwise build a different vtree in
/// silence.
#[test]
fn parse_force_axes_are_typed_off_the_one_parse() {
    use crate::decompose::{ClauseWeight, ForceMode, InitMode, OrientRule, RootRule, WeightRule};

    let force_cfg = |spec: &str| match parse_ok(spec).param {
        SpecParam::Force(cfg) => cfg,
        _ => panic!("{spec} is a force spec"),
    };

    let d = force_cfg("force");
    assert_eq!(d.mode, ForceMode::Mst);
    assert_eq!(d.root, RootRule::Merge);
    assert_eq!(d.orient, OrientRule::X);
    assert_eq!(d.weight, WeightRule::Euclid);
    assert_eq!(d.clause_weight, ClauseWeight::Uniform);
    assert_eq!((d.dim, d.fb, d.seeds), (2, 0, 1));
    assert_eq!(d.init, InitMode::Rand);
    assert_eq!(
        force_cfg("force:treeify=mst"),
        d,
        "'treeify=mst' is the default tree-ifier"
    );
    assert_eq!(force_cfg("force:treeify=cut").mode, ForceMode::Cut);

    // Every axis, set at once.
    let all = force_cfg(
        "force:treeify=mst,root=hybrid,orient=big,weights=co,clause-weight=short,dim=4,\
         feedback=8,restarts=16,init=force1d",
    );
    assert_eq!(all.root, RootRule::Hybrid);
    assert_eq!(all.orient, OrientRule::Big);
    assert_eq!(all.weight, WeightRule::Co);
    assert_eq!(all.clause_weight, ClauseWeight::Short);
    assert_eq!((all.dim, all.fb, all.seeds), (4, 8, 16));
    assert_eq!(all.init, InitMode::Force1d);

    // The shared axes reach the median-cut tree-ifier too.
    let cut = force_cfg("force:treeify=cut,dim=3,restarts=2,clause-weight=short,init=force1d");
    assert_eq!(cut.mode, ForceMode::Cut);
    assert_eq!((cut.dim, cut.seeds), (3, 2));
    assert_eq!(cut.clause_weight, ClauseWeight::Short);
    assert_eq!(cut.init, InitMode::Force1d);
}
