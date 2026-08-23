//! The configuration axes, one at a time and then all at once.
//!
//! Per axis the demand is the same: still valid, still deterministic, and
//! actually DIFFERENT from the default — an axis that silently does
//! nothing is the failure this catches. Two axes additionally promise not
//! to make the load worse, which is checked rather than assumed.

use super::*;

#[test]
fn axes_valid_and_deterministic() {
    let f = axis_formula();
    let variants = [
        cfg_with(|c| c.root = RootRule::Balance),
        cfg_with(|c| c.root = RootRule::Hybrid),
        cfg_with(|c| c.orient = OrientRule::Small),
        cfg_with(|c| c.orient = OrientRule::Big),
        cfg_with(|c| c.weight = WeightRule::Co),
        cfg_with(|c| c.clause_weight = ClauseWeight::Short),
        cfg_with(|c| {
            c.root = RootRule::Balance;
            c.weight = WeightRule::Co;
            c.clause_weight = ClauseWeight::Short;
        }),
    ];
    for cfg in variants {
        let a = vtree_from_force(&f, cfg).unwrap();
        let b = vtree_from_force(&f, cfg).unwrap();
        assert_covers_all_vars(&a, 100, &format!("{cfg:?}"));
        assert_eq!(
            a.to_vtree_text(),
            b.to_vtree_text(),
            "must be deterministic ({cfg:?})"
        );
    }
}

#[test]
fn root_variants_differ_from_merge() {
    let f = axis_formula();
    let merge = vtree_from_force(&f, ForceConfig::new(ForceMode::Mst))
        .unwrap()
        .to_vtree_text();
    for root in [RootRule::Balance, RootRule::Hybrid] {
        let v = vtree_from_force(&f, cfg_with(|c| c.root = root))
            .unwrap()
            .to_vtree_text();
        assert_ne!(v, merge, "root={root:?} must differ from merge");
    }
}

#[test]
fn co_weight_changes_the_mst() {
    let f = axis_formula();
    let euclid = vtree_from_force(&f, ForceConfig::new(ForceMode::Mst)).unwrap();
    let co = vtree_from_force(&f, cfg_with(|c| c.weight = WeightRule::Co)).unwrap();
    assert_covers_all_vars(&euclid, 100, "euclid");
    assert_covers_all_vars(&co, 100, "co");
    assert_ne!(
        euclid.to_vtree_text(),
        co.to_vtree_text(),
        "co-occurrence weighting must change the MST"
    );
}

#[test]
fn dim_axis_valid_deterministic_and_differs() {
    let f = axis_formula();
    for mode in [ForceMode::Mst, ForceMode::Cut] {
        let base = vtree_from_force(&f, ForceConfig::new(mode))
            .unwrap()
            .to_vtree_text();
        for d in [2usize, 3, 4] {
            let cfg = ForceConfig {
                dim: d,
                ..ForceConfig::new(mode)
            };
            let a = vtree_from_force(&f, cfg).unwrap();
            let b = vtree_from_force(&f, cfg).unwrap();
            assert_covers_all_vars(&a, 100, &format!("mode {mode:?}, d={d}"));
            assert_eq!(
                a.to_vtree_text(),
                b.to_vtree_text(),
                "deterministic (mode {mode:?}, d={d})"
            );
            if d == 2 {
                assert_eq!(a.to_vtree_text(), base, "d=2 is the default");
            } else {
                assert_ne!(
                    a.to_vtree_text(),
                    base,
                    "d={d} must differ from d=2 (mode {mode:?})"
                );
            }
        }
    }
}

#[test]
fn fb_axis_valid_deterministic() {
    let f = axis_formula();
    let base = vtree_from_force(&f, ForceConfig::new(ForceMode::Mst))
        .unwrap()
        .to_vtree_text();
    let cfg0 = ForceConfig {
        fb: 0,
        ..ForceConfig::new(ForceMode::Mst)
    };
    assert_eq!(
        vtree_from_force(&f, cfg0).unwrap().to_vtree_text(),
        base,
        "fb=0 is the default build"
    );
    for d in [2usize, 3] {
        for fb in [1u8, 2, 4] {
            let cfg = ForceConfig {
                dim: d,
                fb,
                ..ForceConfig::new(ForceMode::Mst)
            };
            let a = vtree_from_force(&f, cfg).unwrap();
            let b = vtree_from_force(&f, cfg).unwrap();
            assert_covers_all_vars(&a, 100, &format!("d={d}, fb={fb}"));
            assert_eq!(
                a.to_vtree_text(),
                b.to_vtree_text(),
                "deterministic (d={d}, fb={fb})"
            );
        }
    }
}

/// Round 0 is always a candidate, so the keep-best rule guarantees `fb≥1` never
/// leaves a worse max clause-LCA load than `fb=0`.
#[test]
fn fb_never_worsens_max_load() {
    let f = axis_formula();
    let base = vtree_from_force(&f, ForceConfig::new(ForceMode::Mst)).unwrap();
    let base_load = max_load(&base, &f);
    for fb in [1u8, 2, 3] {
        let cfg = ForceConfig {
            fb,
            ..ForceConfig::new(ForceMode::Mst)
        };
        let vt = vtree_from_force(&f, cfg).unwrap();
        let load = max_load(&vt, &f);
        assert!(
            load <= base_load,
            "fb={fb} max-load {load} must be at most fb=0's {base_load}"
        );
    }
}

/// `seeds>1` stays reproducible and, because restart 0 reuses the base seed, its
/// kept vtree's max clause-LCA load is at most the `seeds=1` vtree's. The objective
/// is computed from the finished vtree, so this holds for both tree-ifiers.
#[test]
fn seeds_axis_deterministic_and_never_worsens_load() {
    let f = axis_formula();
    for mode in [ForceMode::Mst, ForceMode::Cut] {
        let base = vtree_from_force(&f, ForceConfig::new(mode)).unwrap();
        let base_load = max_load(&base, &f);
        for seeds in [2u8, 4, 8] {
            let cfg = ForceConfig {
                seeds,
                ..ForceConfig::new(mode)
            };
            let a = vtree_from_force(&f, cfg).unwrap();
            let b = vtree_from_force(&f, cfg).unwrap();
            assert_covers_all_vars(&a, 100, &format!("mode {mode:?}, seeds={seeds}"));
            assert_eq!(
                a.to_vtree_text(),
                b.to_vtree_text(),
                "deterministic (mode {mode:?}, seeds={seeds})"
            );
            let load = max_load(&a, &f);
            assert!(
                load <= base_load,
                "mode {mode:?} seeds={seeds} max-load {load} must be at most seeds=1's {base_load}"
            );
        }
    }
    let base = vtree_from_force(&f, ForceConfig::new(ForceMode::Mst))
        .unwrap()
        .to_vtree_text();
    let one = ForceConfig {
        seeds: 1,
        ..ForceConfig::new(ForceMode::Mst)
    };
    assert_eq!(
        vtree_from_force(&f, one).unwrap().to_vtree_text(),
        base,
        "seeds=1 is the default build"
    );
}

#[test]
fn init_force1d_valid_deterministic_and_differs() {
    let f = axis_formula();
    for mode in [ForceMode::Mst, ForceMode::Cut] {
        for d in [2usize, 3, 4] {
            let cfg = ForceConfig {
                dim: d,
                init: InitMode::Force1d,
                ..ForceConfig::new(mode)
            };
            let a = vtree_from_force(&f, cfg).unwrap();
            let b = vtree_from_force(&f, cfg).unwrap();
            assert_covers_all_vars(&a, 100, &format!("mode {mode:?}, d={d}"));
            assert_eq!(
                a.to_vtree_text(),
                b.to_vtree_text(),
                "init=force1d deterministic (mode {mode:?}, d={d})"
            );
            let rand = vtree_from_force(
                &f,
                ForceConfig {
                    dim: d,
                    ..ForceConfig::new(mode)
                },
            )
            .unwrap()
            .to_vtree_text();
            assert_ne!(
                a.to_vtree_text(),
                rand,
                "init=force1d must differ from init=rand (mode {mode:?}, d={d})"
            );
        }
    }
}

#[test]
fn every_axis_at_once_stays_valid_and_deterministic() {
    let f = axis_formula();
    let cfg = ForceConfig {
        dim: 3,
        fb: 2,
        seeds: 4,
        init: InitMode::Force1d,
        root: RootRule::Hybrid,
        orient: OrientRule::Small,
        weight: WeightRule::Co,
        clause_weight: ClauseWeight::Short,
        mode: ForceMode::Mst,
    };
    let a = vtree_from_force(&f, cfg).unwrap();
    let b = vtree_from_force(&f, cfg).unwrap();
    assert_covers_all_vars(&a, 100, "full stack");
    assert_eq!(
        a.to_vtree_text(),
        b.to_vtree_text(),
        "the full stack must be deterministic"
    );
}
