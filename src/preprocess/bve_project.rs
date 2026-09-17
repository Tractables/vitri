//! CNF-level bounded variable elimination (resolution) for PROJECTED variables.
//!
//! For projected model counting (`pmc`), variables outside the show
//! (KEEP) set are existentially quantified away. At the clause level, ∃v.F is
//! exactly the resolution of F on v (Davis–Putnam / variable elimination):
//! replace all clauses containing v with the set of non-tautological resolvents
//! on v. Because projected variables are quantified — not counted — there is NO
//! model-count bookkeeping here (unlike `dve`, which must track the multiplicity
//! of eliminated counted vars). We simply drop v.
//!
//! We apply this as a BOUNDED pass (Eén–Biere / SatELite style): a projected
//! var is eliminated only when doing so does not grow the clause count
//! (R ≤ K). Vars that would grow the formula are left in place for the
//! diagram-level projection to handle.
//!
//! SOUNDNESS INVARIANTS:
//!   (1) We NEVER eliminate a SHOW variable — every elimination candidate is
//!       checked against the mask first.
//!   (2) Tautological resolvents (containing both x and ¬x) are dropped — they
//!       are valid (always true) and contribute nothing to the conjunction.
//!   (3) Variable ids are PRESERVED: we never renumber. `num_vars` is unchanged
//!       and surviving literals keep their original `VarId`. Callers rely on
//!       stable ids.
//!   (4) The result computes ∃(eliminated vars).F. Hence the set of models of
//!       the result restricted to the remaining variables equals the projection
//!       of models(F) onto those remaining variables — exactly what the
//!       projected-count caller needs (eliminated vars are show-irrelevant by
//!       construction: only unshown ones are eliminated).

use std::collections::HashSet;

use crate::cnf::ShowMask;
use crate::cnf::VarId;
use crate::cnf::occ;
use crate::cnf::{Clause, CnfFormula, Literal, normalize_literals};

/// Existentially eliminate unshown variables by bounded clause-level
/// resolution: a projected var goes when its resolvent count `R` is at most the
/// number of clauses `K` it occurs in, strict SatELite-style no-growth. A
/// pure-literal projected var always goes, since dropping it only shrinks the
/// formula.
///
/// `show` is the set the answer is taken over; everything outside it may be
/// eliminated. Returns a new `CnfFormula` with the SAME `num_vars` (no
/// renumbering).
pub(crate) fn bve_project(formula: &CnfFormula, show: &ShowMask) -> CnfFormula {
    let num_vars = formula.num_vars;
    // A variable this pass may eliminate is exactly one the answer is not taken
    // over.
    let eliminable = |v: u32| !show.is_show(VarId(v));

    // Mutable working set: each clause is a sorted literal vec; `live[i]` flags
    // whether clause i is still present.
    let mut clauses: Vec<Vec<Literal>> = formula
        .clauses
        .iter()
        .filter_map(|c| normalize_literals(c.literals.clone()))
        .collect();
    let mut live: Vec<bool> = vec![true; clauses.len()];

    // Occurrence lists: occ_pos[v] / occ_neg[v] = indices of live clauses
    // containing +v / −v. Built over the normalized literal vectors above, so
    // the indices address `clauses` and `live` directly.
    let (mut occ_pos, mut occ_neg) =
        occ::occurrence_lists_of(clauses.iter().map(|c| c.as_slice()), num_vars as usize);

    // Bounded resolution VE, WORKLIST-driven: a projected var is (re)considered
    // only when its occurrence lists may have changed — seeded with every
    // appearing projected var, then any projected neighbour touched by an
    // elimination is re-queued. Resolvent dedup uses a HashSet, and enumeration
    // aborts the moment the unique count exceeds the R ≤ K budget, so
    // a high-degree var that cannot be eliminated is abandoned in O(budget), not
    // O(|pos|·|neg|).
    //
    // SOUNDNESS / equivalence: the worklist changes the *order* of elimination,
    // and bounded VE is not order-confluent (a different order can eliminate a
    // different SET of vars under the R ≤ K bound, leaving a different residual
    // clause set). That is fine — every step is exactly ∃v, so the projected
    // count pc(show) is invariant under any elimination order.
    let mut queued = vec![false; num_vars as usize];
    let mut queue: std::collections::VecDeque<u32> = std::collections::VecDeque::new();
    for v in 0..num_vars {
        if eliminable(v) && (!occ_pos[v as usize].is_empty() || !occ_neg[v as usize].is_empty()) {
            queue.push_back(v);
            queued[v as usize] = true;
        }
    }

    while let Some(v) = queue.pop_front() {
        let vi = v as usize;
        queued[vi] = false;
        if !eliminable(v) {
            // INVARIANT (1): never eliminate a show var.
            continue;
        }
        let vid = VarId(v);

        // Refresh occurrence lists (drop indices killed by earlier elims).
        purge_dead(&mut occ_pos[vi], &live);
        purge_dead(&mut occ_neg[vi], &live);

        if occ_pos[vi].is_empty() && occ_neg[vi].is_empty() {
            continue;
        }

        // Pure-literal projected var: delete every clause containing it.
        // Re-queue projected neighbours of the killed clauses — their
        // occurrence counts just dropped.
        if occ_pos[vi].is_empty() || occ_neg[vi].is_empty() {
            let to_kill: Vec<usize> = occ_pos[vi]
                .iter()
                .chain(occ_neg[vi].iter())
                .copied()
                .collect();
            occ_pos[vi].clear();
            occ_neg[vi].clear();
            kill_clauses(
                to_kill,
                v,
                &clauses,
                &mut live,
                &mut queue,
                &mut queued,
                show,
            );
            continue;
        }

        // General case: resolve, unless that costs more clauses than it saves.
        let pos: Vec<usize> = occ_pos[vi].clone();
        let neg: Vec<usize> = occ_neg[vi].clone();
        let Some(resolvents) = enumerate_resolvents(&clauses, &pos, &neg, vid) else {
            continue; // Leave v for diagram-level projection.
        };

        // Eliminate v: mark clauses containing it dead, append the resolvents
        // as fresh live clauses, and re-queue every projected neighbour touched.
        occ_pos[vi].clear();
        occ_neg[vi].clear();
        kill_clauses(
            pos.iter().chain(neg.iter()).copied(),
            v,
            &clauses,
            &mut live,
            &mut queue,
            &mut queued,
            show,
        );

        for lits in resolvents {
            let idx = clauses.len();
            for l in &lits {
                if l.positive {
                    occ_pos[l.var.idx()].push(idx);
                } else {
                    occ_neg[l.var.idx()].push(idx);
                }
                requeue(&mut queue, &mut queued, show, l.var.0);
            }
            clauses.push(lits);
            live.push(true);
        }
    }

    // Rebuild from the live clauses. `Clause::new` enforces the per-var
    // uniqueness precondition; our resolvents are already sorted/deduped and
    // tautology-free, and so are the surviving originals.
    // INVARIANT (3): num_vars unchanged, ids preserved.
    let out_clauses: Vec<Clause> = clauses
        .into_iter()
        .zip(live)
        .filter_map(|(lits, alive)| if alive { Some(Clause::new(lits)) } else { None })
        .collect();

    CnfFormula {
        num_vars,
        clauses: out_clauses,
    }
}

/// Drop from `occ` the clause indices an earlier elimination killed.
fn purge_dead(occ: &mut Vec<usize>, live: &[bool]) {
    occ.retain(|&i| live[i]);
}

/// Put `w` back in the queue, if this pass may eliminate it and it is not
/// already waiting: whatever just happened changed its occurrence lists, so
/// a variable that could not be eliminated before may be eliminable now.
fn requeue(
    queue: &mut std::collections::VecDeque<u32>,
    queued: &mut [bool],
    show: &ShowMask,
    w: u32,
) {
    if !show.is_show(VarId(w)) && !queued[w as usize] {
        queue.push_back(w);
        queued[w as usize] = true;
    }
}

/// Mark every clause in `to_kill` dead and re-queue the projected variables
/// that occurred alongside `v` in them. Both ways a variable leaves — pure
/// literal and resolution — retire its clauses exactly this way.
fn kill_clauses(
    to_kill: impl IntoIterator<Item = usize>,
    v: u32,
    clauses: &[Vec<Literal>],
    live: &mut [bool],
    queue: &mut std::collections::VecDeque<u32>,
    queued: &mut [bool],
    show: &ShowMask,
) {
    for i in to_kill {
        if !live[i] {
            continue;
        }
        live[i] = false;
        for l in &clauses[i] {
            if l.var.0 != v {
                requeue(queue, queued, show, l.var.0);
            }
        }
    }
}

/// Resolve Cp (containing +v) and Cn (containing −v) on v: union of their
/// literals minus the v / ¬v literals. Returns None if the resolvent is
/// tautological (some other var appears with both polarities). Result is
/// sorted/deduped.
///
/// NOT unified with `dve::elim`'s `elim_vars` resolvent loop (same resolvent
/// value) — DELIBERATELY SEPARATE. This pass is pure ∃-projection: no count
/// bookkeeping, projected vars are freely dropped, show vars never touched.
/// `elim_vars` is count-preserving DVE and must special-case frozen/forced/
/// pure-literal vars to keep the model count exact. The value coincides; the
/// surrounding contracts do not, so two scoped copies are safer than a shared
/// kernel carrying both contracts.
fn resolve_on(cp: &[Literal], cn: &[Literal], v: VarId) -> Option<Vec<Literal>> {
    let out: Vec<Literal> = cp
        .iter()
        .chain(cn.iter())
        .copied()
        .filter(|l| l.var != v)
        .collect();
    normalize_literals(out)
}

/// Every unique non-tautological resolvent of `v`, or `None` once their
/// count passes the keep-bound R ≤ K (K = live clauses containing `v`), at
/// which point eliminating `v` would grow the formula. Aborting on the bound
/// rather than after enumerating keeps a high-degree variable O(budget).
fn enumerate_resolvents(
    clauses: &[Vec<Literal>],
    pos: &[usize],
    neg: &[usize],
    v: VarId,
) -> Option<Vec<Vec<Literal>>> {
    let k = pos.len() + neg.len();
    let mut seen: HashSet<Vec<Literal>> = HashSet::new();
    let mut resolvents: Vec<Vec<Literal>> = Vec::new();
    for &ip in pos {
        for &in_ in neg {
            if let Some(r) = resolve_on(&clauses[ip], &clauses[in_], v) {
                // INVARIANT (2): only non-tautological resolvents reach here.
                if seen.insert(r.clone()) {
                    resolvents.push(r);
                    if resolvents.len() > k {
                        return None;
                    }
                }
            }
        }
    }
    Some(resolvents)
}
