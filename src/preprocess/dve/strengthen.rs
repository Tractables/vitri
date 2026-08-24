//! Clause strengthening (via CaDiCaL), equivalence merging, Tarjan SCC, and related utilities.

use std::time::Instant;

use super::elim::ElimYield;
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
pub(crate) fn post_dve_strengthen(
    dve: &mut super::types::DveResult,
    frozen: &rustc_hash::FxHashSet<VarId>,
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
                    Some(r) => r.old_id(VarId(p as u32)).idx(),
                    None => p,
                };
                frozen.contains(&VarId(din as u32))
            })
            .map(|p| VarId(p as u32))
            .collect()
    };

    // Renumbering often exposes gate patterns that weren't visible in the
    // original sparse space — e.g. gate input/output vars separated by
    // now-eliminated middles.
    let mapping = super::super::gates::detect_gates(&dve.formula);
    let known_defined = mapping.eliminated;

    let inner = super::pipeline::preprocess_dve(
        &dve.formula,
        10,
        2_000,
        false,
        &known_defined,
        &inner_frozen,
        FrozenEquiv::Ignore,
    );

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
    let to_dve_input = |p: u32| match old_renumbering.as_ref() {
        Some(r) => r.old_id(VarId(p)).0,
        None => p,
    };
    for (p, &fate) in inner.fates.iter().enumerate() {
        if !fate.eliminated() {
            continue;
        }
        // Map an inner equivalence representative (first-pass-survivor space)
        // back to DVE-input space so the weighted-DVE fold can chase the chain
        // in one consistent space.
        dve.fates[to_dve_input(p as u32) as usize] = match fate {
            super::types::DveFate::Equiv { rep } => super::types::DveFate::Equiv {
                rep: Literal::new(VarId(to_dve_input(rep.var.0)), rep.positive),
            },
            other => other,
        };
    }

    let (inner_defined, inner_equiv, inner_free) =
        (inner.num_defined(), inner.num_equiv(), inner.num_free());

    dve.formula = inner.formula;
    // These reference the post-DVE var IDs, not DVE-input space like the
    // arrays above — fine, since Counter mode doesn't use definition_clauses
    // for reintroduction.
    dve.definition_clauses.extend(inner.definition_clauses);

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
/// Each entry of the yield's `definitions` is the pair of binary clauses
/// encoding one equivalence constraint (for later re-introduction in BVE mode).
/// The substitution bookkeeping DVE carries across rounds, indexed by variable:
/// what became of each variable, and the signed representative each merged one
/// folds onto.
pub(super) struct EquivState<'a> {
    pub(super) fates: &'a mut [super::types::DveFate],
    pub(super) representative: &'a mut [i32],
}

pub(super) fn merge_equivalences(
    clauses: &mut Vec<Clause>,
    num_vars: usize,
    state: &mut EquivState<'_>,
    frozen: &rustc_hash::FxHashSet<VarId>,
    policy: FrozenEquiv,
) -> ElimYield {
    let num_lits = num_vars * 2;
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); num_lits];

    for clause in clauses.iter() {
        if clause.literals.len() == 2 {
            let (l0, l1) = (&clause.literals[0], &clause.literals[1]);
            adj[lit_to_idx(l0.var.0 as usize, !l0.positive)]
                .push(lit_to_idx(l1.var.0 as usize, l1.positive) as u32);
            adj[lit_to_idx(l1.var.0 as usize, !l1.positive)]
                .push(lit_to_idx(l0.var.0 as usize, l0.positive) as u32);
        }
    }

    let sccs = tarjan_scc(&adj, num_lits);

    let mut scc_id = vec![0u32; num_lits];
    for (id, scc) in sccs.iter().enumerate() {
        for &node in scc {
            scc_id[node as usize] = id as u32;
        }
    }

    let mut equiv_count = 0usize;
    let mut equiv_def_clauses: Vec<Vec<Clause>> = Vec::new();
    // rep_map[v] = the literal `v` is equivalent to
    let mut rep_map: Vec<Literal> = (0..num_vars)
        .map(|v| Literal::pos(VarId(v as u32)))
        .collect();

    for scc in &sccs {
        if scc.len() <= 1 {
            continue;
        }

        // Check for contradiction: x and ¬x in same SCC → UNSAT
        for &node in scc {
            let var = node as usize / 2;
            let pos = (node as usize).is_multiple_of(2);
            let neg_idx = lit_to_idx(var, !pos);
            if scc_id[node as usize] == scc_id[neg_idx] {
                // UNSAT — add empty clause and return
                clauses.clear();
                clauses.push(Clause::new(vec![]));
                return ElimYield {
                    eliminated: 0,
                    definitions: Vec::new(),
                };
            }
        }

        // Per-SCC representative selection under `FrozenEquiv` (see there for
        // the policy semantics). The contradiction check above runs
        // regardless of policy, so x ≡ ¬x UNSAT is always detected.
        let force_show_rep = policy == FrozenEquiv::ForceShowRep
            && !frozen.is_empty()
            && scc
                .iter()
                .any(|&node| frozen.contains(&VarId((node as usize / 2) as u32)));

        let mut rep_var = u32::MAX;
        let mut rep_positive = true;
        for &node in scc {
            let var = node as usize / 2;
            let positive = (node as usize).is_multiple_of(2);
            if state.fates[var].eliminated() {
                continue;
            }
            if force_show_rep && !frozen.contains(&VarId(var as u32)) {
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
            let var = node as usize / 2;
            let positive = (node as usize).is_multiple_of(2);
            if var as u32 == rep_var || state.fates[var].eliminated() {
                continue;
            }
            let same_pol = positive == rep_positive;
            let rep_lit = Literal::new(VarId(rep_var), same_pol);
            rep_map[var] = rep_lit;
            state.fates[var] = super::types::DveFate::Equiv { rep: rep_lit };
            state.representative[var] = if same_pol {
                rep_var as i32
            } else {
                -(rep_var as i32)
            };
            let v_id = VarId(var as u32);
            let rep_id = VarId(rep_var);
            let def = if same_pol {
                vec![
                    Clause::new(vec![Literal::pos(v_id), Literal::neg(rep_id)]),
                    Clause::new(vec![Literal::neg(v_id), Literal::pos(rep_id)]),
                ]
            } else {
                vec![
                    Clause::new(vec![Literal::pos(v_id), Literal::pos(rep_id)]),
                    Clause::new(vec![Literal::neg(v_id), Literal::neg(rep_id)]),
                ]
            };
            equiv_def_clauses.push(def);
            equiv_count += 1;
        }
    }

    if equiv_count == 0 {
        return ElimYield {
            eliminated: 0,
            definitions: Vec::new(),
        };
    }

    let mut new_clauses: Vec<Clause> = Vec::with_capacity(clauses.len());
    for clause in clauses.iter() {
        let mut new_lits: Vec<Literal> = clause
            .literals
            .iter()
            .map(|lit| {
                let rep = rep_map[lit.var.0 as usize];
                if lit.positive { rep } else { rep.negated() }
            })
            .collect();
        new_lits.sort_by_key(|l| (l.var, !l.positive));
        new_lits.dedup();

        let is_tautology = new_lits.windows(2).any(|w| w[0].var == w[1].var);
        if !is_tautology {
            new_clauses.push(Clause::new(new_lits));
        }
    }

    *clauses = new_clauses;
    super::elim::dedup_clauses(clauses);

    ElimYield {
        eliminated: equiv_count,
        definitions: equiv_def_clauses,
    }
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
pub(super) fn strengthen_clauses(
    clauses: &mut Vec<Clause>,
    num_vars: usize,
    stage_deadline: Option<Instant>,
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
    let (strengthened, _forced) = super::super::cadical::preprocess_cadical(&formula, 1, deadline);

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

/// Delegates to the shared iterative implementation in
/// `crate::preprocess::tarjan`.
fn tarjan_scc(adj: &[Vec<u32>], n: usize) -> Vec<Vec<u32>> {
    let adj_usize: Vec<Vec<usize>> = adj[..n]
        .iter()
        .map(|neighbors| neighbors.iter().map(|&w| w as usize).collect())
        .collect();

    let groups_usize = super::super::tarjan::tarjan_scc_groups(n, &adj_usize);

    groups_usize
        .into_iter()
        .map(|group| group.into_iter().map(|v| v as u32).collect())
        .collect()
}

/// Map a literal to its index in the implication graph. The graph's node
/// numbering is the crate's per-literal table index, so it comes from the one
/// place that encoding lives.
use crate::cnf::occ::literal_index as lit_to_idx;
