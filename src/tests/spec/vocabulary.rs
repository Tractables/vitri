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
        "hypergraph-bisect:imbalance=0.4",
        // goatd.
        "goatd-primal",
        "goatd-primal:seed=7",
        "goatd-primal:best=on",
        "goatd",
        "goatd:seed=3",
        "goatd:best=on",
        "goatd:seed=3,best=off",
        // The single elimination orders: every name in the family, then the
        // parameter each of them takes.
        "minfill",
        "minfill-sample-jw",
        "mindegree",
        "mindegree-sample-jw",
        "nested-dissection",
        "minfill:seed=7",
        "minfill-inc",
        "minfill-sample-jw:seed=7",
        "mindegree-inc:seed=3",
        "nested-dissection-inc",
        // FlowCutter: both graphs, both budget shapes, every conversion key.
        "flowcutter-primal",
        "flowcutter-incidence",
        "flowcutter-primal:best=on",
        "flowcutter-incidence:best=off",
        "flowcutter-primal:budget=200ms",
        "flowcutter-primal:budget=200ms,iters=50",
        "flowcutter-primal:budget=200ms,iters=50,patience=20",
        "flowcutter-incidence:budget=200ms,best=on",
        "flowcutter-primal:budget=100000steps,iters=900",
        "flowcutter-primal:budget=100000steps,iters=900,assign=shallow",
        "flowcutter-incidence:order=td-edge,assign=shallow",
        "flowcutter-primal:order=vars-first",
        "flowcutter-primal:td-root=centroid,var-order=affinity",
        "flowcutter-incidence:td-root=first-bag,assign=deep",
        "flowcutter-primal:order=clause-split",
        "flowcutter-incidence:order=boundary-adjacent",
        "flowcutter-primal:order=largest-first",
        "flowcutter-primal:order=children-by-size",
        "flowcutter-primal:order=left-deep",
        "flowcutter-incidence:order=hypergraph-bisect",
        "flowcutter-primal:order=children-first",
        "flowcutter-incidence:budget=200ms,assign=shallow,best=off",
        // The combiner over a FlowCutter incidence decomposition: bare, and both
        // effort shapes — including the step-budgeted one that names the
        // portfolio's own effort.
        "hybrid-flowcutter-incidence",
        "hybrid-flowcutter-incidence:budget=200ms",
        "hybrid-flowcutter-incidence:budget=200ms,iters=50",
        "hybrid-flowcutter-incidence:budget=150000steps,iters=15",
        // The edge-aligned assembly, spelled out of the general conversion keys.
        "flowcutter-incidence:order=td-edge,assign=shallow,td-root=centroid",
        // Force-directed embedding: both tree-ifiers, every axis, and the
        // shared-axis subset that `treeify=cut` accepts.
        "force",
        "force:treeify=mst",
        "force:treeify=cut",
        "force:dim=3,feedback=2",
        "force:treeify=mst,root=balance,orient=small,weights=co,clause-weight=short,dim=4,\
         feedback=8,restarts=16,init=force1d",
        "force:treeify=cut,clause-weight=short,dim=3,restarts=4,init=force1d",
    ] {
        assert!(
            validate_vtree_spec(spec).is_ok(),
            "{spec} is in the catalog and must be accepted",
        );
    }

    for (spec, needle) in [
        // A parameter that is not key=value at all.
        ("flowcutter-primal:bogus", "bogus"),
        ("force:mst", "mst"),
        ("goatd:7", "7"),
        // Families that take no parameter at all.
        ("portfolio:seed=5", "portfolio"),
        ("portfolio:best=on", "portfolio"),
        ("balanced:assign=shallow", "balanced"),
        ("random:seed=7", "random"),
        // An elimination order names its whole configuration in the base, so
        // the seed is its only parameter.
        ("minfill:best=on", "minfill"),
        ("mindegree-inc:assign=shallow", "assign"),
        ("minfill:seed=abc", "abc"),
        // goatd takes the seed and `best`, nothing else.
        ("goatd-primal:assign=shallow", "assign"),
        ("goatd-primal:seed=abc", "abc"),
        ("goatd:order=td-edge", "order"),
        // bisect takes the imbalance and nothing else.
        ("hypergraph-bisect:best=on", "hypergraph-bisect"),
        ("hypergraph-bisect:imbalance=abc", "abc"),
        // FlowCutter budget shapes.
        ("flowcutter-primal:budget=bogus", "bogus"),
        ("flowcutter-primal:budget=abcms", "abcms"),
        ("flowcutter-primal:budget=200", "200"),
        ("flowcutter-primal:budget=200ms,iters=xi", "xi"),
        ("flowcutter-primal:budget=200ms,patience=xp", "xp"),
        ("flowcutter-primal:budget=abcsteps", "abcsteps"),
        // `best=on` and a conversion key cannot both apply.
        ("flowcutter-primal:order=vars-first,best=on", "order"),
        ("flowcutter-primal:assign=deep,best=on", "assign"),
        // Step-budgeted mode reads the bag assignment and nothing else, so every
        // other conversion key — and `best=on` — has nothing to set there.
        (
            "flowcutter-incidence:budget=100000steps,order=clause-split",
            "order",
        ),
        (
            "flowcutter-primal:budget=900steps,td-root=centroid",
            "td-root",
        ),
        (
            "flowcutter-primal:budget=900steps,var-order=affinity",
            "var-order",
        ),
        ("flowcutter-primal:budget=900steps,best=on", "best=on"),
        ("flowcutter-primal:budget=900steps,patience=10", "patience"),
        // The combiner names ONE fixed assembly rule, so no conversion key can
        // change what it builds. Same budget shapes as `flowcutter-incidence`,
        // so the same malformed budgets are refused.
        ("hybrid-flowcutter-incidence:assign=shallow", "assign"),
        ("hybrid-flowcutter-incidence:best=on", "best"),
        ("hybrid-flowcutter-incidence:order=td-edge", "order"),
        ("hybrid-flowcutter-incidence:budget=bogus", "bogus"),
        ("hybrid-flowcutter-incidence:budget=abcms", "abcms"),
        // Force: a bad tree-ifier, every out-of-range axis value, a duplicated
        // key, and an unknown key.
        ("force:treeify=bogus", "bogus"),
        ("force:dim=9", "9"),
        ("force:feedback=9", "9"),
        ("force:restarts=0", "0"),
        ("force:restarts=17", "17"),
        ("force:init=bogus", "bogus"),
        ("force:clause-weight=bogus", "bogus"),
        ("force:root=bogus", "bogus"),
        ("force:orient=bogus", "bogus"),
        ("force:weights=bogus", "bogus"),
        ("force:dim=3,dim=4", "dim"),
        ("force:best=on", "best"),
        ("force:nonsense=1", "nonsense"),
        // A key outside the vocabulary is refused by name, whatever its value —
        // it is never accepted inertly.
        ("force:refine=goatd", "refine"),
        ("force:refine=none", "refine"),
        // The MST-reshaping axes cannot combine with the median-cut tree-ifier.
        ("force:treeify=cut,root=merge", "root"),
        ("force:treeify=cut,orient=x", "orient"),
        ("force:treeify=cut,weights=co", "weights"),
        ("force:treeify=cut,feedback=2", "feedback"),
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

/// A parameter a family does not accept is refused NAMING both the spec and
/// the parameter, rather than parsed and then ignored — the spec would
/// otherwise build a vtree the writer did not ask for.
#[test]
fn a_parameter_the_family_cannot_honor_is_refused_by_name() {
    for (spec, key) in [
        // The imbalance belongs to the one family that partitions directly.
        ("minfill:imbalance=0.4", "imbalance"),
        ("flowcutter-primal:imbalance=0.4", "imbalance"),
        ("goatd:imbalance=0.4", "imbalance"),
        ("force:imbalance=0.4", "imbalance"),
        // The seed belongs to the families that draw one.
        ("hypergraph-bisect:seed=3", "seed"),
        ("flowcutter-primal:seed=3", "seed"),
        ("balanced:seed=3", "seed"),
        // A conversion key belongs to the family that converts a decomposition.
        ("hypergraph-bisect:assign=deep", "assign"),
        ("minfill:order=td-edge", "order"),
    ] {
        let err = validate_vtree_spec(spec)
            .expect_err(&format!("{spec} names a parameter its family cannot honor"))
            .to_string();
        assert!(
            err.contains(key) && err.contains(spec.split(':').next().unwrap()),
            "{spec} must be refused naming {key:?} and the spec, got: {err}",
        );
    }
}

/// One parameter syntax: the `/`-suffix spelling every conversion setting used
/// to take names no construction any more. `/` is not a delimiter, so such a
/// string is a base name nothing claims — the build reports it the way it
/// reports any other unknown base, rather than building the pre-rename tree.
#[test]
fn the_retired_suffix_spelling_names_no_construction() {
    for spec in [
        "flowcutter-primal/best",
        "flowcutter-primal/shallow",
        "flowcutter-incidence/td-edge/shallow",
        "goatd/best",
        "force/d=3",
    ] {
        assert_eq!(
            classify_base(spec),
            VtreeBase::Unknown,
            "{spec} writes a parameter the retired way and names no family",
        );
    }
    // With a base the grammar does know, the retired spelling lands in the
    // parameter text, where it is refused for not being `key=value`.
    for spec in ["force:mst/root=merge", "flowcutter-primal:200ms/best"] {
        assert!(
            validate_vtree_spec(spec).is_err(),
            "{spec} writes a parameter the retired way and must be refused",
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
    assert_eq!(
        classify_base("flowcutter-primal"),
        Flowcutter { incidence: false },
    );
    assert_eq!(
        classify_base("flowcutter-incidence"),
        Flowcutter { incidence: true },
    );
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

/// Every base the vocabulary offers parses, and every parameter the catalog
/// advertises for it is one that base actually accepts at the value the
/// catalog names. `--help` and `docs/vtrees.md` are both rendered from this
/// pair, so an entry that advertised a key the parser refuses would send a
/// reader to a command line that does not run.
#[test]
fn every_advertised_base_and_parameter_is_one_the_parser_accepts() {
    for base in vtree_spec_bases() {
        assert!(
            validate_vtree_spec(&base).is_ok(),
            "{base} is offered by the vocabulary and must parse on its own",
        );
        for doc in spec_param_docs(&base) {
            assert!(
                !doc.values.is_empty() && !doc.default.is_empty() && !doc.what.is_empty(),
                "{base}:{}= must say what it takes, what it defaults to and what it changes",
                doc.key,
            );
        }
    }
}

/// `docs/vtrees.md` names every base and every parameter the parser accepts,
/// gives each one-word default, and states the `best=auto` size rule at the
/// variable count the parser applies it at.
///
/// `--help` is RENDERED from those two tables and so cannot fall behind them.
/// The doc is prose and can, which is what this holds: between them a reader of
/// either can write any spec this crate builds.
#[test]
fn the_vtree_doc_names_every_base_and_parameter() {
    // Compiled in rather than read from the working tree: `docs/*.md` ships
    // inside the package, so a released copy is held to its own parser too.
    let doc = include_str!("../../../docs/vtrees.md");
    for base in vtree_spec_bases() {
        assert!(
            doc.contains(&format!("`{base}`")),
            "docs/vtrees.md does not name the base {base}",
        );
        for p in spec_param_docs(&base) {
            assert!(
                doc.contains(&format!("`{}`", p.key)),
                "docs/vtrees.md does not name {}=, which {base} takes",
                p.key,
            );
            // A default that is one word is quoted; the three that are a phrase
            // ("100000 timed, 900 step-budgeted") are prose the doc words its
            // own way.
            if !p.default.contains(' ') {
                assert!(
                    doc.contains(&format!("`{}`", p.default)),
                    "docs/vtrees.md does not give the default of {}=",
                    p.key,
                );
            }
        }
    }
    assert!(
        doc.contains("`best=auto`") && doc.contains(&BEST_AUTO_MAX_VARS.to_string()),
        "docs/vtrees.md must state the best=auto size rule at the count it applies at",
    );
}
