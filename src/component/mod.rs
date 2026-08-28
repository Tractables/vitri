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
//! - **LOCAL** — each component is renumbered to a dense `0..K-1` space by
//!   [`CnfFormula::extract_component`], and its own vtree's leaves are LOCAL
//!   `VarId`s. [`ComponentVtree::local_to_outer`] is the correspondence:
//!   `local_to_outer[l]` is the OUTER `VarId` of local variable `l`.
//!
//! Both are 0-based `VarId`s here — the 1-based DIMACS convention appears only at
//! the serialization boundary (`bundle`). Mixing the two is the single easiest
//! way to silently corrupt a vtree, which is why the mapping travels bundled with
//! every component vtree rather than being recoverable only by re-deriving the
//! split.

use std::fmt;
use std::sync::Arc;

use crate::vtree::{VarId, Vtree, VtreeArena, VtreeIdx};

use crate::candidates::CandidateSet;
use crate::cnf::{CnfFormula, Local, ShowMask, ShowSet};
use crate::config::{ComponentPolicy, ConstructionBudget, RunConfig};
use crate::decompose::{BuildLimits, SelectionCtx};
use crate::diagnostics::diag;
use crate::error::VitriError;
use crate::spec::{
    BALANCED_SPEC, BuildRequest, ParsedSpec, SelectionRecord, VtreeArtifacts,
    build_one_vtree_artifacts, parse_vtree_spec,
};

// ── Component descriptors ────────────────────────────────────────────────────

/// A pre-built vtree for an independent component, paired with its clause
/// indices and variable mapping. Created during vtree construction, consumed
/// during compilation.
///
/// `vtree`'s leaves are LOCAL `VarId`s (`0..local_to_outer.len()`);
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
    /// Maps LOCAL `VarId` (0..K-1) → OUTER `VarId`.
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
    /// The component's clauses, renumbered to a dense `0..K-1` LOCAL space.
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
    /// Total wall time spent constructing this result, from the shared
    /// construction entry clock through the complete whole or grafted vtree.
    ///
    /// Unlike [`BuildLimitsReport::spent_ms`](crate::decompose::BuildLimitsReport::spent_ms),
    /// this includes setup, simple constructions, component orchestration and
    /// grafting rather than only portfolio builds that report against a wall.
    pub construction_ms: u64,
}

// ── Spec adjustment ──────────────────────────────────────────────────────────

/// Does `spec` name a construction that benefits from per-component
/// construction? The grammar's own
/// [`VtreeBase::is_structural`](crate::spec::VtreeBase::is_structural) decides,
/// read off the family the one parse already resolved, so this stays in
/// lock-step with the validator/builder's notion of a structural base.
fn is_structural_spec(spec: &ParsedSpec<'_>) -> bool {
    spec.family.is_structural()
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
/// let formula = CnfFormula {
///     num_vars: 3,
///     clauses: vec![
///         Clause::new(vec![Literal::from(1), Literal::from(2)]),
///         Clause::new(vec![Literal::from(2), Literal::from(3)]),
///     ],
/// };
/// let config = RunConfig {
///     vtree_spec: "linear".to_string(),
///     ..RunConfig::default()
/// };
/// let build = build_vtree(&formula, &config, &SelectionCtx::plain())?;
///
/// // One leaf per variable, each variable on exactly one of them.
/// assert_eq!(build.vtree.num_leaves(), formula.num_vars);
/// let mut vars: Vec<u32> = build.vtree.leaf_bottomup().map(|(_, v)| v.0).collect();
/// vars.sort();
/// assert_eq!(vars, [0, 1, 2]);
/// # Ok::<(), vitri::VitriError>(())
/// ```
pub fn build_vtree(
    formula: &CnfFormula,
    config: &RunConfig,
    selection: &SelectionCtx,
) -> Result<VtreeBuild, VitriError> {
    config.validate()?;
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
/// construction reads: the spec the run will actually build ([`spec_for_size`])
/// and the [`BuildLimits`] it may spend.
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
    if formula.num_vars == 0 {
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
    let mut built = build_vtree_split(request, config.components, &mut ())?;
    built.construction_ms = started.elapsed().as_millis() as u64;
    Ok(built)
}

// ── Per-component construction ───────────────────────────────────────────────

/// What the per-component construction loop reports as it goes, for a caller
/// that needs to assert what the loop DID rather than only what it returned.
///
/// Production builds observe through `()`, whose empty implementation these
/// default bodies are: the calls monomorphize away, so a release build has no
/// trace value to fill, thread or discard. The recording implementation lives
/// in the test tree, which is the only place anything implements this.
pub(crate) trait BuildObserver {
    /// A component reused an earlier component's vtree out of the per-build
    /// cache instead of constructing a fresh one.
    fn cached_vtree_reused(&mut self) {}
    /// `mask` is the LOCAL show mask installed for this component's
    /// projection-aware selection.
    fn component_show_mask(&mut self, _mask: &ShowMask) {}
}

impl BuildObserver for () {}

/// Canonical identity of a component-local CNF for the per-build vtree cache.
/// Two components map to the same key iff their local-numbered clause sets are
/// identical after normalization (literals sorted within each clause, clauses
/// sorted, num_vars included) and the component-local show mask construction
/// would install also matches — two components with identical clauses but
/// different show masks feed portfolio's selection different inputs, so they
/// must not share a cached vtree.
#[derive(PartialEq, Eq, Hash)]
struct ComponentKey {
    num_vars: u32,
    /// Normal form: each clause's `(var_id, positive)` literals sorted, then the
    /// clause list sorted. Deterministic — order-insensitive to the source CNF.
    clauses: Vec<Vec<(u32, bool)>>,
    /// The local show mask construction would see (`None` for plain MC or the
    /// tiny minfill path, which ignores the show mask).
    show: Option<crate::cnf::ShowMask>,
}

impl ComponentKey {
    fn new(sub: &CnfFormula, show: Option<crate::cnf::ShowMask>) -> Self {
        let mut clauses: Vec<Vec<(u32, bool)>> = sub
            .clauses
            .iter()
            .map(|c| {
                let mut lits: Vec<(u32, bool)> =
                    c.literals.iter().map(|l| (l.var.0, l.positive)).collect();
                lits.sort_unstable();
                lits
            })
            .collect();
        clauses.sort_unstable();
        ComponentKey {
            num_vars: sub.num_vars,
            clauses,
            show,
        }
    }
}

/// The component size up to which construction takes the minfill path.
const TINY_COMPONENT_MAX_VARS: u32 = 30;

/// Whether a component of `num_vars` variables is built by minfill rather than
/// by the requested spec. Asked once per component, since the answer decides
/// both the builder and — minfill ignoring the show mask — whether the cache key
/// carries one.
const fn is_tiny_component(num_vars: u32) -> bool {
    num_vars <= TINY_COMPONENT_MAX_VARS
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
        Err(_) => (
            Arc::new(Vtree::balanced(sub.num_vars)),
            SelectionRecord {
                winning_spec: Some(BALANCED_SPEC.to_string()),
                scores: None,
                td_meta: None,
            },
        ),
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
/// no-op for the strategies [`is_structural_spec`] answers `false` for.
///
/// `policy` is the caller's opt-out: [`ComponentPolicy::Whole`] builds one vtree
/// over the whole formula whatever its component structure. `observer` hears
/// what the loop decided; production passes `&mut ()` and hears nothing.
pub(crate) fn build_vtree_split<O: BuildObserver>(
    req: BuildRequest<'_>,
    policy: ComponentPolicy,
    observer: &mut O,
) -> Result<VtreeBuild, VitriError> {
    if !policy.is_whole()
        && is_structural_spec(req.spec)
        && let Some(comps) = req.formula.detect_components()
    {
        diag!(
            "[components] {} independent sub-problems detected",
            comps.len()
        );
        return build_per_component(req, &comps, observer);
    }

    let built = build_one_vtree_artifacts(req)?;
    assert_one_leaf_per_var(
        &built.vtree,
        req.formula.num_vars,
        format_args!("vtree for spec {:?}", req.spec.raw),
    );
    Ok(VtreeBuild {
        vtree: built.vtree,
        components: None,
        selections: vec![built.selection],
        candidate_sets: vec![built.candidate_set],
        limits: built.limits,
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
fn build_per_component<O: BuildObserver>(
    req: BuildRequest<'_>,
    comps: &[Vec<usize>],
    observer: &mut O,
) -> Result<VtreeBuild, VitriError> {
    let BuildRequest {
        formula,
        spec,
        ctx,
        limits,
    } = req;
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
    let mut in_component = vec![false; formula.num_vars as usize];
    // Memoize component-local vtree construction across structurally
    // identical components within this build (repeated gadgets are common
    // in real CNFs), keyed by the component's local CNF normal form plus
    // the local show mask construction would see. Scoped to this call, no
    // global state. Only reached on the multi-component path —
    // single-component formulas skip this machinery entirely.
    //
    // The retained candidate set is cached alongside the vtree: identical
    // components score identically, so the second one's candidate set is
    // the first one's, and recomputing it would duplicate exactly the work
    // the cache exists to avoid. Empty on the default path, so this costs a
    // moved empty `Vec` per entry.
    let mut vtree_cache: std::collections::HashMap<ComponentKey, VtreeArtifacts> =
        std::collections::HashMap::new();
    // `limits.deadline` is one absolute budget for the whole build, divided
    // between the components by clause count.
    //
    // A component whose share is already zero starts expired, and portfolio
    // answers that with a construction error — so there is no separate
    // deadline check here, and the error propagates straight out of this
    // loop (`?` below), aborting the whole multi-component build. Tiny
    // components skip this: minfill takes no deadline and cannot fail this
    // way.
    let mut clauses_left: usize = comps.iter().map(|c| c.len()).sum();
    for comp_indices in comps {
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
        let tiny = is_tiny_component(sub_formula.num_vars);
        // The component-local show mask that construction would see: the view's
        // own restriction, computed once and reused as both the cache key's show
        // axis and the per-component `SelectionCtx` payload.
        let local_show = if tiny {
            None
        } else {
            comp_show.map(|s| s.mask(sub_formula.num_vars))
        };
        let key = ComponentKey::new(&sub_formula, local_show.clone());
        let artifacts = if let Some(cached) = vtree_cache.get(&key) {
            // Cache hit: an earlier component with an identical local CNF
            // and show mask already built this vtree. The local→outer
            // remap still uses this component's own `local_to_outer`, so
            // reusing the cached vtree needs no remap code of its own —
            // sound because it has one leaf per local variable, exactly
            // what this identical component needs.
            observer.cached_vtree_reused();
            cached.clone()
        } else {
            let built = if tiny {
                tiny_component_artifacts(
                    &sub_formula,
                    crate::decompose::ConversionRequest {
                        spec: Some(crate::decompose::MINFILL_SPEC),
                        reading: spec.reading,
                        effort_scale: crate::budget::vtree_effort_scale(limits.budget_ms),
                        deadline: limits.deadline,
                        trace: ctx.conversion.trace,
                    },
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
                if let Some(local) = &local_show {
                    observer.component_show_mask(local);
                }
                build_one_vtree_artifacts(BuildRequest {
                    formula: &sub_formula,
                    spec,
                    ctx: &comp_ctx,
                    limits: &comp_limits,
                })?
            };
            limits_report.absorb(built.limits.clone());
            vtree_cache.insert(key, built.clone());
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
            sub_formula.num_vars,
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
    let free_vars: Vec<VarId> = (0..formula.num_vars)
        .map(VarId)
        .filter(|v| !in_component[v.idx()])
        .collect();
    let full_vtree = Arc::new(graft_component_vtrees(
        &comp_vtrees,
        &free_vars,
        formula.num_vars,
    ));
    assert_one_leaf_per_var(
        &full_vtree,
        formula.num_vars,
        format_args!("grafted full vtree"),
    );
    Ok(VtreeBuild {
        vtree: full_vtree,
        components: Some(comp_vtrees),
        selections,
        candidate_sets,
        limits: limits_report,
        // Filled by `build_vtree_anchored` after grafting completes.
        construction_ms: 0,
    })
}

// ── Grafting ─────────────────────────────────────────────────────────────

/// Graft per-component vtrees and free variables into a single vtree.
///
/// Each component contributes its own vtree, over LOCAL variable ids, through
/// its [`ComponentVtree::local_to_outer`] map. Free variables (not in any
/// component) are added as leaves. Component roots are joined via a right-linear
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
