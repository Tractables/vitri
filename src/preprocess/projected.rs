//! The projection-preserving chain: the stages that are sound when the
//! answer is a count over a SHOW set rather than over every variable.
//!
//! # Why the plain chain cannot be reused here
//!
//! [`crate::preprocess::simplify`]'s chain is count-preserving for plain model
//! counting only. Under a projection each of its stages has a way to be wrong:
//!
//! - **Definable-variable elimination** removes a variable whose value is a
//!   function of the others. If that variable is a SHOW variable and the function
//!   reads HIDDEN variables, two models agreeing on the rest of the show set can
//!   still disagree on it — so removing it MERGES two distinct show-projections
//!   into one and undercounts.
//! - **Equivalence substitution** eliminates one member of `x ≡ y`. Sound only if
//!   the survivor is the SHOW variable; substituting a show variable by a hidden
//!   representative leaves the show set naming a variable the formula no longer
//!   has.
//! - **A free variable's `×2`** is a factor of 2 for a show variable and a factor
//!   of ONE for a projected-out variable, which the shared `2^k` cannot tell
//!   apart.
//!
//! So this module runs a different chain, each stage of which is ×1 for the
//! projected count:
//!
//! 1. [`strengthen_projected_hidden`] — the full DVE pipeline
//!    FROZEN on the show set, so every variable it eliminates is hidden
//!    (∃-absorbed, ×1). It may additionally prove a show variable equivalent to
//!    another SHOW variable; the survivor is forced to be a counted variable and
//!    the eliminated member is dropped from the show set (`pc(S) == pc(S\{elim})`,
//!    still ×1). Those merges are returned so a WEIGHTED caller can fold the
//!    eliminated member's literal weights into its survivor.
//! 2. **Projected BVE** — resolution variable elimination restricted to the
//!    projected-out variables, which is clause-level ∃ and therefore exactly ×1.
//!
//! Both stages PRESERVE variable ids, so the show set, the weight tables and any
//! variable map established upstream (by an Arjun projection minimization) stay
//! valid across them without a second renumbering to compose.
//!
//! # The chain is the whole contract
//!
//! [`strengthen_and_bve`] takes a formula, a show set and the run's deadline and
//! nothing else: there is no stage parameter, no eliminator to supply, and no way
//! to reach [`crate::preprocess::simplify`]'s count-preserving stage list. The
//! projected and weighted-projected modes run the SAME two stages — the only
//! difference between them is what a caller does with
//! [`ProjectedReduction::folds`] afterwards
//! ([`Weights::fold_eliminated`](crate::cnf::Weights::fold_eliminated)).

use crate::cnf::{CnfFormula, EquivFold, Reduced, ShowSet, VarId};
use crate::preprocess::bve_project::bve_project;

/// What [`strengthen_and_bve`] produced.
pub(crate) struct ProjectedReduction {
    /// The reduced formula. Same variable ids and same `num_vars` as the input —
    /// eliminated variables simply stop occurring in clauses.
    pub formula: CnfFormula,
    /// The show set after dropping every variable proved equivalent to another
    /// counted one (see `folds`).
    pub show_set: ShowSet<Reduced>,
    /// One per show variable merged away as equivalent to another SHOW
    /// variable. The survivor is always a counted variable; the show-set drop
    /// is already applied above, so what is left is [`EquivFold`]'s weighted
    /// obligation.
    pub folds: Vec<EquivFold>,
}

/// The strengthening stage's own ceiling. A ceiling rather than a spend: a run
/// whose deadline falls sooner stops at the deadline, and a run with no deadline
/// spends up to this.
const STRENGTHEN_BUDGET_MS: u64 = 3_000;

/// Run the projected reduction: hidden-variable strengthening, then
/// projected BVE.
///
/// `show_set` is the projection the answer is taken over. A caller with no show
/// set is doing plain counting and belongs on the count-preserving chain — every
/// projected entry point guarantees a non-missing show set before reaching here.
///
/// `deadline` is the run's, and it binds here as everywhere else: the
/// strengthening stage gets whatever is left of it, up to
/// [`STRENGTHEN_BUDGET_MS`].
pub(crate) fn strengthen_and_bve(
    formula: &CnfFormula,
    mut show_set: ShowSet<Reduced>,
    deadline: Option<std::time::Instant>,
) -> ProjectedReduction {
    let budget_ms = crate::budget::clamp(
        std::time::Duration::from_millis(STRENGTHEN_BUDGET_MS),
        deadline,
    )
    .as_millis() as u64;
    let (strengthened, folds) = strengthen_projected_hidden(formula, &show_set, 8, budget_ms);
    // Determined (elim) show vars leave the show set — the survivor alone is
    // counted; `elim` is ∃-eliminated by the BVE pass below.
    for f in &folds {
        show_set.remove(f.eliminated);
    }

    let reduced = bve_project(&strengthened, &show_set.mask(formula.num_vars));
    ProjectedReduction {
        formula: reduced,
        show_set,
        folds,
    }
}

/// Projection-aware hidden-variable strengthening for the `pmc` and `pwmc`
/// chains.
///
/// Runs the full DVE pipeline (equivalence merge + resolution VE + definability
/// elimination + vivification) FROZEN on the show variables, so every variable
/// it eliminates is a HIDDEN (projected-out) variable. Each such elimination is
/// ×1 for the projected count: hidden vars are existentially quantified
/// (∃-absorbed), so resolution VE, definability elimination, AND equivalence
/// merge of a hidden var all preserve the projected count exactly — and, since
/// hidden vars are weight-1 in the projected-weighted fold, the projected-
/// weighted count too.
///
/// VarIds are preserved (`keep_original_vars=true`) so the result composes with
/// the id-preserving `bve_project` that runs after it and with the
/// `show`/`free_show` accounting in the projected chain.
///
/// Show vars are eliminated only in two accounted ways. `frozen=show` blocks
/// resolution VE and the definability loop from touching them, so the only
/// removals are:
///   1. **Equivalence merge** (`FrozenEquiv::ForceShowRep`): when a show var is
///      equivalent to another counted var, the survivor is forced to be a show
///      var and the eliminated members are *determined* (×1). DVE gives them the
///      `Equiv` fate; the function returns them so the projected chain drops
///      them from the show set (counting only the survivor).
///   2. **Genuine freeness**: a show var whose clauses are all redundant becomes
///      clauseless (the `Free` fate). It stays in the show set and the projected
///      chain's `×2` free-show factor is correct.
///
/// A backbone forcing a show var is NOT a removal: CaDiCaL vivification re-pins
/// every forced literal as a unit clause, so a forced show var stays present
/// (×1 at its forced value), never mis-marked free. A defensive guard still
/// rejects the result if any show var was eliminated by an unexpected path
/// (neither equiv nor free) — that would signal a `frozen`-protection leak.
///
/// Returns `(residual, determined_show)` where `determined_show` lists, for each
/// show var merged away as equivalent, an [`EquivFold`]. The survivor's
/// variable is ALWAYS a counted (show) var — `ForceShowRep` forces the SCC
/// representative to be a show var.
pub(super) fn strengthen_projected_hidden(
    formula: &CnfFormula,
    show_set: &ShowSet<Reduced>,
    rounds: usize,
    budget_ms: u64,
) -> (CnfFormula, Vec<EquivFold>) {
    // Step 1: count-preserving BCP FIRST. This is essential for soundness, not
    // just speed: DVE has a latent bug when fed unit clauses that force CONFLICTING
    // hidden vars — it eliminates them as "defined" and silently drops the UNSAT
    // (plain MC never hits this because its pipeline propagates units before DVE).
    // `bcp_simplify` propagates all units to fixpoint, detects that UNSAT, re-pins
    // forced SHOW vars as units (so they stay ×1, never mis-counted free), and
    // ∃-absorbs forced hidden vars — leaving DVE a unit-light formula like the one
    // plain MC's pipeline produces.
    let bcp = super::count_preserve::bcp_simplify(formula, &show_set.mask(formula.num_vars));
    if bcp.unsat {
        // Single empty clause → the projected chain's degenerate-residual check reports
        // projected count 0 (UNSAT).
        return (CnfFormula::contradiction(formula.num_vars), Vec::new());
    }

    // Step 2: full DVE FROZEN on the show vars.
    let frozen: rustc_hash::FxHashSet<VarId> = show_set.iter_vars().collect();
    let known_defined = rustc_hash::FxHashSet::default();
    let policy = super::dve::FrozenEquiv::ForceShowRep;
    let res = super::dve::preprocess_dve(
        &bcp.formula,
        rounds,
        budget_ms,
        /*keep_original_vars=*/ true,
        &known_defined,
        &frozen,
        policy,
    );

    // Multiplier soundness. A show var CONSTRAINED in the BCP'd formula can become
    // absent from the residual in two ways:
    //   (a) Equivalence merge: the survivor is a show var, the merged var is
    //       determined ×1 — sound, handed back below.
    //   (b) DVE ∃-eliminates the HIDDEN vars through which the show var was forced.
    //       That elimination is sound for the count on its own, but only because
    //       `bcp_simplify` already re-pinned the forced show var with a unit clause
    //       and DVE keeps a derived unit on a frozen (show) var as a clause instead
    //       of propagating it away — so the show var's constraint always survives
    //       in the residual, never mis-marked free (×2) when it is actually
    //       BACKBONE-forced (×1).
    // Show vars merged away as equivalent are determined: each is functionally
    // fixed by a SURVIVING show var (ForceShowRep forces the SCC representative to
    // be a show var). Hand back an `EquivFold` per merged var so the projected
    // chain can discharge both halves of its obligation. Survivor + composed
    // polarity come from chasing the chain of representatives to its ultimate
    // non-eliminated show var (chains form across DVE rounds). If any chain fails
    // to resolve to a live show var (cannot happen under ForceShowRep; guard for
    // soundness), discard the whole DVE reduction — `res.formula` dropped a show
    // var we cannot account for — and return the BCP'd input unchanged.
    let mut determined: Vec<EquivFold> = Vec::new();
    let mut fold_ok = true;
    for v in show_set.iter_vars().map(|v| v.idx()) {
        if res
            .fates
            .get(v)
            .copied()
            .and_then(super::dve::types::DveFate::as_equiv)
            .is_none()
        {
            continue;
        }
        let survivor = super::weighted_lift::dve_equiv_survivor(&res.fates, v);
        // A survivor that is not FROZEN is not a show variable, so it cannot
        // stand in for one — same verdict as no survivor at all.
        match survivor {
            Some(surv) if frozen.contains(&surv.var) => {
                determined.push(EquivFold {
                    eliminated: VarId(v as u32),
                    survivor: surv,
                });
            }
            _ => {
                fold_ok = false;
                break;
            }
        }
    }
    if !fold_ok {
        return (bcp.formula, Vec::new());
    }
    (res.formula, determined)
}
