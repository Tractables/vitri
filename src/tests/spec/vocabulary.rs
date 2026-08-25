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
        "primal-bisect",
        "primal-bisect:imbalance=0.4",
        // goatd.
        "goatd-primal",
        "goatd-primal:seed=7",
        "goatd-incidence",
        "goatd-incidence:seed=3",
        "goatd-primal:refine=off",
        "goatd-incidence:refine=on,seed=3",
        "goatd-incidence:seed=3,fold=edge",
        // The single elimination orders: every name in the family, then the
        // parameter each of them takes.
        "minfill-primal",
        "minfill-primal:ties=jw-sample",
        "mindegree-primal",
        "mindegree-primal:ties=jw-sample",
        "nested-dissection-primal",
        "minfill-primal:seed=7",
        "minfill-incidence",
        "minfill-primal:ties=jw-sample,seed=7",
        "mindegree-incidence:seed=3",
        "nested-dissection-incidence",
        // The order is one decomposition, so the conversion keys read it the
        // way they read a FlowCutter one.
        "minfill-incidence:fold=hypergraph",
        "mindegree-primal:place=shallow,root=centroid,fold=edge",
        // FlowCutter: both graphs, both budget shapes, every conversion key.
        "flowcutter-primal",
        "flowcutter-incidence",
        "flowcutter-primal:budget=200ms",
        "flowcutter-primal:budget=200ms,iters=50",
        "flowcutter-primal:budget=200ms,iters=50,patience=20",
        "flowcutter-primal:budget=100000steps,iters=900",
        "flowcutter-primal:budget=100000steps,iters=900,place=shallow",
        "flowcutter-incidence:fold=edge,place=shallow",
        "flowcutter-incidence:root=first,place=deep",
        "flowcutter-primal:root=leaf",
        "flowcutter-primal:root=centroid,fold=hypergraph",
        "flowcutter-incidence:fold=hypergraph",
        "flowcutter-primal:fold=balanced",
        "flowcutter-incidence:budget=200ms,place=shallow",
        // The reading named down to its last key, which leaves the search the
        // one choice `root=leaf` keeps open.
        "flowcutter-incidence:fold=edge,place=shallow,root=centroid",
        "flowcutter-incidence:fold=edge,place=shallow,root=leaf",
        // The guided bisection over a FlowCutter incidence decomposition: bare,
        // and both effort shapes — including the step-budgeted one that names
        // the portfolio's own effort.
        "guided-bisect",
        "guided-bisect:budget=200ms",
        "guided-bisect:budget=200ms,iters=50",
        "guided-bisect:budget=150000steps,iters=15",
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
        // `nested-dissection` has only the deterministic core.
        (
            "nested-dissection-primal:ties=jw-sample",
            "nested-dissection",
        ),
        (
            "nested-dissection-incidence:ties=jw-sample",
            "nested-dissection",
        ),
        // Each family takes the keys its own construction reads.
        ("minfill-primal:refine=off", "refine"),
        ("goatd-incidence:ties=jw-sample", "ties"),
        ("hypergraph-bisect:budget=200ms", "budget"),
        // The guided bisection folds its own bisections, so it reads none of
        // the three keys that name a reading of a decomposition.
        ("guided-bisect:root=centroid", "root"),
        ("guided-bisect:place=shallow", "place"),
        ("guided-bisect:fold=edge", "fold"),
        // A parameter that is not key=value at all.
        ("flowcutter-primal:bogus", "bogus"),
        ("force:mst", "mst"),
        ("goatd-incidence:7", "7"),
        // Families that take no parameter at all.
        ("portfolio:seed=5", "portfolio"),
        ("portfolio:fold=edge", "portfolio"),
        ("balanced:place=shallow", "balanced"),
        ("random:seed=7", "random"),
        ("minfill-primal:seed=abc", "abc"),
        // goatd takes the seed and the refinement switch, plus the three
        // conversion keys — nothing else.
        ("goatd-primal:budget=200ms", "budget"),
        ("goatd-primal:seed=abc", "abc"),
        // bisect takes the imbalance and nothing else.
        ("hypergraph-bisect:seed=3", "seed"),
        ("hypergraph-bisect:imbalance=abc", "abc"),
        // FlowCutter budget shapes.
        ("flowcutter-primal:budget=bogus", "bogus"),
        ("flowcutter-primal:budget=abcms", "abcms"),
        ("flowcutter-primal:budget=200", "200"),
        ("flowcutter-primal:budget=200ms,iters=xi", "xi"),
        ("flowcutter-primal:budget=200ms,patience=xp", "xp"),
        ("flowcutter-primal:budget=abcsteps", "abcsteps"),
        ("flowcutter-primal:budget=900steps,patience=10", "patience"),
        // A value outside the key's own vocabulary, on each of the three.
        ("flowcutter-primal:root=deepest", "deepest"),
        ("flowcutter-primal:place=middle", "middle"),
        ("flowcutter-primal:fold=largest-first", "largest-first"),
        // The pre-rename spellings of the three keys and their values.
        ("flowcutter-primal:td-root=centroid", "td-root"),
        ("flowcutter-primal:assign=shallow", "assign"),
        ("flowcutter-primal:order=vars-first", "order"),
        ("flowcutter-primal:var-order=affinity", "var-order"),
        ("flowcutter-incidence:assembly=hybrid", "assembly"),
        ("flowcutter-primal:best=on", "best"),
        ("flowcutter-primal:root=first-bag", "first-bag"),
        ("flowcutter-primal:fold=children-first", "children-first"),
        ("flowcutter-primal:fold=td-edge", "td-edge"),
        (
            "flowcutter-primal:fold=children-by-size",
            "children-by-size",
        ),
        (
            "flowcutter-primal:fold=hypergraph-bisect",
            "hypergraph-bisect",
        ),
        (
            "flowcutter-primal:fold=boundary-adjacent",
            "boundary-adjacent",
        ),
        // The guided bisection reads FlowCutter's budget, so the same
        // malformed budgets are refused there.
        ("guided-bisect:budget=bogus", "bogus"),
        ("guided-bisect:budget=abcms", "abcms"),
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
        ("force:fold=edge", "fold"),
        ("force:nonsense=1", "nonsense"),
        // A key outside the vocabulary is refused by name, whatever its value —
        // it is never accepted inertly.
        ("force:refine=goatd-incidence", "refine"),
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

/// A fold the search no longer selects is refused NAMING the key and the
/// family, rather than mapped onto whichever surviving fold is closest.
///
/// These six spellings each named a way of folding a bag that the search never
/// returned as the cheapest reading. A spec still writing one is a spec written
/// against an older vocabulary, and the tree it would build now is not the tree
/// it asked for — so the writer hears about it.
#[test]
fn a_fold_the_search_no_longer_selects_is_refused_by_name() {
    for value in [
        "clause-split",
        "by-size",
        "vars-first",
        "left-deep",
        "boundary",
        "affinity",
    ] {
        let spec = format!("flowcutter-primal:fold={value}");
        let err = validate_vtree_spec(&spec)
            .expect_err(&format!("{spec} names a fold the search does not select"))
            .to_string();
        assert!(
            err.contains("fold") && err.contains("flowcutter-primal"),
            "{spec} must be refused naming the key and the family, got: {err}",
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
        ("minfill-primal:imbalance=0.4", "imbalance"),
        ("flowcutter-primal:imbalance=0.4", "imbalance"),
        ("goatd-incidence:imbalance=0.4", "imbalance"),
        ("force:imbalance=0.4", "imbalance"),
        // The seed belongs to the families that draw one.
        ("hypergraph-bisect:seed=3", "seed"),
        ("flowcutter-primal:seed=3", "seed"),
        ("balanced:seed=3", "seed"),
        // A conversion key belongs to the families that convert a decomposition.
        ("hypergraph-bisect:place=deep", "place"),
        ("primal-bisect:fold=edge", "fold"),
        ("primal-bisect:root=centroid", "root"),
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
        // The view-less and `-inc` spellings the graph-view suffixes replaced,
        // and the assembly rule that is now a parameter.
        "minfill",
        "minfill-inc",
        // A graph view belongs to the families that decompose one; `force`
        // embeds the variables and decomposes nothing.
        "force-primal",
        "portfolio-incidence",
        "minfill-sample-jw",
        "mindegree-sample-jw-inc",
        "nested-dissection",
        "goatd",
        "hybrid-flowcutter-incidence",
        "flowcutter-incidence/td-edge/shallow",
        "goatd-incidence/best",
        "force/d=3",
        // The guided bisection under the name the parameter spelling gave it.
        "hybrid",
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
    assert_eq!(classify_base("guided-bisect"), GuidedBisect);
    assert_eq!(classify_base("hypergraph-bisect"), HypergraphBisect);
    assert_eq!(classify_base("force"), Force);

    // Prefix family.
    assert_eq!(classify_base("random"), Random);
    assert_eq!(classify_base("random-anything"), Random);
    assert_eq!(classify_base("goatd-primal"), Goatd { incidence: false },);
    assert_eq!(classify_base("goatd-incidence"), Goatd { incidence: true });

    // Every name in the elimination-order table, in both graph views. The
    // family carries the construction it resolved to, so the classifier is
    // also what pins the name and the graph view a builder is handed.
    for name in crate::decompose::elimination_spec_names() {
        for (view, incidence) in crate::decompose::VIEW_SUFFIXES {
            assert_eq!(
                classify_base(&format!("{name}{view}")),
                Elimination { name, incidence },
                "{name}{view} names the order on that graph view",
            );
        }
        // The order alone names no construction: which graph it runs on is
        // part of what a spec has to say.
        assert_eq!(classify_base(name), Unknown, "{name} names no graph view");
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
/// and gives each one-word default.
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
            // A default that is one word is quoted; the ones that are a phrase
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
}
