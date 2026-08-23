//! Suffixes: the `/`-separated words that set individual fields.
//!
//! Each suffix sets exactly one field and leaves the rest at their
//! defaults; when two suffixes address the same field the leftmost wins.
//! The FORCE axes are typed off this same parse rather than a second one,
//! so they are checked here too.

use super::*;

/// Every construction suffix the grammar has, each written on its own, with the
/// whole resulting config compared against the default with exactly one field
/// moved. A suffix that set a second field — or the wrong one — would otherwise
/// build a different vtree without saying so.
#[test]
fn every_construction_suffix_sets_its_own_field_and_only_that() {
    let with = |f: fn(&mut TdToVtreeConfig)| {
        let mut cfg = TdToVtreeConfig::default();
        f(&mut cfg);
        cfg
    };
    let cases: [(&str, TdToVtreeConfig); 13] = [
        (
            "shallow",
            with(|c| c.bag_assignment = BagAssignment::Shallowest),
        ),
        ("deep", with(|c| c.bag_assignment = BagAssignment::Deepest)),
        (
            "centroid",
            with(|c| c.root_strategy = TdRootStrategy::Centroid),
        ),
        (
            "first-bag",
            with(|c| c.root_strategy = TdRootStrategy::FirstBag),
        ),
        (
            "affinity",
            with(|c| c.var_order = VarOrderInBag::ClauseAffinity),
        ),
        (
            "vars-first",
            with(|c| c.item_ordering = ItemOrdering::VariablesFirst),
        ),
        (
            "children-by-size",
            with(|c| c.item_ordering = ItemOrdering::ChildrenBySize),
        ),
        (
            "clause-split",
            with(|c| c.item_ordering = ItemOrdering::ClauseSplit),
        ),
        (
            "left-deep",
            with(|c| c.item_ordering = ItemOrdering::LeftDeep),
        ),
        (
            "largest-first",
            with(|c| c.item_ordering = ItemOrdering::LargestFirst),
        ),
        (
            "hypergraph-bisect",
            with(|c| c.item_ordering = ItemOrdering::HypergraphBisect),
        ),
        (
            "boundary-adjacent",
            with(|c| c.item_ordering = ItemOrdering::BoundaryAdjacent),
        ),
        (
            "td-edge",
            with(|c| c.item_ordering = ItemOrdering::TdEdgeAligned),
        ),
    ];
    for (name, expected) in cases {
        let spec = format!("flowcutter-primal/{name}");
        let p = parse_ok(&spec);
        assert_eq!(
            p.td_config, expected,
            "/{name} must set its own field and leave every other at its default",
        );
        assert!(!p.use_best, "/{name} is not the ranking suffix");
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
    /// The axis name, the value written after it, and the reading that says the
    /// value landed on its own axis rather than anywhere else.
    type Case = (&'static str, &'static str, fn(&ForceConfig) -> bool);

    let cases: &[Case] = &[
        ("root", "merge", |c| c.root == RootRule::Merge),
        ("root", "balance", |c| c.root == RootRule::Balance),
        ("root", "hybrid", |c| c.root == RootRule::Hybrid),
        ("orient", "x", |c| c.orient == OrientRule::X),
        ("orient", "small", |c| c.orient == OrientRule::Small),
        ("orient", "big", |c| c.orient == OrientRule::Big),
        ("w", "euclid", |c| c.weight == WeightRule::Euclid),
        ("w", "co", |c| c.weight == WeightRule::Co),
        ("cw", "uniform", |c| {
            c.clause_weight == ClauseWeight::Uniform
        }),
        ("cw", "short", |c| c.clause_weight == ClauseWeight::Short),
        ("d", "2", |c| c.dim == 2),
        ("d", "3", |c| c.dim == 3),
        ("d", "4", |c| c.dim == 4),
        ("init", "rand", |c| c.init == InitMode::Rand),
        ("init", "force1d", |c| c.init == InitMode::Force1d),
    ];
    for &(axis, value, holds) in cases {
        let spec = format!("force/{axis}={value}");
        assert!(
            holds(&force_cfg(&spec)),
            "{spec} must land on the {axis} axis",
        );
    }
    // The two numeric axes, over the whole range each row admits.
    for fb in 0..=8u8 {
        let spec = format!("force/fb={fb}");
        assert_eq!(force_cfg(&spec).fb, fb, "{spec} sets the feedback rounds");
    }
    for seeds in 1..=16u8 {
        let spec = format!("force/seeds={seeds}");
        assert_eq!(
            force_cfg(&spec).seeds,
            seeds,
            "{spec} sets the restart count",
        );
    }
}

#[test]
fn parse_no_suffix_returns_default_config() {
    let p = parse_ok("flowcutter-primal");
    assert_eq!(p.base, "flowcutter-primal");
    assert_eq!(p.td_config, TdToVtreeConfig::default());
    assert!(!p.use_best);
}

#[test]
fn parse_with_best_and_shallow() {
    let p = parse_ok("flowcutter-primal/shallow/best");
    assert_eq!(p.base, "flowcutter-primal");
    assert_eq!(p.td_config.bag_assignment, BagAssignment::Shallowest);
    assert!(p.use_best);
}

#[test]
fn parse_item_ordering_suffixes() {
    for (suffix, expected) in [
        ("/vars-first", ItemOrdering::VariablesFirst),
        ("/children-by-size", ItemOrdering::ChildrenBySize),
        ("/clause-split", ItemOrdering::ClauseSplit),
        ("/left-deep", ItemOrdering::LeftDeep),
        ("/largest-first", ItemOrdering::LargestFirst),
        ("/hypergraph-bisect", ItemOrdering::HypergraphBisect),
        ("/boundary-adjacent", ItemOrdering::BoundaryAdjacent),
        ("/td-edge", ItemOrdering::TdEdgeAligned),
    ] {
        let spec = format!("flowcutter-primal{suffix}");
        let p = parse_ok(&spec);
        assert_eq!(p.base, "flowcutter-primal", "suffix {suffix} should strip");
        assert_eq!(
            p.td_config.item_ordering, expected,
            "suffix {suffix} should map to {expected:?}"
        );
    }
}

#[test]
fn parse_root_and_var_order_suffixes() {
    let p = parse_ok("flowcutter-primal/centroid/affinity");
    assert_eq!(p.td_config.root_strategy, TdRootStrategy::Centroid);
    assert_eq!(p.td_config.var_order, VarOrderInBag::ClauseAffinity);
}

/// Pinned because the suffixes are applied right to left precisely to get
/// this order, and the opposite would silently change which vtree a
/// multi-ordering spec builds.
#[test]
fn parse_leftmost_suffix_wins_for_one_field() {
    let p = parse_ok("flowcutter-primal/children-by-size/left-deep");
    assert_eq!(p.td_config.item_ordering, ItemOrdering::ChildrenBySize);
    let p = parse_ok("flowcutter-primal/shallow/deep");
    assert_eq!(p.td_config.bag_assignment, BagAssignment::Shallowest);
}

/// The `force` axes come back TYPED off the one parse, defaults included, so the
/// constructor never re-reads the spec string. Pinned per axis: a suffix that
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
        force_cfg("force:mst"),
        d,
        "':mst' is the default tree-ifier"
    );
    assert_eq!(force_cfg("force:cut").mode, ForceMode::Cut);

    // Every axis, set at once.
    let all =
        force_cfg("force:mst/root=hybrid/orient=big/w=co/cw=short/d=4/fb=8/seeds=16/init=force1d");
    assert_eq!(all.root, RootRule::Hybrid);
    assert_eq!(all.orient, OrientRule::Big);
    assert_eq!(all.weight, WeightRule::Co);
    assert_eq!(all.clause_weight, ClauseWeight::Short);
    assert_eq!((all.dim, all.fb, all.seeds), (4, 8, 16));
    assert_eq!(all.init, InitMode::Force1d);

    // The shared axes reach the median-cut tree-ifier too.
    let cut = force_cfg("force:cut/d=3/seeds=2/cw=short/init=force1d");
    assert_eq!(cut.mode, ForceMode::Cut);
    assert_eq!((cut.dim, cut.seeds), (3, 2));
    assert_eq!(cut.clause_weight, ClauseWeight::Short);
    assert_eq!(cut.init, InitMode::Force1d);
}
