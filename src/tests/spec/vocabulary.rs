//! The set of spellings the validator accepts is written out in full, and
//! every base name is classified into exactly one family — so a spelling
//! added to the parser without being added here, or a family the
//! classifier does not cover, shows up as a failure rather than as an
//! undocumented feature.

use super::*;

/// THE accepted-spec pin: every spelling the crate's own catalog names must
/// parse, and every malformed form must be rejected NAMING its offending
/// token. One parse decides both, so this is the whole grammar in one place.
#[test]
fn the_accepted_spec_set() {
    for spec in [
        // Simple baselines.
        "balanced",
        "linear",
        "reverse-linear",
        "random",
        "random-anything",
        // Portfolio.
        "portfolio",
        // Single-configuration backends.
        "hypergraph-bisect",
        "hypergraph-bisect:0.4",
        // goatd.
        "goatd-primal",
        "goatd-primal:7",
        "goatd-primal/best",
        "goatd",
        "goatd:3",
        "goatd/best",
        // The single elimination orders: every name in the family, then the two
        // tokens each of them takes.
        "minfill",
        "minfill-sample-jw",
        "mindegree",
        "mindegree-sample-jw",
        "nested-dissection",
        "minfill:7",
        "minfill-inc",
        "minfill-sample-jw:7",
        "mindegree-inc:3",
        "nested-dissection-inc",
        // FlowCutter: both graphs, all three param shapes, the suffix families.
        "flowcutter-primal",
        "flowcutter-incidence",
        "flowcutter-primal/best",
        "flowcutter-incidence/best",
        "flowcutter-primal:200ms",
        "flowcutter-primal:200ms,50iters",
        "flowcutter-primal:200ms,50iters,20patience",
        "flowcutter-incidence:200ms/best",
        "flowcutter-primal:100000,900steps",
        "flowcutter-primal:100000,900steps/shallow",
        "flowcutter-incidence/td-edge/shallow",
        "flowcutter-primal/vars-first",
        "flowcutter-primal/centroid/affinity",
        "flowcutter-incidence/first-bag/deep",
        "flowcutter-primal/clause-split",
        "flowcutter-incidence/boundary-adjacent",
        "flowcutter-primal/largest-first",
        "flowcutter-primal/children-by-size",
        "flowcutter-primal/left-deep",
        "flowcutter-incidence/hypergraph-bisect",
        "flowcutter-incidence:200ms/shallow/best",
        // The combiner over a FlowCutter incidence decomposition: bare, and both
        // effort shapes — including the step-budgeted one that names the
        // portfolio's own effort.
        "hybrid-flowcutter-incidence",
        "hybrid-flowcutter-incidence:200ms",
        "hybrid-flowcutter-incidence:200ms,50iters",
        "hybrid-flowcutter-incidence:150000,15steps",
        // The edge-aligned assembly, spelled out of the general suffixes.
        "flowcutter-incidence/td-edge/shallow/centroid",
        // Force-directed embedding: both tree-ifiers, every axis, and the
        // shared-axis subset that `:cut` accepts.
        "force",
        "force:mst",
        "force:cut",
        "force/d=3/fb=2",
        "force:mst/root=balance/orient=small/w=co/cw=short/d=4/fb=8/seeds=16/init=force1d",
        "force:cut/cw=short/d=3/seeds=4/init=force1d",
    ] {
        assert!(
            validate_vtree_spec(spec).is_ok(),
            "{spec} is in the catalog and must be accepted",
        );
    }

    for (spec, needle) in [
        // Unknown suffix, any family.
        ("flowcutter-primal/bogus", "/bogus"),
        ("nonsense/bogus", "/bogus"),
        // Families that take no token at all.
        ("portfolio:5", "portfolio"),
        ("portfolio/best", "portfolio"),
        ("balanced/shallow", "balanced"),
        ("random:7", "random"),
        // An elimination order names its whole configuration in the base, so no
        // suffix — the otherwise-tolerated `/best` included — and its one token
        // is the seed.
        ("minfill/best", "minfill"),
        ("mindegree-inc/shallow", "/shallow"),
        ("minfill:abc", "abc"),
        // goatd tolerates only `/best`, and its param is a u64 seed.
        ("goatd-primal/shallow", "/shallow"),
        ("goatd-primal:abc", "abc"),
        // bisect takes no suffix and a float param.
        ("hypergraph-bisect/best", "hypergraph-bisect"),
        ("hypergraph-bisect:abc", "abc"),
        // FlowCutter param shapes.
        ("flowcutter-primal:bogus", "bogus"),
        ("flowcutter-primal:abcms", "abc"),
        ("flowcutter-primal:200ms,xi", "xi"),
        ("flowcutter-primal:200ms,xp", "xp"),
        ("flowcutter-primal:200ms,9q", "9q"),
        ("flowcutter-primal:abc,900steps", "abc"),
        ("flowcutter-primal:100000,xsteps", "x"),
        // `/best` and an item ordering cannot both apply.
        ("flowcutter-primal/vars-first/best", "/vars-first"),
        // Step-budgeted mode reads the bag assignment and nothing else, so every
        // other suffix family — item ordering, root strategy, in-bag variable
        // order — and `/best` have nothing to set there.
        (
            "flowcutter-incidence:100000,900steps/clause-split",
            "/clause-split",
        ),
        ("flowcutter-primal:900steps/centroid", "/centroid"),
        ("flowcutter-primal:900steps/first-bag", "/first-bag"),
        ("flowcutter-primal:900steps/affinity", "/affinity"),
        ("flowcutter-primal:900steps/best", "/best"),
        (
            "flowcutter-incidence:100000,900steps/shallow/centroid",
            "/centroid",
        ),
        // The combiner names ONE fixed assembly rule, so no suffix — not an
        // unknown one, not a construction one, and not the otherwise-tolerated
        // `/best` — can change what it builds. Same param shapes as
        // `flowcutter-incidence`, so the same malformed budgets are refused.
        ("hybrid-flowcutter-incidence/bogus", "/bogus"),
        ("hybrid-flowcutter-incidence/best", "/best"),
        ("hybrid-flowcutter-incidence/shallow", "/shallow"),
        ("hybrid-flowcutter-incidence/td-edge", "/td-edge"),
        ("hybrid-flowcutter-incidence:bogus", "bogus"),
        ("hybrid-flowcutter-incidence:abcms", "abc"),
        ("hybrid-flowcutter-incidence:100000,xsteps", "x"),
        // Force: a bad tree-ifier, every out-of-range axis value, a duplicated key,
        // a suffix that is not key=value, and an unknown key.
        ("force:bogus", "bogus"),
        ("force/d=9", "/d=9"),
        ("force/fb=9", "/fb=9"),
        ("force/seeds=0", "/seeds=0"),
        ("force/seeds=17", "/seeds=17"),
        ("force/init=bogus", "/init=bogus"),
        ("force/cw=bogus", "/cw=bogus"),
        ("force/root=bogus", "/root=bogus"),
        ("force/orient=bogus", "/orient=bogus"),
        ("force/w=bogus", "/w=bogus"),
        ("force/d=3/d=4", "/d="),
        ("force/best", "/best"),
        ("force/nonsense", "/nonsense"),
        ("force/bogus=1", "/bogus="),
        // An axis outside the vocabulary is refused by name, whatever its value —
        // it is never accepted inertly.
        ("force/refine=goatd", "/refine="),
        ("force/refine=none", "/refine="),
        // The MST-reshaping axes cannot combine with the median-cut tree-ifier.
        ("force:cut/root=merge", "root"),
        ("force:cut/orient=x", "orient"),
        ("force:cut/w=co", "w"),
        ("force:cut/fb=2", "fb"),
    ] {
        let err = validate_vtree_spec(spec)
            .expect_err(&format!("{spec} must be rejected"))
            .to_string();
        assert!(
            err.contains(needle),
            "{spec} must be rejected naming {needle:?}, got: {err}",
        );
    }
}

/// `classify_base` is THE single base-name classifier every consumer routes
/// through — a misclassification here silently desyncs every one of them.
/// Covers each variant, the `random` prefix family, and the elimination-order
/// family the construction table defines.
#[test]
fn classify_base_covers_every_family() {
    use VtreeBase::*;
    assert_eq!(classify_base("balanced"), Balanced);
    assert_eq!(classify_base("linear"), Linear);
    assert_eq!(classify_base("reverse-linear"), ReverseLinear);
    assert_eq!(classify_base("portfolio"), Portfolio);
    assert_eq!(classify_base("flowcutter-primal"), FlowcutterPrimal);
    assert_eq!(classify_base("flowcutter-incidence"), FlowcutterIncidence);
    // The combiner base is its OWN family, not the plain incidence one it
    // shares a prefix (and a decomposition) with.
    assert_eq!(
        classify_base("hybrid-flowcutter-incidence"),
        HybridFlowcutterIncidence,
    );
    assert_eq!(classify_base("hypergraph-bisect"), HypergraphBisect);
    assert_eq!(classify_base("force"), Force);

    // Prefix family.
    assert_eq!(classify_base("random"), Random);
    assert_eq!(classify_base("random-anything"), Random);
    assert_eq!(classify_base("goatd-primal"), Goatd { incidence: false },);
    assert_eq!(classify_base("goatd"), Goatd { incidence: true });

    // Every name in the elimination-order table, and each one's `-inc` view.
    // The family carries the construction it resolved to, so the classifier is
    // also what pins the name and the graph view a builder is handed.
    for name in crate::decompose::elimination_spec_names() {
        assert_eq!(
            classify_base(name),
            Elimination {
                name,
                incidence: false
            },
            "{name} names a family",
        );
        assert_eq!(
            classify_base(&format!("{name}-inc")),
            Elimination {
                name,
                incidence: true
            },
            "{name}-inc names the same family on the incidence graph",
        );
    }

    // Everything else is Unknown — including the retired per-order goatd
    // spelling those names replaced.
    assert_eq!(classify_base("goatd-elimination-MinDegree"), Unknown);
    assert_eq!(classify_base("nonsense"), Unknown);
    assert_eq!(classify_base(""), Unknown);
}
