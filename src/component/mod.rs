//! Per-component vtree orchestration: split a formula into its independent
//! components, build one vtree per component under a shared budget, and
//! graft them into a single whole-formula vtree.
//!
//! This is the layer above the single-spec construction dispatch in
//! [`spec`](crate::spec)
//! — construction dispatch decides *how* one vtree is built, this module decides
//! *what formulas* get their own vtree and *how the budget is divided between
//! them*. It is the production selection path — the standalone tool and an
//! embedding caller both come through here, so a consumer driving the library
//! gets the same vtree the `vitri` binary writes out.
//!
//! # Variable numbering
//!
//! Two spaces coexist and every item here states which one it is in:
//!
//! - **OUTER** — the variable space of the formula handed to [`build_vtree`].
//!   [`ComponentVtree::clause_indices`] index that formula's clause list, and
//!   the grafted whole-formula vtree's leaves are OUTER `VarId`s. Which of the
//!   crate's [three spaces](crate::cnf::Space) that is belongs to the caller:
//!   reached through [`crate::bundle::run`] it is always REDUCED, the space of
//!   `reduced.cnf`, which is the only formula this crate builds a vtree over.
//! - **LOCAL** — each component is renumbered to a dense `1..=K` space by
//!   [`CnfFormula::extract_component`], and its own vtree's leaves are LOCAL
//!   `VarId`s. [`ComponentVtree::local_to_outer`] is the correspondence:
//!   `local_to_outer[l.idx()]` is the OUTER `VarId` of local variable `l`.
//!
//! Mixing the two is the single easiest way to silently corrupt a vtree, which
//! is why the mapping travels bundled with every component vtree rather than
//! being recoverable only by re-deriving the split.

mod canonical;

use std::fmt;
use std::sync::Arc;

use crate::vtree::{VarId, Vtree, VtreeArena, VtreeIdx};

use self::canonical::canonical_form;

use crate::candidates::CandidateSet;
use crate::cnf::{CnfFormula, Local, ShowMask, ShowSet};
use crate::config::{ComponentPolicy, ConstructionBudget, RunConfig};
use crate::decompose::{BuildLimits, SelectionCtx};
use crate::diagnostics::diag;
use crate::error::VitriError;
use crate::spec::{
    BALANCED_SPEC, BuildRequest, SelectionRecord, VtreeArtifacts, VtreeBase,
    build_one_vtree_artifacts, parse_vtree_spec,
};

// ── Component descriptors ────────────────────────────────────────────────────

/// A pre-built vtree for an independent component, paired with its clause
/// indices and variable mapping. Created during vtree construction, consumed
/// during compilation.
///
/// `vtree`'s leaves are LOCAL `VarId`s (`1..=local_to_outer.len()`);
/// `clause_indices` and `local_to_outer`'s values are OUTER — see the module
/// docs.
///
/// The component-local view the vtree was built over — the renumbered CNF and
/// the restricted show set — is deliberately NOT carried here. One of these
/// exists per component and lives as long as the build, so keeping the views
/// would hold a second copy of the whole clause set, renumbered and split up,
/// from construction until the last file is written, on the largest formulas as
/// much as the small ones. The writer re-derives what it needs instead: that
/// derivation is pure, so it costs a pass over the component's clauses and
/// cannot disagree with what construction saw.
#[derive(Clone, Debug)]
pub struct ComponentVtree {
    /// Vtree containing only this component's variables, over its LOCAL space.
    pub vtree: Arc<Vtree>,
    /// Clause indices (into the outer formula) belonging to this component.
    pub clause_indices: Vec<usize>,
    /// Maps LOCAL `VarId` → OUTER `VarId`: entry `l.idx()` for local `l`.
    pub local_to_outer: Vec<VarId>,
}

/// A component seen from inside: its clauses renumbered to their own LOCAL
/// space, the show set restricted to that space, and the map back out.
///
/// The vtree a component is built over and the CNF written beside that vtree
/// are the same formula in the same numbering, and the `c p show` line of that
/// CNF is the same restriction of the outer show set that selection scored the
/// component under. Deriving all of it in one place is what keeps those
/// agreements true by construction rather than by two ends happening to spell
/// the same derivation.
pub(crate) struct LocalView {
    /// The component's clauses, renumbered to a dense `1..=K` LOCAL space.
    pub formula: CnfFormula,
    /// The outer show set read over that space, or `None` when the instance
    /// declares none.
    pub show: Option<ShowSet<Local>>,
    /// LOCAL id → the outer id it stands for, as in
    /// [`ComponentVtree::local_to_outer`].
    pub local_to_outer: Vec<VarId>,
}

/// The [`LocalView`] of the component `clause_indices` cuts out of `formula`,
/// under the outer show mask `show` (`None` when the instance is unprojected).
pub(crate) fn local_view(
    formula: &CnfFormula,
    clause_indices: &[usize],
    show: Option<&ShowMask>,
) -> LocalView {
    let (sub, local_to_outer) = formula.extract_component(clause_indices);
    LocalView {
        show: show.map(|m| m.restrict(&local_to_outer)),
        formula: sub,
        local_to_outer,
    }
}

/// Everything one vtree build produced: the whole-formula vtree, the component
/// split it was grafted from (if any), and the retained candidate sets.
///
/// The first two fields are what a consuming compiler needs; `candidate_sets` is
/// export-only — nothing in selection reads it, and it is empty unless the
/// caller explicitly asked for a candidate set. Bundling the three keeps the
/// candidate set a by-product of the one selection path, not a second pass to
/// reconstruct.
#[derive(Debug)]
pub struct VtreeBuild {
    /// Vtree over the whole formula's variable space (the graft, when split).
    pub vtree: Arc<Vtree>,
    /// Per-component descriptors, or `None` when the formula was built whole.
    pub components: Option<Vec<ComponentVtree>>,
    /// What each component's construction reported about the vtree it selected,
    /// one per component in the same order as `components` — or exactly one
    /// entry when `components` is `None`. A construction with nothing to report
    /// contributes a default entry.
    pub selections: Vec<SelectionRecord>,
    /// Retained candidate sets, aligned with `selections` and populated the
    /// same way: one per component, or exactly one entry when `components` is
    /// `None`.
    ///
    /// The ENTRY is empty when there was nothing to retain — the caller did not
    /// ask for a set ([`RunConfig::candidates`] ≤ 1, the default, the zero-cost
    /// path), or the component's vtree came straight from minfill rather than
    /// the portfolio, its one candidate being the vtree already emitted beside
    /// it. An empty entry is that answer; a missing one would be the same
    /// answer spelled a second way.
    pub candidate_sets: Vec<CandidateSet>,
    /// What the construction's wall bounds did — summed over every component
    /// this build actually constructed. See
    /// [`BuildLimitsReport`](crate::decompose::BuildLimitsReport).
    pub limits: crate::decompose::BuildLimitsReport,
    /// How many components reused an earlier component's vtree instead of
    /// constructing one: two components with the same clauses and the same
    /// share of the show set, up to a renaming of their variables, get the
    /// same vtree (renamed to match), and a repeated gadget is built once.
    /// Zero for a formula built whole.
    pub cached_components: usize,
    /// Total wall time spent constructing this result, from the shared
    /// construction entry clock through the complete whole or grafted vtree.
    ///
    /// Unlike [`BuildLimitsReport::spent_ms`](crate::decompose::BuildLimitsReport::spent_ms),
    /// this includes setup, simple constructions, component orchestration and
    /// grafting rather than only portfolio builds that report against a wall.
    pub construction_ms: u64,
}

// ── Entry points ─────────────────────────────────────────────────────────────

/// Build a vtree over `formula` under an explicit [`RunConfig`] — the library
/// entry point, and what a consumer embedding this crate calls.
///
/// The layer this adds is the validated, anchored config: everything below it
/// runs on a config that has already been checked and whose budget is measured
/// from a fixed instant.
///
/// `selection` says what construction should optimize FOR — the projected show
/// mask ([`SelectionCtx::for_show`]) or [`SelectionCtx::plain`] — and
/// `config` says what it may SPEND: this is where the run's budget, its
/// candidate retention and the construction deadline (through the one resolver
/// [`RunConfig::construction_deadline`]) are read off `config`, once, for the
/// whole build.
///
/// `formula` must already be the reduced formula (see
/// [`crate::bundle::preprocess`]) — this builds a vtree over whatever it is
/// handed, and a vtree over the raw CNF does not fit the reduced one.
///
/// # Errors
///
/// [`VitriError::Config`] for a request this crate refuses (see
/// [`RunConfig::validate`]), [`VitriError::Spec`] for a `--vtree` string naming
/// a construction this crate does not have or carrying a token its family
/// cannot honor, [`VitriError::Input`] for a formula with no variables to build
/// over, [`VitriError::Env`] for a `VITRI_*` variable the construction reads,
/// and [`VitriError::Construction`] when the chosen construction ran and could
/// not produce a vtree.
///
/// # Examples
///
/// ```
/// use vitri::cnf::{Clause, CnfFormula, Literal};
/// use vitri::component::build_vtree;
/// use vitri::config::RunConfig;
/// use vitri::decompose::SelectionCtx;
///
/// // (x1 ∨ x2) ∧ (x2 ∨ x3).
/// let formula = CnfFormula::new(
///     3,
///     vec![
///         Clause::new(vec![Literal::from(1), Literal::from(2)]),
///         Clause::new(vec![Literal::from(2), Literal::from(3)]),
///     ],
/// )?;
/// let config = RunConfig {
///     vtree_spec: "linear".to_string(),
///     ..RunConfig::default()
/// };
/// let build = build_vtree(&formula, &config, &SelectionCtx::plain())?;
///
/// // One leaf per variable, each variable on exactly one of them.
/// assert_eq!(build.vtree.num_leaves(), formula.num_vars());
/// let mut vars: Vec<u32> = build.vtree.leaf_bottomup().map(|(_, v)| v.get()).collect();
/// vars.sort();
/// assert_eq!(vars, [1, 2, 3]);
/// # Ok::<(), vitri::VitriError>(())
/// ```
pub fn build_vtree(
    formula: &CnfFormula,
    config: &RunConfig,
    selection: &SelectionCtx,
) -> Result<VtreeBuild, VitriError> {
    config.validate()?;
    selection.goatd.validate()?;
    // Called on its own, this call IS the run, so it starts the clock. Reached
    // through [`crate::run`], preprocessing has already spent part of the
    // budget and the anchored config says how much is left.
    build_vtree_anchored(
        formula,
        &config.anchored(std::time::Instant::now()),
        selection,
    )
}

/// [`build_vtree`] on a config that has been validated and whose budget is
/// already anchored ([`RunConfig::anchored`]) — the body of the public entry,
/// and what [`crate::run`] calls with what preprocessing left of the budget.
///
/// The layer this adds is the resolution of the config into what one
/// construction reads: the run's spec, parsed once by
/// [`parse_vtree_spec`](crate::spec::parse_vtree_spec) and filled out from the
/// run's own reading, and the [`BuildLimits`] it may spend.
///
/// `selection` carries the show mask, when there is one, in the var space of
/// `formula`. On a multi-component formula each component is renumbered into its
/// own local var space ([`CnfFormula::extract_component`]), so the mask is
/// remapped per component before projection-aware selection reads it.
pub(crate) fn build_vtree_anchored(
    formula: &CnfFormula,
    config: &RunConfig,
    selection: &SelectionCtx,
) -> Result<VtreeBuild, VitriError> {
    // Every construction below reaches a constructor that requires at least one
    // leaf and says so by panicking. Reported here instead: a formula is
    // caller-supplied input, and this entry answers for one that cannot be
    // built over rather than aborting the process the library is embedded in.
    if formula.num_vars() == 0 {
        return Err(VitriError::input(
            "the formula declares 0 variables; a vtree has at least one leaf, so there is \
             nothing to build one over",
        ));
    }
    // How much of the run construction gets is the caller's to say and
    // `construction_deadline` is where it is said. The clock is read where
    // construction starts rather than where the run did, because the default
    // policy is a share of what is still LEFT.
    //
    // It is also where a deterministic budget arms the construction meter, at
    // that same instant — the one the deadline just resolved is counted forward
    // from. The guard lives to the end of this call, so everything built below
    // spends ONE budget and nothing after it is metered.
    let started = std::time::Instant::now();
    let _metered = matches!(
        config.construction_budget,
        ConstructionBudget::Deterministic { .. }
    )
    .then(|| crate::decompose::meter::arm(started));
    let limits = BuildLimits {
        deadline: config.construction_deadline(started),
        budget_ms: config.budget_ms,
        candidates: config.candidates,
    };
    // The one parse of the run: everything below reads the typed value, so a
    // formula that splits into components does not re-read the grammar per
    // component.
    let mut parsed = parse_vtree_spec(&config.vtree_spec)?;
    // The run's own reading fills whatever the spec left open, once, so every
    // component of one formula is read the same way.
    parsed.inherit(config.reading);
    let request = BuildRequest {
        formula,
        spec: &parsed,
        ctx: selection,
        limits: &limits,
    };
    let mut built = build_vtree_split(request, config.components)?;
    built.construction_ms = started.elapsed().as_millis() as u64;
    Ok(built)
}

// ── Per-component construction ───────────────────────────────────────────────

/// The component size up to which construction takes the minfill path.
const TINY_COMPONENT_MAX_VARS: u32 = 30;

/// Whether a component of `num_vars` variables is built by minfill rather than
/// by the requested spec. Asked once per component, since the answer decides
/// both the builder and — minfill ignoring the show mask — whether the cache key
/// carries one.
const fn is_tiny_component(num_vars: u32) -> bool {
    num_vars <= TINY_COMPONENT_MAX_VARS
}

/// The fewest variables, and the fewest clauses per variable, at which a split
/// function-preserving formula is built with `force` on every component rather
/// than the portfolio ([`is_large_and_clause_dense`]), provided no component
/// dominates the split ([`no_component_dominates`]).
const DENSE_SPLIT_MIN_VARS: u32 = 1000;
const DENSE_SPLIT_MIN_CLAUSES_PER_VAR: f64 = 4.5;

/// Is `formula` large and clause-dense enough that, when it is a
/// function-preserving reduction that splits into components, `force` on every
/// component compiles better than the portfolio?
///
/// Measured over function-preserving reductions of the 780 solvable Model
/// Counting Competition track-1 instances, compiled by a bottom-up TDD
/// compiler at a two-minute timeout: the formulas this admits that also split
/// compile 17 more under `force`, in less total time on those both compile,
/// while `force` on every formula compiles far fewer than the portfolio. The
/// three this admits that compile only under the portfolio are each one
/// component and a few small ones, which [`no_component_dominates`] leaves to
/// the portfolio. Counting-mode reductions eliminate variables first, which
/// changes both measures, and there the same rule gained nothing; so only a
/// function-preserving formula is asked ([`SelectionCtx::preserves_function`]).
fn is_large_and_clause_dense(formula: &CnfFormula) -> bool {
    formula.num_vars() >= DENSE_SPLIT_MIN_VARS
        && formula.clauses().len() as f64
            >= DENSE_SPLIT_MIN_CLAUSES_PER_VAR * f64::from(formula.num_vars())
}

/// The largest share of a split formula's variables one component may hold for
/// the rule that builds every component with `force` to apply
/// ([`no_component_dominates`]).
///
/// Calibrated on the same reductions as [`is_large_and_clause_dense`]: of the
/// 42 formulas that rule admitted, the 17 that compiled only under `force` have
/// no component holding more than 0.63 of the variables, and the three that
/// compiled only under the portfolio have one holding 0.98 or more. Three
/// more have a component holding 0.94 or more, and compiled under neither;
/// in none of the other 19 does a component hold more than 0.68.
const DENSE_SPLIT_MAX_LARGEST_SHARE: f64 = 0.75;

/// Does no component of the split `comps` of `formula` hold more than
/// [`DENSE_SPLIT_MAX_LARGEST_SHARE`] of the variables the components cover?
///
/// A formula that splits only by shedding a few small components is still one
/// formula to build, and on that one the portfolio compiled better than
/// `force`; the rule is for formulas whose structure really falls apart.
fn no_component_dominates(formula: &CnfFormula, comps: &[Vec<usize>]) -> bool {
    let sizes: Vec<usize> = comps
        .iter()
        .map(|clauses| formula.component_vars(clauses).len())
        .collect();
    let covered: usize = sizes.iter().sum();
    let largest = sizes.iter().copied().max().unwrap_or(0);
    largest as f64 <= DENSE_SPLIT_MAX_LARGEST_SHARE * covered as f64
}

/// Vtree-builder invariant: exactly one leaf per variable of the formula the
/// vtree serves. Catches a malformed vtree at the construction site instead of
/// downstream in compile, where the symptom (a model count that collapses to 0)
/// is far harder to trace. `what` names which vtree broke it.
fn assert_one_leaf_per_var(vtree: &Vtree, num_vars: u32, what: fmt::Arguments<'_>) {
    assert_eq!(
        vtree.num_leaves(),
        num_vars,
        "{what}: leaf count ({}) ≠ num_vars ({}) — vtree builder produced a malformed vtree",
        vtree.num_leaves(),
        num_vars,
    );
}

/// The vtree a component at or under [`TINY_COMPONENT_MAX_VARS`] gets: minfill,
/// which needs no selection context (it ignores the show mask) and no deadline,
/// so a spent construction budget cannot fail such a component. Its own fallback
/// stands in on the rare case minfill itself errors.
///
/// One candidate, so no candidate set — the component's own vtree is that
/// candidate — but the construction still names itself, and minfill's bag
/// metadata travels with the tree it describes.
fn tiny_component_artifacts(
    sub: &CnfFormula,
    request: crate::decompose::ConversionRequest<'_>,
) -> VtreeArtifacts {
    let (vtree, selection) = match crate::decompose::vtree_from_minfill(
        sub,
        crate::decompose::INTERNAL_ELIMINATION_SEED,
        request,
    ) {
        Ok(b) => (
            b.vtree,
            SelectionRecord {
                winning_spec: Some(crate::decompose::MINFILL_SPEC.to_string()),
                scores: None,
                td_meta: b.td.meta,
            },
        ),
        // A balanced vtree over the same variables rather than a failed build:
        // the component is small enough that the shape hardly matters, and the
        // run has a vtree for every component either way. The diagnostic is how
        // a reader finds out which construction they actually got.
        Err(error) => {
            crate::diagnostics::diag!(
                "minfill on a tiny component failed ({error}); using a balanced vtree"
            );
            (
                Arc::new(Vtree::balanced(sub.num_vars())),
                SelectionRecord {
                    winning_spec: Some(BALANCED_SPEC.to_string()),
                    scores: None,
                    td_meta: None,
                },
            )
        }
    };
    VtreeArtifacts {
        vtree,
        selection,
        candidate_set: CandidateSet::default(),
        // Minfill takes no deadline, so there is no wall here to have bound
        // anything: a component built this way is neither a complete portfolio
        // build nor a truncated one.
        limits: crate::decompose::BuildLimitsReport::default(),
    }
}

/// Builds a separate vtree per independent component and grafts them together,
/// for structural vtree strategies (those using the primal/incidence graph). A
/// no-op for a spec whose base reads no graph
/// ([`VtreeBase::is_structural`](crate::spec::VtreeBase::is_structural)).
///
/// `policy` is the caller's opt-out: [`ComponentPolicy::Whole`] builds one vtree
/// over the whole formula whatever its component structure.
pub(crate) fn build_vtree_split(
    req: BuildRequest<'_>,
    policy: ComponentPolicy,
) -> Result<VtreeBuild, VitriError> {
    if !policy.is_whole()
        && req.spec.family.is_structural()
        && let Some(comps) = req.formula.detect_components()
    {
        diag!(
            "[components] {} independent sub-problems detected",
            comps.len()
        );
        if req.ctx.preserves_function
            && matches!(req.spec.family, VtreeBase::Portfolio)
            && is_large_and_clause_dense(req.formula)
            && no_component_dominates(req.formula, &comps)
        {
            diag!(
                "[components] large, clause-dense, function-preserving and evenly split: force on every component"
            );
            let mut force = parse_vtree_spec("force")?;
            force.inherit(req.spec.reading);
            return build_per_component(
                BuildRequest {
                    spec: &force,
                    ..req
                },
                &comps,
            );
        }
        return build_per_component(req, &comps);
    }

    // Nothing was split, so the portfolio's pick line is about the whole
    // formula.
    crate::score::agg::set_component(None);
    let built = build_one_vtree_artifacts(req)?;
    assert_one_leaf_per_var(
        &built.vtree,
        req.formula.num_vars(),
        format_args!("vtree for spec {:?}", req.spec.raw),
    );
    Ok(VtreeBuild {
        vtree: built.vtree,
        components: None,
        selections: vec![built.selection],
        candidate_sets: vec![built.candidate_set],
        limits: built.limits,
        cached_components: 0,
        // Filled by `build_vtree_anchored`, whose one clock covers both this
        // whole-formula path and the component path below.
        construction_ms: 0,
    })
}

/// One vtree per independent component of `formula` — `comps` is the split
/// [`CnfFormula::detect_components`] found, each entry the clause indices of one
/// component — grafted into a single whole-formula vtree.
///
/// Every component is built over its own LOCAL space and the result is grafted
/// back, so the returned [`VtreeBuild::vtree`] is over `formula`'s space and
/// `components` carries the correspondence.
fn build_per_component(
    req: BuildRequest<'_>,
    comps: &[Vec<usize>],
) -> Result<VtreeBuild, VitriError> {
    let BuildRequest {
        formula,
        spec,
        ctx,
        limits,
    } = req;
    let mut cached_components = 0;
    let mut comp_vtrees = Vec::new();
    // Both aligned 1:1 with `comp_vtrees` — one entry per component,
    // always, whether or not it has anything in it to report.
    let mut candidate_sets: Vec<CandidateSet> = Vec::new();
    let mut selections: Vec<SelectionRecord> = Vec::new();
    // Accumulated where components are BUILT, not where their artifacts are
    // used: a component that took its vtree out of the cache below spent no
    // construction wall, and counting the cached copy would report time that
    // was never spent.
    let mut limits_report = crate::decompose::BuildLimitsReport::default();
    let mut in_component = vec![false; formula.num_vars() as usize];
    // Memoize component-local vtree construction across components that are
    // identical up to a renaming of their variables within this build
    // (repeated gadgets are common in real CNFs). The key is the clause set
    // and the local show mask construction would see, both relabelled into
    // the component's canonical numbering (`canonical`); the cached
    // artifacts are held in that same canonical numbering and renamed back
    // into each user's local numbering. Scoped to this call, no global
    // state. Only reached on the multi-component path —
    // single-component formulas skip this machinery entirely.
    //
    // The retained candidate set is cached alongside the vtree: identical
    // components score identically, so the second one's candidate set is
    // the first one's, and recomputing it would duplicate exactly the work
    // the cache exists to avoid. Empty on the default path, so this costs a
    // moved empty `Vec` per entry.
    let mut vtree_cache: std::collections::HashMap<canonical::ComponentKey, VtreeArtifacts> =
        std::collections::HashMap::new();
    // `limits.deadline` is one absolute budget for the whole build, divided
    // between the components by clause count.
    //
    // A component whose share is already zero starts expired, and portfolio
    // answers that by giving its first candidate one short attempt and
    // reporting the rest as never started — so there is no separate deadline
    // check here, and a budget spent by the earlier components costs the later
    // ones the rest of their catalog rather than the build. Tiny components
    // skip this: minfill takes no deadline and never consults one.
    let mut clauses_left: usize = comps.iter().map(|c| c.len()).sum();
    for (index, comp_indices) in comps.iter().enumerate() {
        // Which component the portfolio's pick line is about. This numbering
        // is the one the written `components/compNNN` files carry, since both
        // walk `comps` in order.
        crate::score::agg::set_component(Some(index));
        let comp_deadline = limits
            .deadline
            .map(|d| crate::budget::pro_rata_deadline(d, comp_indices.len(), clauses_left));
        clauses_left = clauses_left.saturating_sub(comp_indices.len());
        let LocalView {
            formula: sub_formula,
            show: comp_show,
            local_to_outer,
        } = local_view(formula, comp_indices, ctx.objective.show_mask());
        for &v in &local_to_outer {
            in_component[v.idx()] = true;
        }
        // One answer per component, feeding both the builder choice below and
        // the show mask: minfill ignores the mask, so a tiny component is keyed
        // on `None` and two tiny components differing only in their mask share
        // a cache entry, which is exactly right for what was built.
        let tiny = is_tiny_component(sub_formula.num_vars());
        // The component-local show mask that construction would see: the view's
        // own restriction, computed once and reused as both the cache key's show
        // axis and the per-component `SelectionCtx` payload.
        let local_show = if tiny {
            None
        } else {
            comp_show.map(|s| s.mask(sub_formula.num_vars()))
        };
        let canon = canonical_form(&sub_formula, local_show.as_ref());
        let artifacts = if let Some(cached) = vtree_cache.get(&canon.key) {
            // Cache hit: an earlier component with the same clauses and show
            // mask up to a renaming of its variables already built this
            // vtree. The cache holds artifacts in canonical numbering, so
            // they are renamed into this component's local numbering here;
            // the local→outer remap below then applies as for any built one.
            cached_components += 1;
            let canonical_to_local = canon.canonical_to_local();
            cached.relabeled(|c| canonical_to_local[c.idx()])
        } else {
            let built = if tiny {
                tiny_component_artifacts(
                    &sub_formula,
                    crate::decompose::ConversionRequest::of(
                        crate::decompose::MINFILL_SPEC,
                        spec.reading,
                        crate::budget::vtree_effort_scale(limits.budget_ms),
                        // No deadline, which is what the function above says
                        // it takes: a component this small is built out of
                        // whatever the run has left.
                        None,
                        ctx.conversion.trace,
                    ),
                )
            } else {
                // Build a per-component SelectionCtx carrying the remapped
                // local mask so portfolio's show-aware peak metric scores
                // this component's show vars, instead of indexing the
                // outer mask by local id and scoring the wrong variables.
                // `None` local_show → no show mask → selection unchanged.
                // Everything else — cost veto, candidate-set size, every
                // research knob — is inherited from the whole-formula ctx.
                let comp_ctx = SelectionCtx {
                    objective: ctx
                        .objective
                        .with_mask(local_show.clone().map(std::rc::Rc::new)),
                    ..ctx.clone()
                };
                let comp_limits = BuildLimits {
                    deadline: comp_deadline,
                    ..limits.clone()
                };
                build_one_vtree_artifacts(BuildRequest {
                    formula: &sub_formula,
                    spec,
                    ctx: &comp_ctx,
                    limits: &comp_limits,
                })?
            };
            limits_report.absorb(built.limits.clone());
            let canonical_artifacts = built.relabeled(|v| canon.to_canonical(v));
            vtree_cache.insert(canon.key, canonical_artifacts);
            built
        };
        let VtreeArtifacts {
            vtree: sub_vtree,
            selection: sub_selection,
            candidate_set: sub_candidates,
            limits: _,
        } = artifacts;
        assert_one_leaf_per_var(
            &sub_vtree,
            sub_formula.num_vars(),
            format_args!("component vtree for spec {:?}", spec.raw),
        );
        comp_vtrees.push(ComponentVtree {
            vtree: sub_vtree,
            clause_indices: comp_indices.clone(),
            local_to_outer,
        });
        selections.push(sub_selection);
        candidate_sets.push(sub_candidates);
    }
    let free_vars: Vec<VarId> = VarId::all(formula.num_vars())
        .filter(|v| !in_component[v.idx()])
        .collect();
    let full_vtree = Arc::new(graft_component_vtrees(
        &comp_vtrees,
        &free_vars,
        formula.num_vars(),
    ));
    assert_one_leaf_per_var(
        &full_vtree,
        formula.num_vars(),
        format_args!("grafted full vtree"),
    );
    Ok(VtreeBuild {
        vtree: full_vtree,
        components: Some(comp_vtrees),
        selections,
        candidate_sets,
        limits: limits_report,
        cached_components,
        // Filled by `build_vtree_anchored` after grafting completes.
        construction_ms: 0,
    })
}

// ── Grafting ─────────────────────────────────────────────────────────────

/// Graft per-component vtrees and free variables into a single vtree.
///
/// Each component contributes its own vtree, over LOCAL variable ids, through
/// its [`ComponentVtree::local_to_outer`] map. Free variables (not in any
/// component) are added as leaves. Component roots are joined via a left-linear
/// chain (smallest components first, which is how they arrive sorted).
///
/// # Panics
///
/// Panics if `components` and `free_vars` are both empty (nothing to graft).
fn graft_component_vtrees(
    components: &[ComponentVtree],
    free_vars: &[VarId],
    total_vars: u32,
) -> Vtree {
    let mut nodes = VtreeArena::new();
    let mut subtree_roots: Vec<VtreeIdx> = Vec::new();

    for comp in components {
        subtree_roots.push(nodes.graft(&comp.vtree, |local| comp.local_to_outer[local.idx()]));
    }

    for &var in free_vars {
        subtree_roots.push(nodes.leaf(var));
    }

    assert!(
        !subtree_roots.is_empty(),
        "graft_component_vtrees: no components or free vars"
    );
    let mut root = subtree_roots[0];
    for &next_root in &subtree_roots[1..] {
        root = nodes.internal(root, next_root);
    }

    Vtree::from_nodes(nodes.into_nodes(), root, total_vars)
}

#[cfg(test)]
mod tests;
