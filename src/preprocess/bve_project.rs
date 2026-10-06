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
//!
//! DEADLINE: the pass stops eliminating once its deadline passes and returns
//! the live clauses as they stand. By (4) every elimination it completed is
//! exactly ∃v, so a pass cut short has the same projected count as one run to
//! its fixpoint; it has only eliminated fewer variables. The clock is read
//! between eliminations and every [`POLL_PAIRS`] resolvent pairs inside one —
//! both points at which the clause store, the live flags and the occurrence
//! lists agree, so stopping leaves nothing half-applied.

use std::collections::HashSet;
use std::time::Instant;

use crate::cnf::ShowMask;
use crate::cnf::VarId;
use crate::cnf::occ;
use crate::cnf::{Clause, CnfFormula, Literal, normalize_literals, resolve_sorted};

/// Existentially eliminate unshown variables by bounded clause-level
/// resolution: a projected var goes when its resolvent count `R` is at most the
/// number of clauses `K` it occurs in, strict SatELite-style no-growth. A
/// pure-literal projected var always goes, since dropping it only shrinks the
/// formula.
///
/// `show` is the set the answer is taken over; everything outside it may be
/// eliminated. Returns a new `CnfFormula` with the SAME `num_vars` (no
/// renumbering).
///
/// `deadline` bounds the pass: once it has passed, no further variable is
/// eliminated and the live clauses are returned as they stand. Every completed
/// elimination is exactly ∃v, so the projected count over `show` is the
/// input's wherever the deadline lands. `None` runs the pass to its fixpoint,
/// and the clock is never read.
pub(crate) fn bve_project(
    formula: &CnfFormula,
    show: &ShowMask,
    deadline: Option<Instant>,
) -> CnfFormula {
    let num_vars = formula.num_vars();
    // A variable this pass may eliminate is exactly one the answer is not taken
    // over.
    let eliminable = |v: u32| !show.is_show(VarId::from_idx(v as usize));

    // Mutable working set: each clause is a sorted literal vec; `live[i]` flags
    // whether clause i is still present.
    let mut clauses: Vec<Vec<Literal>> = formula
        .clauses()
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
    // aborts the moment the unique count exceeds the R ≤ K budget, so a
    // high-degree var that cannot be eliminated is abandoned after K + 1 unique
    // resolvents. Pairs whose resolvent is a tautology or a duplicate count
    // toward nothing, so they can still make one var's enumeration visit all
    // |pos|·|neg| pairs — the reason the deadline is also read inside it.
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
        // Between eliminations: nothing is half-applied.
        if crate::budget::expired(deadline) {
            break;
        }
        let vi = v as usize;
        queued[vi] = false;
        if !eliminable(v) {
            // INVARIANT (1): never eliminate a show var.
            continue;
        }
        let vid = VarId::from_idx(v as usize);

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
        let resolvents = match enumerate_resolvents(&clauses, &pos, &neg, vid, deadline) {
            Resolvents::Fit(resolvents) => resolvents,
            // Leave v for diagram-level projection.
            Resolvents::Grow => continue,
            // Enumeration only reads the clauses: nothing is half-applied.
            Resolvents::OutOfTime => break,
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
                requeue(&mut queue, &mut queued, show, l.var.idx() as u32);
            }
            clauses.push(lits);
            live.push(true);
        }
    }

    // Rebuild from the live clauses, whether the worklist ran dry or the
    // deadline stopped it. `Clause::new` enforces the per-var uniqueness
    // precondition; our resolvents are already sorted/deduped and
    // tautology-free, and so are the surviving originals.
    // INVARIANT (3): num_vars unchanged, ids preserved.
    let out_clauses: Vec<Clause> = clauses
        .into_iter()
        .zip(live)
        .filter_map(|(lits, alive)| if alive { Some(Clause::new(lits)) } else { None })
        .collect();

    CnfFormula::from_parts(num_vars, out_clauses)
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
    if !show.is_show(VarId::from_idx(w as usize)) && !queued[w as usize] {
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
            let w = l.var.idx() as u32;
            if w != v {
                requeue(queue, queued, show, w);
            }
        }
    }
}

/// How many resolvent pairs [`enumerate_resolvents`] visits between reads of
/// the clock. A read at every pair would rival the cost of the resolution
/// itself; at this spacing the read is noise next to the pairs, and the
/// overshoot past a deadline is at most one batch. A power of two, so the test
/// is a mask.
const POLL_PAIRS: usize = 1 << 10;

/// What enumerating one variable's resolvents found.
enum Resolvents {
    /// Every unique non-tautological resolvent, and no more of them than
    /// clauses containing the variable.
    Fit(Vec<Vec<Literal>>),
    /// More unique resolvents than clauses containing the variable:
    /// eliminating it would grow the formula.
    Grow,
    /// The deadline passed before the enumeration finished.
    OutOfTime,
}

/// Every unique non-tautological resolvent of `v`, unless their count passes
/// the keep-bound R ≤ K (K = live clauses containing `v`), at which point
/// eliminating `v` would grow the formula. Aborting on the bound stops a
/// variable that cannot go after K + 1 unique resolvents; a pair whose
/// resolvent is a tautology or a duplicate still costs a visit, so the clock is
/// read every [`POLL_PAIRS`] pairs as well.
///
/// The resolvent is [`resolve_sorted`], which relies on every clause here being
/// sorted by variable: the originals are normalized on the way in and the
/// resolvents come out of it in that form. The loop around it is NOT unified
/// with `dve::elim`'s `elim_vars` — DELIBERATELY SEPARATE. This pass is pure
/// ∃-projection: no count bookkeeping, projected vars are freely dropped, show
/// vars never touched. `elim_vars` is count-preserving DVE and must
/// special-case frozen/forced/pure-literal vars to keep the model count exact.
/// The resolvent coincides; the surrounding contracts do not, so two scoped
/// loops are safer than one carrying both contracts.
fn enumerate_resolvents(
    clauses: &[Vec<Literal>],
    pos: &[usize],
    neg: &[usize],
    v: VarId,
    deadline: Option<Instant>,
) -> Resolvents {
    let k = pos.len() + neg.len();
    let mut seen: HashSet<Vec<Literal>> = HashSet::new();
    let mut resolvents: Vec<Vec<Literal>> = Vec::new();
    let mut pairs: usize = 0;
    for &ip in pos {
        for &in_ in neg {
            pairs += 1;
            if pairs & (POLL_PAIRS - 1) == 0 && crate::budget::expired(deadline) {
                return Resolvents::OutOfTime;
            }
            if let Some(r) = resolve_sorted(&clauses[ip], &clauses[in_], v) {
                // INVARIANT (2): only non-tautological resolvents reach here.
                if seen.insert(r.clone()) {
                    resolvents.push(r);
                    if resolvents.len() > k {
                        return Resolvents::Grow;
                    }
                }
            }
        }
    }
    Resolvents::Fit(resolvents)
}
