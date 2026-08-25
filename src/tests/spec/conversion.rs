//! Conversion parameters: the three keys that name a reading of a
//! decomposition, and the `force` axes typed off the same parse.
//!
//! Each key fixes exactly one dimension and leaves the rest for the search.

use super::*;

/// Every value the three conversion keys have, each written on its own, with
/// the whole resulting reading compared against one with a single dimension
/// named. A value that named a second dimension — or the wrong one — would
/// otherwise cut the search down to a different tree without saying so.
#[test]
fn every_conversion_value_names_its_own_dimension_and_only_that() {
    let cases: [(&str, Reading); 13] = [
        (
            "root=first",
            Reading {
                root: Some(Root::First),
                ..Reading::default()
            },
        ),
        (
            "root=centroid",
            Reading {
                root: Some(Root::Centroid),
                ..Reading::default()
            },
        ),
        (
            "place=deep",
            Reading {
                place: Some(Place::Deep),
                ..Reading::default()
            },
        ),
        (
            "place=shallow",
            Reading {
                place: Some(Place::Shallow),
                ..Reading::default()
            },
        ),
        (
            "fold=balanced",
            Reading {
                fold: Some(Fold::Balanced),
                ..Reading::default()
            },
        ),
        (
            "fold=by-size",
            Reading {
                fold: Some(Fold::BySize),
                ..Reading::default()
            },
        ),
        (
            "fold=vars-first",
            Reading {
                fold: Some(Fold::VarsFirst),
                ..Reading::default()
            },
        ),
        (
            "fold=left-deep",
            Reading {
                fold: Some(Fold::LeftDeep),
                ..Reading::default()
            },
        ),
        (
            "fold=clause-split",
            Reading {
                fold: Some(Fold::ClauseSplit),
                ..Reading::default()
            },
        ),
        (
            "fold=hypergraph",
            Reading {
                fold: Some(Fold::Hypergraph),
                ..Reading::default()
            },
        ),
        (
            "fold=boundary",
            Reading {
                fold: Some(Fold::Boundary),
                ..Reading::default()
            },
        ),
        (
            "fold=td-edge",
            Reading {
                fold: Some(Fold::TdEdge),
                ..Reading::default()
            },
        ),
        (
            "fold=affinity",
            Reading {
                fold: Some(Fold::Affinity),
                ..Reading::default()
            },
        ),
    ];
    for (param, expected) in cases {
        let spec = format!("flowcutter-primal:{param}");
        let p = parse_ok(&spec);
        assert_eq!(
            p.reading, expected,
            "{param} must name its own dimension and leave every other open",
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
fn a_spec_with_no_conversion_parameter_leaves_every_dimension_to_the_search() {
    let p = parse_ok("flowcutter-primal");
    assert_eq!(p.base, "flowcutter-primal");
    assert_eq!(p.reading, Reading::default());
}

/// All three keys at once is a search of exactly one reading, and the parse
/// carries all three off the one pass — a spec that dropped one would search a
/// dimension the caller had already decided.
#[test]
fn the_three_keys_are_read_together() {
    let p = parse_ok("flowcutter-primal:root=centroid,place=shallow,fold=affinity");
    assert_eq!(
        p.reading,
        Reading {
            root: Some(Root::Centroid),
            place: Some(Place::Shallow),
            fold: Some(Fold::Affinity),
        },
    );
}

/// A run-wide reading is a DEFAULT the spec refines, not a second setting
/// competing with it: what the spec names wins, what it leaves open is taken
/// from the run.
#[test]
fn a_spec_refines_the_run_wide_reading_rather_than_replacing_it() {
    let run = Reading {
        root: Some(Root::Centroid),
        place: Some(Place::Shallow),
        fold: Some(Fold::LeftDeep),
    };
    let mut p = parse_ok("flowcutter-primal:fold=td-edge");
    p.inherit(run);
    assert_eq!(
        p.reading,
        Reading {
            root: Some(Root::Centroid),
            place: Some(Place::Shallow),
            fold: Some(Fold::TdEdge),
        },
    );
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
