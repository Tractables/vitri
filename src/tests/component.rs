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

/// Construction telemetry is part of every public build result: simple
/// constructions, a whole-formula portfolio, and a component graft all take
/// their value from the same entry clock. No elapsed magnitude is contractual.
#[test]
fn construction_time_is_present_on_simple_portfolio_and_component_results() {
    let simple_formula = chain_components(&[9]);
    for spec in ["linear", "portfolio"] {
        let config = RunConfig {
            vtree_spec: spec.to_string(),
            ..RunConfig::default()
        };
        let built = build_vtree(&simple_formula, &config, &SelectionCtx::plain())
            .unwrap_or_else(|e| panic!("{spec} must build: {e}"));
        let _: u64 = built.construction_ms;
    }

    let split_formula = chain_components(&[5, 6]);
    let split = build_vtree(
        &split_formula,
        &RunConfig {
            vtree_spec: "portfolio".to_string(),
            components: ComponentPolicy::Split,
            ..RunConfig::default()
        },
        &SelectionCtx::plain(),
    )
    .expect("the component portfolio must build");
    assert!(split.components.is_some());
    let _: u64 = split.construction_ms;
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

/// The tiny-component construction takes no deadline at all, so a build that
/// starts with none left still produces a vtree, and by the min-fill shortcut
/// rather than by the portfolio's one short attempt.
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
/// a deadline that has ALREADY passed still returns a vtree, on the
/// single-component path and on the split alike. The portfolio gives its first
/// candidate one short attempt instead of skipping the whole catalog, so a
/// spent budget costs the caller the rest of the catalog rather than the tree.
///
/// The single chain can be small, because the deadline is spent before
/// construction starts and nothing about the formula's shape is ever reached.
/// The pair has to be 32 variables each: at or under the tiny threshold a
/// component takes the min-fill shortcut, which never consults the deadline and
/// so would not exercise this at all. The deadline is one absolute budget shared
/// across the split, so both components start past it and each takes its own
/// attempt.
#[test]
fn an_expired_vtree_deadline_still_builds_a_vtree() {
    // An already-gone run deadline: the construction deadline the entry point
    // derives from it is gone too.
    let expired = RunConfig {
        deadline: Some(std::time::Instant::now() - std::time::Duration::from_secs(1)),
        ..RunConfig::default()
    };

    let formula = chain_components(&[9]);
    let built = build_vtree(&formula, &expired, &SelectionCtx::plain())
        .expect("a spent deadline must still hand back a vtree");
    assert_covers_all_vars(&built.vtree, formula.num_vars, "the single-component build");
    assert!(
        !built.limits.skipped.is_empty(),
        "the candidates behind the one attempt must be reported as never started",
    );

    let two = chain_components(&[32, 32]);
    let grafted = build_vtree(&two, &expired, &SelectionCtx::plain())
        .expect("a spent deadline must still hand back a grafted vtree");
    assert_covers_all_vars(&grafted.vtree, two.num_vars, "the graft");
    assert_eq!(
        grafted.components.as_ref().map(Vec::len),
        Some(2),
        "two independent chains are two components",
    );
    assert_eq!(
        grafted.limits.truncated_builds, 2,
        "each component's build takes its own attempt and leaves the rest of the catalog unstarted",
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

// ── The deterministic construction budget ────────────────────────────────────

/// The property the budget exists for: two constructions over the same formula
/// at the same unit budget do the same work, consider the same candidates and
/// select the same vtree.
///
/// Nothing asserted here is a duration, so nothing here depends on how fast or
/// how loaded the machine is — which is the whole claim. A wall-clock budget
/// cannot promise it: which candidates a loaded machine gets through is what
/// decides the tree.
#[test]
fn a_deterministic_budget_spends_the_same_work_and_selects_the_same_vtree() {
    let formula = two_chains();
    let config = RunConfig {
        construction_budget: crate::config::ConstructionBudget::for_wall_ms(2_000),
        ..Default::default()
    };

    let runs: Vec<(u64, VtreeBuild)> = (0..3)
        .map(|_| {
            let before = crate::decompose::meter::units_spent();
            let built =
                build_vtree(&formula, &config, &SelectionCtx::plain()).expect("the fixture builds");
            (crate::decompose::meter::units_spent() - before, built)
        })
        .collect();

    let (first_units, first) = &runs[0];
    for (i, (units, built)) in runs.iter().enumerate().skip(1) {
        assert_eq!(
            units, first_units,
            "run {i} charged a different amount of work"
        );
        assert!(
            built.vtree.same_tree(&first.vtree),
            "run {i} selected a different vtree",
        );
        let won: Vec<_> = built.selections.iter().map(|s| &s.winning_spec).collect();
        let won_first: Vec<_> = first.selections.iter().map(|s| &s.winning_spec).collect();
        assert_eq!(won, won_first, "run {i} selected a different construction");
    }
    assert!(
        *first_units > 0,
        "a construction that charged nothing is not being metered at all",
    );
}

/// The work a construction does is a function of its formula and its budget
/// and of nothing else — in particular not of what this process built before
/// it. A construction that inherited state from an earlier one would be
/// reproducible only in a process that had run the same things in the same
/// order, which is not reproducible at all.
#[test]
fn a_construction_carries_nothing_over_from_the_one_before_it() {
    let formula = two_chains();
    let spend = |ms: u64| {
        let config = RunConfig {
            construction_budget: crate::config::ConstructionBudget::for_wall_ms(ms),
            ..Default::default()
        };
        let before = crate::decompose::meter::units_spent();
        build_vtree(&formula, &config, &SelectionCtx::plain()).expect("the fixture builds");
        crate::decompose::meter::units_spent() - before
    };
    let first = spend(2_000);
    // A build at a different budget in between, so anything a construction
    // leaves behind for the next one has a different value to leave.
    spend(50);
    assert_eq!(
        spend(2_000),
        first,
        "the same construction charged differently after an unrelated build",
    );
}

/// What the wall bounds did is a property of the BUILDS, so a build that walked
/// its whole catalog says so — and says how long it took, which is the number a
/// caller reading a result file cannot otherwise recover.
#[test]
fn a_build_that_walked_its_whole_catalog_reports_no_truncation() {
    let build = build_vtree(
        &crate::tests::common::wide_component(),
        &RunConfig::default(),
        &SelectionCtx::plain(),
    )
    .expect("the fixture builds");
    assert_eq!(build.limits.complete_builds, 1);
    assert_eq!(build.limits.truncated_builds, 0);
    assert!(
        build.limits.skipped.is_empty(),
        "nothing was skipped: {:?}",
        build.limits.skipped,
    );
}

/// A component that took its vtree out of the cache was not constructed, and
/// counting it would report construction time nobody spent. Two identical
/// components, one build.
#[test]
fn the_wall_report_counts_the_components_that_were_actually_built() {
    let build = build_vtree(&two_chains(), &RunConfig::default(), &SelectionCtx::plain())
        .expect("the fixture builds");
    assert_eq!(
        build.components.as_ref().map(|c| c.len()),
        Some(2),
        "the fixture must split, or there is no reuse to report on",
    );
    assert_eq!(
        build.limits.complete_builds, 1,
        "the second component reused the first one's vtree, which spends no \
         construction wall",
    );
    assert_eq!(build.limits.truncated_builds, 0);
}

/// The per-component reports add up into one report for the construction, which
/// is what makes the returned counts answer "what did this build do" rather
/// than "what did its last component do".
#[test]
fn the_reports_of_several_builds_add_up_into_one() {
    use crate::decompose::BuildLimitsReport;
    let mut whole = BuildLimitsReport::default();
    whole.absorb(BuildLimitsReport {
        truncated_builds: 1,
        complete_builds: 0,
        spent_ms: 700,
        skipped: vec!["goatd-incidence".to_string()],
    });
    whole.absorb(BuildLimitsReport {
        truncated_builds: 0,
        complete_builds: 2,
        spent_ms: 40,
        skipped: Vec::new(),
    });
    assert_eq!(
        whole,
        BuildLimitsReport {
            truncated_builds: 1,
            complete_builds: 2,
            spent_ms: 740,
            skipped: vec!["goatd-incidence".to_string()],
        },
    );
}
