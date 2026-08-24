use crate::cnf::CnfFormula;
use crate::cnf::{Local, Reduced, ShowMask, ShowSet};
use crate::component::*;
use crate::config::ComponentPolicy;
use crate::config::RunConfig;
use crate::decompose::BuildLimits;
use crate::decompose::SelectionCtx;
use crate::error::VitriError;
use crate::spec::{BuildRequest, parse_vtree_spec};
use crate::tests::common::{assert_covers_all_vars, chain_components};
use crate::vtree::VarId;

/// What the per-component construction loop reported, recorded: each component
/// saw its own remapped show set, and the repeated-gadget cache actually fired.
/// The one implementation of `BuildObserver` that hears anything — production
/// observes through `()`.
#[derive(Default)]
struct ComponentBuildTrace {
    /// In component order, the LOCAL show masks construction installed for each
    /// component too big for the minfill path. Empty for a plain-MC build,
    /// which installs none.
    show_masks: Vec<ShowMask>,
    /// How many components reused a cached component-local vtree instead of
    /// constructing a fresh one — a structurally identical repeated gadget.
    vtree_cache_hits: usize,
}

impl BuildObserver for ComponentBuildTrace {
    fn cached_vtree_reused(&mut self) {
        self.vtree_cache_hits += 1;
    }

    fn component_show_mask(&mut self, mask: &ShowMask) {
        self.show_masks.push(mask.clone());
    }
}

/// `build_vtree_split` with the loop's own account of what it did — the entry
/// the tests below drive when the assertion is about the decisions rather than
/// the vtree.
fn build_vtree_traced(
    req: BuildRequest<'_>,
    policy: ComponentPolicy,
) -> Result<(VtreeBuild, ComponentBuildTrace), VitriError> {
    let mut trace = ComponentBuildTrace::default();
    let build = build_vtree_split(req, policy, &mut trace)?;
    Ok((build, trace))
}

/// A formula with no variables has no vtree, and the constructors below say so
/// by panicking. The library entry above them is handed caller-supplied input,
/// so it reports instead — whatever construction was named.
#[test]
fn a_spec_the_grammar_cannot_build_over_no_variables_reports_instead_of_aborting() {
    let empty = CnfFormula {
        num_vars: 0,
        clauses: Vec::new(),
    };
    for spec in [
        "balanced",
        "linear",
        "reverse-linear",
        "random",
        "portfolio",
        "minfill-primal",
    ] {
        let cfg = RunConfig {
            vtree_spec: spec.to_string(),
            ..Default::default()
        };
        match build_vtree(&empty, &cfg, &SelectionCtx::plain()) {
            Ok(_) => panic!("{spec} has no vtree to build over an empty variable space"),
            Err(err) => {
                assert!(
                    matches!(err, crate::error::VitriError::Input { .. }),
                    "{spec} over no variables must report the input, got {err:?}",
                );
                assert!(
                    err.to_string().contains('0'),
                    "the message must name the variable count, got: {err}",
                );
            }
        }
    }
}

/// Two independent 35-var chains over outer vars 0..=34 and 35..=69 — the
/// shared fixture for the per-component tests below (>30 vars each, so both
/// route through the full spec dispatch rather than the tiny minfill path).
fn two_chains() -> CnfFormula {
    chain_components(&[35, 35])
}

/// Regression: on a multi-component projected instance, each component's
/// show-aware vtree selection must read the OUTER show mask remapped into
/// that component's LOCAL var space — not the outer mask indexed by local
/// var id (which aliases arbitrary wrong variables). Constructs two >30-var
/// components whose local↔outer maps disagree on show-ness (local index i is
/// show for component A but the corresponding outer var of component B is
/// not) and asserts each component installs its own correctly-remapped mask.
/// Fails on unfixed `build_vtree` (no per-component scope → nothing recorded).
#[test]
fn projected_show_mask_remapped_per_component() {
    let formula = two_chains();

    // OUTER show mask: only outer vars {0,1,2} are show.
    let outer_mask = ShowSet::<Reduced>::from_zero_based([0, 1, 2]).mask(70);

    let parsed = parse_vtree_spec("portfolio").expect("the spec must parse");
    let (build, trace) = build_vtree_traced(
        BuildRequest {
            formula: &formula,
            spec: &parsed,
            ctx: &SelectionCtx::projected(std::rc::Rc::new(outer_mask.clone())),
            limits: &BuildLimits::default(),
        },
        ComponentPolicy::Split,
    )
    .expect("the vtree must build");
    let comp = build.components;
    let recorded = trace.show_masks;

    assert!(comp.is_some(), "expected a multi-component split");
    assert_eq!(
        recorded.len(),
        2,
        "both >30-var components must install a per-component show scope"
    );

    // Components sort by (clause count, min var): A (min var 0) then B (35).
    // Component A: local i → outer i → show iff i in {0,1,2}.
    let expect_a = ShowSet::<Local>::from_zero_based([0, 1, 2]).mask(35);
    assert_eq!(recorded[0], expect_a, "component A show set mis-remapped");

    // Component B: local i → outer 35+i → NEVER show.
    let expect_b = ShowSet::<Local>::empty().mask(35);
    assert_eq!(
        recorded[1], expect_b,
        "component B must see an all-hidden local show set, not aliased outer {{0,1,2}}"
    );
}

/// Within one `build_vtree`, two structurally-identical components (same
/// local CNF, same — here absent — show mask) build the vtree ONCE; the
/// second reuses the cached component-local structure. Deterministic spec
/// (`minfill`) so reuse is transparent.
#[test]
fn identical_components_build_their_vtree_once() {
    let formula = two_chains();

    let parsed = parse_vtree_spec("minfill-primal").expect("the spec must parse");
    let (build, trace) = build_vtree_traced(
        BuildRequest {
            formula: &formula,
            spec: &parsed,
            ctx: &SelectionCtx::plain(),
            limits: &BuildLimits::default(),
        },
        ComponentPolicy::Split,
    )
    .expect("the vtree must build");
    let comp = build.components;
    let hits = trace.vtree_cache_hits;

    assert!(comp.is_some(), "expected a multi-component split");
    assert_eq!(
        comp.as_ref().unwrap().len(),
        2,
        "expected exactly two components"
    );
    assert_eq!(
        hits, 1,
        "the second identical component must reuse the cached vtree (build once for the pair)"
    );
}

/// The cache's pitfall: identical clause structure but DIFFERENT component-local show
/// masks must NOT share a cache entry — the show scope changes portfolio's
/// selection, so each component must build independently.
#[test]
fn different_show_masks_do_not_share_a_cache_entry() {
    let formula = two_chains();

    // Only component A (outer vars 0..=34) has a show var; component B none.
    let outer_mask = ShowSet::<Reduced>::from_zero_based([0]).mask(70);

    let parsed = parse_vtree_spec("portfolio").expect("the spec must parse");
    let (build, trace) = build_vtree_traced(
        BuildRequest {
            formula: &formula,
            spec: &parsed,
            ctx: &SelectionCtx::projected(std::rc::Rc::new(outer_mask.clone())),
            limits: &BuildLimits::default(),
        },
        ComponentPolicy::Split,
    )
    .expect("the vtree must build");
    let comp = build.components;
    let hits = trace.vtree_cache_hits;
    let masks = trace.show_masks;

    assert!(comp.is_some(), "expected a multi-component split");
    assert_eq!(
        hits, 0,
        "components with different local show masks must not share a cache entry"
    );
    assert_eq!(
        masks.len(),
        2,
        "both components must construct and install their own show scope"
    );
}

/// The per-component descriptors must state a numbering a consumer can
/// actually follow: `local_to_outer` is dense (one entry per LOCAL var, in
/// the component vtree's leaf space), strictly increasing (extraction sorts),
/// and the components partition the clause list with no overlap.
#[test]
fn component_descriptors_state_a_consistent_numbering() {
    let formula = two_chains();
    let cfg = RunConfig {
        vtree_spec: "minfill-primal".to_string(),
        components: ComponentPolicy::Split,
        ..Default::default()
    };
    let built = build_vtree(&formula, &cfg, &SelectionCtx::plain()).expect("the vtree must build");
    let full = built.vtree;
    let comps = built.components.expect("expected a multi-component split");

    let mut seen_clauses = vec![false; formula.clauses.len()];
    let mut seen_outer = vec![false; formula.num_vars as usize];
    for cv in &comps {
        assert_eq!(
            cv.vtree.num_leaves() as usize,
            cv.local_to_outer.len(),
            "one vtree leaf per LOCAL variable"
        );
        assert!(
            cv.local_to_outer.windows(2).all(|w| w[0].0 < w[1].0),
            "local_to_outer must be strictly increasing in the outer space"
        );
        for &ci in &cv.clause_indices {
            assert!(!seen_clauses[ci], "clause {ci} claimed by two components");
            seen_clauses[ci] = true;
        }
        for &outer in &cv.local_to_outer {
            assert!(
                !seen_outer[outer.idx()],
                "outer var {outer:?} claimed by two components"
            );
            seen_outer[outer.idx()] = true;
        }
    }
    assert!(
        seen_clauses.iter().all(|&b| b),
        "components must cover every clause"
    );
    assert_eq!(
        full.num_leaves(),
        formula.num_vars,
        "graft covers the outer space"
    );
}

/// A formula with nothing to split still reports one selection and one
/// candidate set: the two lists carry one entry per vtree BUILT, whether or not
/// the build was per-component, so a consumer reads `components` to learn the
/// shape and never the lengths.
#[test]
fn a_single_component_formula_reports_no_split_but_one_selection() {
    let formula = chain_components(&[40]);
    let cfg = RunConfig {
        vtree_spec: "minfill-primal".to_string(),
        components: ComponentPolicy::Split,
        ..Default::default()
    };
    let built = build_vtree(&formula, &cfg, &SelectionCtx::plain()).expect("the vtree must build");
    assert!(
        built.components.is_none(),
        "a formula that does not split reports no components"
    );
    assert_eq!(built.selections.len(), 1, "one vtree built, one selection");
    assert_eq!(
        built.candidate_sets.len(),
        1,
        "one vtree built, one candidate set"
    );
    assert_eq!(built.vtree.num_leaves(), formula.num_vars);
}

/// A variable no clause names belongs to no component, so the graft is the only
/// place it can get its leaf. Dropped there, the vtree would not span the
/// variable space the formula declares.
#[test]
fn a_variable_no_clause_names_still_gets_exactly_one_leaf() {
    let mut formula = two_chains();
    formula.num_vars += 1;
    let free = VarId(formula.num_vars - 1);

    let cfg = RunConfig {
        vtree_spec: "minfill-primal".to_string(),
        components: ComponentPolicy::Split,
        ..Default::default()
    };
    let built = build_vtree(&formula, &cfg, &SelectionCtx::plain()).expect("the vtree must build");
    let comps = built
        .components
        .as_ref()
        .expect("expected a multi-component split");
    assert!(
        comps.iter().all(|c| !c.local_to_outer.contains(&free)),
        "a variable no clause names joins no component",
    );

    assert_covers_all_vars(&built.vtree, formula.num_vars, "the graft");
}

/// The tiny-component construction takes no deadline, so a build that starts
/// with none left still produces a vtree — the complement of the larger
/// components, for which a spent deadline is a construction error.
#[test]
fn a_tiny_component_builds_even_with_no_budget_left() {
    let formula = chain_components(&[5, 6]);
    let cfg = RunConfig {
        deadline: Some(std::time::Instant::now() - std::time::Duration::from_secs(1)),
        vtree_spec: "portfolio".to_string(),
        components: ComponentPolicy::Split,
        ..Default::default()
    };
    let built = build_vtree(&formula, &cfg, &SelectionCtx::plain())
        .expect("a component small enough to skip the portfolio has no deadline to be past");
    assert_eq!(
        built.components.as_ref().map(Vec::len),
        Some(2),
        "two independent chains are two components",
    );
    for sel in &built.selections {
        assert_eq!(
            sel.winning_spec.as_deref(),
            Some("minfill-primal"),
            "a component under the threshold is built by min-fill, whatever was asked for",
        );
    }
}

/// The construction budget at the layer a caller actually uses: a build handed
/// a deadline that has ALREADY passed fails with a construction error, on the
/// single-component path and on the split alike. The complement of the tiny
/// component above, whose min-fill shortcut takes no deadline at all.
///
/// The single chain can be small, because the deadline is spent before
/// construction starts and nothing about the formula's shape is ever reached.
/// The pair has to be 32 variables each: at or under the tiny threshold a
/// component takes the min-fill shortcut and cannot fail this way. Only the
/// first of the two ever runs, since the deadline is one absolute budget shared
/// across the whole split.
#[test]
fn expired_vtree_deadline_fails_construction() {
    // An already-gone run deadline: the construction deadline the entry point
    // derives from it is gone too.
    let expired = RunConfig {
        deadline: Some(std::time::Instant::now() - std::time::Duration::from_secs(1)),
        ..RunConfig::default()
    };

    // Single component. Matched by hand rather than `.expect_err()`: `VtreeBuild`
    // (the `Ok` side) carries an `Arc<Vtree>` and does not derive `Debug`, which
    // `.expect_err()`'s bound would otherwise require adding just for this test.
    let formula = chain_components(&[9]);
    match build_vtree(&formula, &expired, &SelectionCtx::plain()) {
        Ok(_) => panic!("an already-spent deadline must fail construction, not build a vtree"),
        Err(err) => assert!(
            matches!(err, crate::error::VitriError::Construction { .. }),
            "expected a construction error, got {err:?}",
        ),
    }

    // Two independent components, both big enough to reach portfolio: the
    // deadline is absolute and shared, so the first component already starts
    // past it, and the whole build fails before the second component is ever
    // reached.
    let two = chain_components(&[32, 32]);
    let err2 = match build_vtree(&two, &expired, &SelectionCtx::plain()) {
        Ok(_) => panic!("an already-spent deadline must fail a multi-component build too"),
        Err(err) => err,
    };
    assert!(
        matches!(err2, crate::error::VitriError::Construction { .. }),
        "expected a construction error, got {err2:?}",
    );
}

/// The threshold is inclusive: a component of exactly that many variables takes
/// the shortcut, one variable more is built by the spec the caller typed.
#[test]
fn a_component_at_the_tiny_threshold_takes_the_shortcut_and_one_past_it_does_not() {
    for (size, built_by) in [(30u32, "minfill-primal"), (31, "hypergraph-bisect")] {
        let formula = chain_components(&[size, size]);
        let cfg = RunConfig {
            vtree_spec: "hypergraph-bisect".to_string(),
            components: ComponentPolicy::Split,
            ..Default::default()
        };
        let built = build_vtree(&formula, &cfg, &SelectionCtx::plain())
            .unwrap_or_else(|e| panic!("a {size}-variable component must build: {e}"));
        assert_eq!(
            built.selections.len(),
            2,
            "two {size}-variable chains are two components",
        );
        for sel in &built.selections {
            assert_eq!(
                sel.winning_spec.as_deref(),
                Some(built_by),
                "a component of {size} variables must be built by {built_by}",
            );
        }
    }
}
