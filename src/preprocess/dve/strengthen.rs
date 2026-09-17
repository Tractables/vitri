//! Clause strengthening (via CaDiCaL), equivalence merging, and related utilities.

use std::time::Instant;

use crate::cnf::VarId;
use crate::cnf::{Clause, CnfFormula, Literal};
use crate::diagnostics::diag;

/// How `merge_equivalences` treats `frozen` (show / counted) variables when an
/// SCC of the binary-implication graph contains one.
///
/// Equivalence-merge substitutes every non-representative member of an SCC with
/// the representative. For the projected count the representative must end up a
/// *counted* variable, or the eliminated show var would be mis-accounted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FrozenEquiv {
    /// Ignore the frozen set: lowest-indexed var is the representative, frozen
    /// vars may be eliminated — whoever needs the weight of an eliminated
    /// frozen var is responsible for folding it into the representative the
    /// fate names. Behaviour is byte-identical to passing an empty frozen set.
    Ignore,
    /// Force the representative of any frozen-touching SCC to be a frozen var,
    /// and eliminate the rest as *determined* (×1). The survivor stays counted;
    /// every eliminated frozen member is functionally fixed by it. The caller
    /// reads the [`DveFate::Equiv`] fates to drop those show vars from the show set
    /// (each contributes ×1, not the ×2 of a genuinely free show var). Sound
    /// only when nothing needs the eliminated var's weight — a path that must
    /// fold weights instead cannot use this variant.
    ForceShowRep,
}

/// Runs the whole DVE pipeline again (not just strengthening, despite the
/// name) on the compact renumbered formula: after the main loop renumbers,
/// the denser primal graph can expose new DVE candidates that weren't
/// simplicial in the original sparse numbering.
pub(crate) fn post_dve_strengthen_with_meter(
    dve: &mut super::types::DveResult,
    frozen: &rustc_hash::FxHashSet<VarId>,
    meter: &mut crate::preprocess::meter::PreprocessMeter,
) {
    if dve.formula.clauses.is_empty() || dve.formula.num_vars == 0 {
        return;
    }

    let clauses_before = dve.formula.clauses.len();
    let vars_before = dve.formula.num_vars;

    // Map the frozen set (DVE-INPUT space, the first-pass input) into the compact
    // first-pass-survivor space the inner pass operates on. `dve.renumbering`
    // currently names first-pass survivors by their DVE-input id (it has not been
    // composed yet), so freeze a survivor p iff its DVE-input id is frozen. This
    // keeps the weighted-DVE FREEZE invariant: vars freeze chose to KEEP in the
    // residual must not be eliminated by this second pass either.
    let inner_frozen: rustc_hash::FxHashSet<VarId> = if frozen.is_empty() {
        rustc_hash::FxHashSet::default()
    } else {
        (0..vars_before as usize)
            .filter(|&p| {
                let din = match &dve.renumbering {
                    Some(r) => r.old_id(VarId::from_idx(p)).idx(),
                    None => p,
                };
                frozen.contains(&VarId::from_idx(din))
            })
            .map(VarId::from_idx)
            .collect()
    };

    // Renumbering often exposes gate patterns that weren't visible in the
    // original sparse space — e.g. gate input/output vars separated by
    // now-eliminated middles.
    let mapping = super::super::gates::detect_gates(&dve.formula);
    let known_defined = mapping.eliminated;

    let inner = super::pipeline::preprocess_dve_with_meter(
        &dve.formula,
        super::pipeline::DveConfig {
            max_rounds: 10,
            time_limit_ms: 2_000,
            keep_original_vars: false,
            known_defined: &known_defined,
            frozen: &inner_frozen,
            frozen_equiv: FrozenEquiv::Ignore,
        },
        meter,
    );
    dve.elapsed_ms = dve.elapsed_ms.saturating_add(inner.elapsed_ms);

    if inner.total_eliminated() == 0 {
        return;
    }

    // Compose the two renumberings: the inner pass names its variables by
    // first-pass-survivor id, this pass names survivors by DVE-input id, and what
    // every consumer below wants is inner id → DVE-input id. A pass that
    // renumbered nothing (`None`) leaves the other one standing on its own.
    let old_renumbering = dve.renumbering.take();
    dve.renumbering = match (old_renumbering.as_ref(), inner.renumbering.as_ref()) {
        (Some(old), Some(inner_r)) => Some(old.compose(inner_r)),
        (Some(_), None) => old_renumbering.clone(),
        (None, inner_r) => inner_r.cloned(),
    };

    // Merge the inner pass's per-variable fates into the DVE-INPUT-space ones.
    // The weighted-DVE correction (`crate::preprocess::weighted_lift`'s
    // `dve_correction` / `dve_eligibility`) reads `dve.fates` indexed by
    // DVE-input id — without this merge it would see only the first pass,
    // missing weight corrections for the inner pass's eliminations.
    // `inner.fates` is indexed by first-pass-survivor id `p`; map p → DVE-input
    // id via `old_renumbering`.
    let to_dve_input = |p: usize| match old_renumbering.as_ref() {
        Some(r) => r.old_id(VarId::from_idx(p)).idx(),
        None => p,
    };
    for (p, &fate) in inner.fates.iter().enumerate() {
        if !fate.eliminated() {
            continue;
        }
        // Map an inner equivalence representative (first-pass-survivor space)
        // back to DVE-input space so the weighted-DVE fold can chase the chain
        // in one consistent space.
        dve.fates[to_dve_input(p)] = match fate {
            super::types::DveFate::Equiv { rep } => super::types::DveFate::Equiv {
                rep: Literal::new(VarId::from_idx(to_dve_input(rep.var.idx())), rep.positive),
            },
            other => other,
        };
    }

    let (inner_defined, inner_equiv, inner_free) =
        (inner.num_defined(), inner.num_equiv(), inner.num_free());

    dve.formula = inner.formula;

    diag!(
        "[post-dve] {} → {} vars, {} → {} clauses ({} defined, {} equiv, {} free)",
        vars_before,
        dve.formula.num_vars,
        clauses_before,
        dve.formula.clauses.len(),
        inner_defined,
        inner_equiv,
        inner_free,
    );
}

/// Equivalence merging step (GPMC: `MergeAdjEquivs`).
///
/// GPMC uses stronger SAT-based pairwise probing; SCC is cheaper and catches
/// most equivalences — the interleaving with CaDiCaL strengthening (which
/// creates new binary clauses) recovers the rest across subsequent rounds.
///
/// The implication graph, its components, the `x ≡ ¬x` check and the
/// substitution are [`crate::preprocess::equivalence`]'s. What is DVE's own is
/// the [`FrozenEquiv`] choice of representative and the `Equiv` fate each
/// merged variable gets. Returns how many variables were merged away.
pub(super) fn merge_equivalences(
    clauses: &mut Vec<Clause>,
    num_vars: usize,
    fates: &mut [super::types::DveFate],
    frozen: &rustc_hash::FxHashSet<VarId>,
    policy: FrozenEquiv,
) -> usize {
    use crate::preprocess::equivalence;

    let sccs = equivalence::implication_sccs(clauses, num_vars);
    let representative = equivalence::scc_representatives(&sccs, num_vars * 2);

    if equivalence::has_equiv_contradiction(&representative, num_vars) {
        clauses.clear();
        clauses.push(Clause::new(vec![]));
        return 0;
    }

    let mut equiv_count = 0usize;
    // rep_map[v] = the literal `v` is equivalent to
    let mut rep_map: Vec<Literal> = (0..num_vars)
        .map(|v| Literal::pos(VarId::from_idx(v)))
        .collect();

    for scc in &sccs {
        if scc.len() <= 1 {
            continue;
        }

        // Per-SCC representative selection under `FrozenEquiv` (see there for
        // the policy semantics).
        let force_show_rep = policy == FrozenEquiv::ForceShowRep
            && !frozen.is_empty()
            && scc
                .iter()
                .any(|&node| frozen.contains(&VarId::from_idx(node / 2)));

        let mut rep_var = u32::MAX;
        let mut rep_positive = true;
        for &node in scc {
            let var = node / 2;
            let positive = node.is_multiple_of(2);
            if fates[var].eliminated() {
                continue;
            }
            if force_show_rep && !frozen.contains(&VarId::from_idx(var)) {
                continue;
            }
            if (var as u32) < rep_var {
                rep_var = var as u32;
                rep_positive = positive;
            }
        }
        if rep_var == u32::MAX {
            continue; // all eliminated (or no eligible show rep under ForceShowRep)
        }

        for &node in scc {
            let var = node / 2;
            let positive = node.is_multiple_of(2);
            if var as u32 == rep_var || fates[var].eliminated() {
                continue;
            }
            let rep_lit = Literal::new(VarId::from_idx(rep_var as usize), positive == rep_positive);
            rep_map[var] = rep_lit;
            fates[var] = super::types::DveFate::Equiv { rep: rep_lit };
            equiv_count += 1;
        }
    }

    if equiv_count == 0 {
        return 0;
    }

    let mut new_clauses: Vec<Clause> = Vec::with_capacity(clauses.len());
    for clause in clauses.iter() {
        if let Some(lits) = equivalence::substitute_clause(clause, &rep_map, None) {
            new_clauses.push(Clause::new(lits));
        }
    }

    *clauses = new_clauses;
    super::elim::dedup_clauses(clauses);

    equiv_count
}

/// Clause strengthening via CaDiCaL (GPMC: `Strengthen`).
///
/// Runs one round of CaDiCaL preprocessing (vivification, failed literal
/// probing, self-subsuming resolution, subsumption), all model-count-preserving.
///
/// Returns true if any clauses were shortened or removed.
///
/// `stage_deadline` is the wall of the DVE pass this call runs inside; the bound
/// derived from it below is what stops the round. `None` is the unbounded round,
/// which is what the tests that compare against it pass.
pub(super) fn strengthen_clauses_with_meter(
    clauses: &mut Vec<Clause>,
    num_vars: usize,
    stage_deadline: Option<Instant>,
    meter: &mut crate::preprocess::meter::PreprocessMeter,
) -> bool {
    if clauses.is_empty() {
        return false;
    }

    // Moves `clauses` into the CaDiCaL input instead of cloning it; the
    // no-change path below restores the originals from `formula.clauses`
    // without a copy.
    let len_before = clauses.len();
    let total_lits_before: usize = clauses.iter().map(|c| c.literals.len()).sum();

    let formula = CnfFormula {
        num_vars: num_vars as u32,
        clauses: std::mem::take(clauses),
    };

    // The bound on the vivification round, and the one place it is derived.
    //
    // The DVE budget does not bound this call. It is polled between rounds — the
    // loop in `pipeline` checks it on entry to each round and breaks when it is
    // spent — so it decides whether a round starts and nothing about how long one
    // runs. CaDiCaL polls no clock of its own either, so with `None` here a
    // single round ran until it was finished, which on some formulas is many
    // times the whole DVE budget.
    //
    // Half of what is left of the stage wall, not all of it: strengthening
    // exists to shorten clauses so the next round has more to eliminate, and a
    // step free to spend the entire window leaves the rounds it strengthens for
    // nothing to spend. An expired wall yields a zero budget, which fires the
    // terminator on its first check.
    //
    // A cut round degrades to the reduction reached so far, never to a wrong one:
    // every appearing variable is frozen, so CaDiCaL may only vivify, subsume and
    // propagate — never eliminate — and its clause database is
    // model-count-equivalent to the input at every point. This is the same
    // partial-output contract the pipeline's CaDiCaL simplify stage already runs
    // under with a real deadline.
    let deadline = stage_deadline.map(|d| {
        let now = Instant::now();
        now + d.saturating_duration_since(now) / 2
    });
    let (strengthened, _forced) =
        super::super::cadical::preprocess_cadical_with_meter(&formula, 1, deadline, meter);

    if strengthened.clauses.len() == len_before {
        let total_lits_after: usize = strengthened.clauses.iter().map(|c| c.literals.len()).sum();
        if total_lits_before == total_lits_after {
            *clauses = formula.clauses;
            return false;
        }
    }

    *clauses = strengthened.clauses;
    true
}
