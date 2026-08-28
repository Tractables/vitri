//! Variable elimination via resolution, BCP, clause utilities, and the per-round DVE step.

use crate::cnf::VarId;
use crate::cnf::{Clause, Literal};

use crate::cnf::occ;

use super::definability::{
    MAX_DUAL_CNF_CLAUSES, PRIMAL_GRAPH_MAX_VARS, PrimalGraph, is_ve_candidate,
    pick_def_vars_with_meter,
};
use super::types::DveFate;

/// One variable's view of the clause set, taken by [`split_on`]: the clauses
/// that mention it, stripped of it and split by the polarity they carried; the
/// clauses that do not; and unmodified copies of the ones that do, which BVE
/// re-introduction needs.
///
/// A clause carrying both polarities of the variable is counted positive — it
/// is a tautology in that variable, so which side it lands on cannot change a
/// resolvent.
struct PolaritySplit {
    pos: Vec<Clause>,
    neg: Vec<Clause>,
    remaining: Vec<Clause>,
    originals: Vec<Clause>,
}

/// Drain `clauses` into the split on `v`.
fn split_on(clauses: &mut Vec<Clause>, v: u32) -> PolaritySplit {
    let mut split = PolaritySplit {
        pos: Vec::new(),
        neg: Vec::new(),
        remaining: Vec::new(),
        originals: Vec::new(),
    };
    for clause in clauses.drain(..) {
        let mut found_pos = false;
        let mut found_neg = false;
        for lit in &clause.literals {
            if lit.var.0 == v {
                if lit.positive {
                    found_pos = true;
                } else {
                    found_neg = true;
                }
            }
        }
        if found_pos || found_neg {
            split.originals.push(clause.clone());
            let stripped: Vec<Literal> = clause
                .literals
                .iter()
                .filter(|l| l.var.0 != v)
                .copied()
                .collect();
            let target = if found_pos {
                &mut split.pos
            } else {
                &mut split.neg
            };
            target.push(Clause::new(stripped));
        } else {
            split.remaining.push(clause);
        }
    }
    split
}

/// Eliminate variables via resolution (GPMC's `ElimVars`).
///
/// The blowup guard (`remaining.len() + resolvents.len() > max_clauses`) is
/// weaker than a strict resolvent-bounded check: after prior vars in the batch
/// have reduced the clause count, a nominally non-resolvent-bounded var
/// (pos*neg greater than pos+neg) may still fit under max_clauses and be
/// eliminated.
///
/// Returns `(eliminated_var_ids, forced_unit_literals, definition_clauses_per_var)`.
pub(super) fn elim_vars(
    clauses: &mut Vec<Clause>,
    vars_to_elim: &[u32],
    max_clauses: usize,
    frozen: &rustc_hash::FxHashSet<VarId>,
) -> (Vec<u32>, Vec<Literal>, Vec<Vec<Clause>>) {
    let mut eliminated_ids: Vec<u32> = Vec::new();
    let mut forced_lits = Vec::new();
    let mut all_def_clauses: Vec<Vec<Clause>> = Vec::new();

    for &v in vars_to_elim {
        if clauses.len() > max_clauses {
            break;
        }

        let PolaritySplit {
            pos: pos_clauses,
            neg: neg_clauses,
            mut remaining,
            originals: original_clauses_for_v,
        } = split_on(clauses, v);

        // If v has no remaining clauses (earlier resolutions in this batch removed
        // them all — e.g., two defined vars in the same XOR cluster), skip it here
        // so the end-of-round free-var scan credits its ×2 factor.
        if pos_clauses.is_empty() && neg_clauses.is_empty() {
            *clauses = remaining;
            continue;
        }

        // NOT unified with `preprocess::bve_project`'s `resolve_on` (near-identical
        // resolvent value) — DELIBERATELY SEPARATE, on two axes. The CONTRACTS differ
        // on what decides model counts: this kernel is COUNT-PRESERVING DVE (a unit
        // resolvent that forces a FROZEN show/projected var is kept as a clause, not
        // propagated — see the `frozen` branch below; a pure-literal defined var is
        // restored rather than dropped; it emits `forced_lits` + `def_clauses` for the
        // caller's ×N bookkeeping). `bve_project` is pure ∃-projection with NO count
        // bookkeeping (it freely drops projected vars and never touches show vars).
        // The ALGORITHMS differ too, so neither body is a drop-in for the other: the
        // loop below is a two-pointer merge of two clauses ALREADY sorted by variable,
        // while `resolve_on` concatenates, sorts and dedups and reads a tautology off
        // adjacency — it accepts unsorted input that this one would silently mis-merge.
        let mut resolvents: Vec<Clause> = Vec::new();
        let mut abort = false;

        for c1 in &pos_clauses {
            for c2 in &neg_clauses {
                let mut merged: Vec<Literal> = Vec::new();
                let mut is_tautology = false;
                let mut i = 0;
                let mut j = 0;

                while i < c1.literals.len() && j < c2.literals.len() {
                    let l1 = &c1.literals[i];
                    let l2 = &c2.literals[j];

                    if l1.var < l2.var {
                        merged.push(*l1);
                        i += 1;
                    } else if l1.var > l2.var {
                        merged.push(*l2);
                        j += 1;
                    } else {
                        if l1.positive == l2.positive {
                            merged.push(*l1);
                            i += 1;
                            j += 1;
                        } else {
                            is_tautology = true;
                            break;
                        }
                    }
                }

                if !is_tautology {
                    merged.extend_from_slice(&c1.literals[i..]);
                    merged.extend_from_slice(&c2.literals[j..]);

                    if merged.len() <= 1 {
                        if let Some(&lit) = merged.first() {
                            // SOUNDNESS (projected counting): if the unit forces a
                            // FROZEN (show/projected) variable, do NOT propagate it
                            // away — propagation deletes every clause mentioning the
                            // var, so the show var vanishes from the residual and the
                            // driver mis-counts it as free (×2) instead of forced (×1).
                            // Keep the derived unit clause so the show var stays
                            // constrained. This preserves ∃H.F exactly.
                            // See `dve_frozen_unit_resolvent_preserves_pmc`.
                            if frozen.contains(&lit.var) {
                                resolvents.push(Clause::new(merged));
                            } else {
                                forced_lits.push(lit);
                            }
                        } else {
                            // Shouldn't happen for defined vars.
                            resolvents.push(Clause::new(merged));
                        }
                    } else {
                        resolvents.push(Clause::new(merged));
                    }
                }

                if remaining.len() + resolvents.len() > max_clauses {
                    abort = true;
                    break;
                }
            }
            if abort {
                break;
            }
        }

        if abort {
            // Clause blowup: restore v's literal and abort. Remaining vars are
            // retried in the next DVE round or aggressive cascade iteration.
            for mut c in pos_clauses {
                c.literals.push(Literal::pos(VarId(v)));
                c.literals.sort_by_key(|l| l.var);
                remaining.push(c);
            }
            for mut c in neg_clauses {
                c.literals.push(Literal::neg(VarId(v)));
                c.literals.sort_by_key(|l| l.var);
                remaining.push(c);
            }
            *clauses = remaining;
            break;
        }

        // Pure literal in residual: only one polarity remains (originally,
        // or after earlier in-batch resolutions tautologized the other).
        // Forcing v is sound for SAT but UNSOUND for #SAT — dropping the
        // (v ∨ ...) clauses loses the models where v=0 satisfies them via
        // the other literals. Restore the original v-clauses and skip this
        // var; it remains in the residual formula and contributes correctly.
        // See regression test `dve_pure_literal_on_defined_var_preserves_mc`.
        if pos_clauses.is_empty() || neg_clauses.is_empty() {
            remaining.extend(original_clauses_for_v);
            *clauses = remaining;
            continue;
        }

        remaining.extend(resolvents);
        *clauses = remaining;
        eliminated_ids.push(v);
        all_def_clauses.push(original_clauses_for_v);
    }

    (eliminated_ids, forced_lits, all_def_clauses)
}

/// What one elimination step removed: how many variables went, and the clauses
/// that define each of them, in elimination order — one group per variable, so
/// the two are read together or not at all.
pub(super) struct ElimYield {
    pub(super) eliminated: usize,
    pub(super) definitions: Vec<Vec<Clause>>,
}

/// Snapshot of a single DVE round, used to decide whether to terminate the loop.
pub(super) struct RoundStats {
    pub(super) round: usize,
    pub(super) dve_elim: usize,
    pub(super) equiv_elim: usize,
    /// `dve_elim` count captured on the first round (`s.round == 0`) and
    /// then held fixed as the reference for the diminishing-returns check.
    /// Field name is 1-indexed ("round 1") matching the surrounding docs;
    /// the `s.round` counter itself is 0-indexed.
    pub(super) round1_dve_elim: usize,
    pub(super) vars_before: usize,
    pub(super) vars_after: usize,
    pub(super) clauses_before: usize,
    pub(super) clauses_after: usize,
    pub(super) orig_clause_count: usize,
    pub(super) progress: bool,
}

pub(super) fn should_terminate_dve(s: &RoundStats) -> bool {
    if s.round == 0 && s.dve_elim > 0 && s.equiv_elim == 0 {
        let elim_rate = s.dve_elim as f64 / s.vars_before.max(1) as f64;
        if elim_rate < 0.02 {
            return true;
        }
    }
    if !s.progress {
        return true;
    }

    // `dve_round`'s resolution guard already refuses any single elimination
    // that grows the clause count; this only catches drift accumulated across
    // rounds.
    if s.clauses_after > (s.orig_clause_count as f64 * 1.1) as usize {
        return true;
    }

    // Strengthen-only rounds leave the counts unchanged even though progress
    // was made.
    if s.vars_after == s.vars_before && s.clauses_after == s.clauses_before {
        return true;
    }

    // From round 3 (`s.round >= 2`, 0-indexed) on, stop once elimination drops
    // below 5% of round 1's count.
    let dim_threshold = (s.round1_dve_elim / 20).max(3);
    if s.dve_elim < dim_threshold && s.equiv_elim == 0 && s.round >= 2 {
        return true;
    }

    false
}

pub(super) fn count_active_vars(clauses: &[Clause]) -> usize {
    clauses
        .iter()
        .flat_map(|c| c.literals.iter().map(|l| l.var.0))
        .collect::<std::collections::HashSet<_>>()
        .len()
}

/// Prerequisite for `elim_vars`'s merge-based resolution.
pub(super) fn sort_clause_literals(clauses: &mut [Clause]) {
    for clause in clauses.iter_mut() {
        clause.literals.sort_by_key(|l| l.var);
    }
}

/// Length-first sort order is what makes `dedup()` catch every duplicate.
pub(super) fn dedup_clauses(clauses: &mut Vec<Clause>) {
    for clause in clauses.iter_mut() {
        clause.literals.sort_by_key(|l| (l.var, !l.positive));
        clause.literals.dedup();
    }
    clauses.sort_by(|a, b| {
        a.literals.len().cmp(&b.literals.len()).then_with(|| {
            a.literals
                .iter()
                .zip(b.literals.iter())
                .map(|(la, lb)| la.var.cmp(&lb.var).then(la.positive.cmp(&lb.positive)))
                .find(|o| !o.is_eq())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    clauses.dedup();
}

/// NOT unified with `preprocess::unit_propagation::propagate` — DELIBERATELY a
/// SEPARATE BCP flavor. This one is COUNT-PRESERVING: a forced FROZEN
/// (show/projected) literal is never propagated (its unit clause is retained so
/// the show var still counts ×1, not ×2 — see the `frozen` filter). It is
/// seed-driven (takes an already-forced list from resolution), mutates in place,
/// and signals UNSAT by leaving an empty clause and returning early.
/// `propagate` is equivalence-only (no frozen concept), self-discovers units
/// from the formula, returns a fresh clause vec, and signals UNSAT via a
/// canonical single empty clause with explicit assignment-array conflict
/// detection. The count contract differs, so the two are not merged.
pub(super) fn propagate_forced(
    clauses: &mut Vec<Clause>,
    forced: &[Literal],
    frozen: &rustc_hash::FxHashSet<VarId>,
) {
    if forced.is_empty() {
        return;
    }

    use rustc_hash::FxHashSet;
    let mut assigned: FxHashSet<(u32, bool)> = FxHashSet::default();
    let mut queue: Vec<Literal> = forced
        .iter()
        .copied()
        .filter(|l| !frozen.contains(&l.var))
        .collect();

    while let Some(lit) = queue.pop() {
        if !assigned.insert((lit.var.0, lit.positive)) {
            continue;
        }

        let mut i = 0;
        while i < clauses.len() {
            let contains_lit = clauses[i]
                .literals
                .iter()
                .any(|l| l.var == lit.var && l.positive == lit.positive);
            if contains_lit {
                clauses.swap_remove(i);
                continue;
            }

            let contains_negation = clauses[i]
                .literals
                .iter()
                .any(|l| l.var == lit.var && l.positive != lit.positive);
            if contains_negation {
                clauses[i].literals.retain(|l| l.var != lit.var);
                if clauses[i].literals.is_empty() {
                    return;
                }
                if clauses[i].literals.len() == 1 {
                    let unit = clauses[i].literals[0];
                    // A newly-derived unit on a frozen show var also must not be
                    // propagated (same invariant as the initial seed list).
                    if !frozen.contains(&unit.var) {
                        queue.push(unit);
                    }
                }
            }

            i += 1;
        }
    }
}

/// The count may be less than `defined.len()` if some vars were
/// non-resolvent-bounded and skipped for later retry.
pub(super) fn apply_elimination(
    clauses: &mut Vec<Clause>,
    defined: &[u32],
    fates: &mut [DveFate],
    max_clauses: usize,
    frozen: &rustc_hash::FxHashSet<VarId>,
) -> ElimYield {
    sort_clause_literals(clauses);
    let (elim_ids, forced, def_clauses) = elim_vars(clauses, defined, max_clauses, frozen);
    let elim_count = elim_ids.len();

    for v in elim_ids {
        fates[v as usize] = DveFate::Defined;
    }

    if !forced.is_empty() {
        propagate_forced(clauses, &forced, frozen);
    }
    dedup_clauses(clauses);

    ElimYield {
        eliminated: elim_count,
        definitions: def_clauses,
    }
}

/// `known_defined` lists vars structurally known to be defined (e.g. from
/// syntactic gate detection); these bypass the SAT definability probe and go
/// straight to resolution.
///
/// The `max_clauses` ceiling handed to `apply_elimination` is always the
/// current clause count, so no elimination that grows the formula is
/// accepted. High-fanout defined vars are left in place for the downstream
/// compiler to absorb instead.
pub(super) fn dve_round(
    clauses: &mut Vec<Clause>,
    num_vars: usize,
    fates: &mut [DveFate],
    time_limit_ms: u64,
    known_defined: &rustc_hash::FxHashSet<VarId>,
    frozen: &rustc_hash::FxHashSet<VarId>,
    meter: &mut crate::preprocess::meter::PreprocessMeter,
) -> ElimYield {
    let graph = if num_vars <= PRIMAL_GRAPH_MAX_VARS {
        Some(PrimalGraph::new(num_vars, clauses))
    } else {
        None
    };
    let freq = occ::literal_frequency(clauses, num_vars);

    // Known-defined (gate) vars skip the SAT definability probe but still
    // respect the profitability guard on small formulas. On large formulas
    // (no primal graph), all gate vars bypass the filter — this is essential
    // for circuit encodings where gates have high downstream fanout.
    let mut sat_candidates: Vec<u32> = Vec::new();
    let mut preknown: Vec<u32> = Vec::new();
    let bypass_ve_filter = graph.is_none();
    for v in 0..num_vars {
        if fates[v].eliminated() {
            continue;
        }
        if frozen.contains(&VarId(v as u32)) {
            continue;
        }
        if known_defined.contains(&VarId(v as u32)) {
            let pf = freq[v * 2] as u64;
            let nf = freq[v * 2 + 1] as u64;
            if pf == 0 && nf == 0 {
                continue;
            }
            if bypass_ve_filter || is_ve_candidate(graph.as_ref(), &freq, v) {
                preknown.push(v as u32);
            }
            // Gate vars that fail the VE filter are skipped this round —
            // strengthening may reduce their frequency enough to pass next round.
        } else if is_ve_candidate(graph.as_ref(), &freq, v) {
            sat_candidates.push(v as u32);
        }
    }

    if preknown.is_empty() && sat_candidates.is_empty() {
        return ElimYield {
            eliminated: 0,
            definitions: Vec::new(),
        };
    }

    // Sort both by total frequency (lowest first): cheaper dual-CNF probes for
    // SAT candidates, cheaper resolution for preknown.
    let freq_key = |&v: &u32| {
        let v = v as usize;
        freq[v * 2] + freq[v * 2 + 1]
    };
    sat_candidates.sort_by_key(freq_key);
    preknown.sort_by_key(freq_key);

    // CRITICAL: never run the SAT probe while any preknown gate var remains
    // un-eliminated in the formula. The probe shares non-candidate vars between
    // dual-CNF copies, so un-eliminated preknown gate clauses "pin" shared
    // state and can make SAT candidates look defined only because the gates
    // hold them in place. After such a spuriously-defined var is eliminated,
    // CNF resolvents can't encode the projected multiplicities and model count
    // is corrupted. See regression test `dve_preknown_first_preserves_mc`.
    //
    // Sequencing: eliminate preknown first. If any preknown vars from
    // `known_defined` remain un-eliminated (because profitability guard
    // deferred them), skip the SAT probe entirely — the next outer round will
    // retry with a reduced formula that may unlock the deferred preknown.
    let mut yielded = ElimYield {
        eliminated: 0,
        definitions: Vec::new(),
    };

    if !preknown.is_empty() {
        let max_clauses = clauses.len();
        let step = apply_elimination(clauses, &preknown, fates, max_clauses, frozen);
        yielded.eliminated += step.eliminated;
        yielded.definitions.extend(step.definitions);
    }

    let preknown_remaining = known_defined.iter().any(|v| !fates[v.idx()].eliminated());

    // Skip the SAT definability probe when the clause set is very large —
    // the dual-CNF encoding duplicates all candidate-touching clauses, which
    // causes OOM or stack overflow on multi-million-clause formulas.
    if !preknown_remaining && !sat_candidates.is_empty() && clauses.len() <= MAX_DUAL_CNF_CLAUSES {
        sat_candidates.retain(|&v| !fates[v as usize].eliminated());
        if !sat_candidates.is_empty() {
            let sat_defined =
                pick_def_vars_with_meter(clauses, num_vars, &sat_candidates, time_limit_ms, meter);
            if !sat_defined.is_empty() {
                let max_clauses = clauses.len();
                let step = apply_elimination(clauses, &sat_defined, fates, max_clauses, frozen);
                yielded.eliminated += step.eliminated;
                yielded.definitions.extend(step.definitions);
            }
        }
    }

    yielded
}
