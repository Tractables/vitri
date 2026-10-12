//! Definability elimination: one DVE round's probe and resolution, plus the
//! round bookkeeping the pipeline's loop reads.
//!
//! A variable may be eliminated only while it is defined in the formula as it
//! stands — then resolving it away (`∃x`) keeps the model count over the other
//! variables. A round has two ways to know that, and runs one phase for each:
//!
//! 1. **Probed.** [`pick_def_vars_with_meter`] returns a set jointly defined by
//!    the variables outside it. Eliminating members of the set, and any
//!    equivalence-preserving edit (subsumption), leaves the remaining members
//!    jointly defined, so every member stays eliminable for the whole phase.
//!    Nothing outside the set is eliminated in this phase: that could remove a
//!    variable the members are defined by.
//! 2. **By gate.** A variable whose own live clauses contain a gate definition
//!    ([`super::gates`]) is defined by that definition, whatever else the round
//!    did. The probed set's guarantee is no longer needed once its phase is
//!    over, so this phase may eliminate any unfrozen variable with a gate.
//!
//! Either way the elimination is one per variable, cheapest first by
//! occurrence product and requeued whenever its clauses change. A gate
//! definition, when the variable has one, restricts resolution to the pairs
//! with a definition clause on one side ([`super::gates`] says why that is
//! exact). Resolvents are tautology-filtered and subsumption-checked against
//! the live clauses; a variable goes when its surviving resolvents number at
//! most its clauses plus [`RESOLVENT_GROWTH_BOUND`] and none is longer than
//! [`MAX_RESOLVENT_LEN`], and the clauses its resolvents subsume go with it.
//! A variable left in no clause is not eliminated here: the pipeline names it
//! free (×2) when it finishes.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::cnf::{Clause, Literal, VarId};

use super::clause_db::{ClauseDb, ClauseId, LitMarks, subset_sorted};
use super::definability::{MAX_DUAL_CNF_CLAUSES, pick_def_vars_with_meter};
use super::gates::{Definition, find_definition};
use super::types::DveFate;

/// How many more clauses than it removes a variable's elimination may add.
/// One: a single extra clause often opens eliminations that shrink the
/// formula overall, which no growth at all forgoes; two or more mostly add
/// clauses.
pub(super) const RESOLVENT_GROWTH_BOUND: usize = 1;

/// The longest resolvent an elimination may add; a variable with a longer one
/// stays. Long resolvents are what a growth bound on the clause count alone
/// cannot see: the clauses stay few while the literals multiply.
pub(super) const MAX_RESOLVENT_LEN: usize = 32;

/// Work one round's elimination may do, in literal visits — resolution,
/// subsumption checks and gate matching all charge it. A deterministic bound,
/// under the stage's wall clock, which still applies.
const ELIMINATION_STEP_BUDGET: u64 = 100_000_000;

/// The most resolution pairs one variable's elimination may examine; a
/// variable with more stays. Keeps a variable whose resolvents are mostly
/// tautologies or subsumed from spending the round's budget alone.
const MAX_PAIRS_PER_VARIABLE: u64 = 100_000;

/// How many elimination attempts pass between two reads of the wall clock.
const CLOCK_POLL_ATTEMPTS: usize = 64;

/// What one round eliminated, by the phase that did it.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RoundElimination {
    /// Variables the probe returned as jointly defined.
    pub(super) proven_defined: usize,
    /// Of those, how many were eliminated.
    pub(super) probed: usize,
    /// Variables eliminated through a gate definition after the probed phase.
    pub(super) by_gate: usize,
    /// Whether the step budget or the wall clock cut the round short.
    pub(super) budget_hit: bool,
}

impl RoundElimination {
    pub(super) fn eliminated(&self) -> usize {
        self.probed + self.by_gate
    }
}

/// The outcome of one elimination attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Attempt {
    /// Resolved away; its fate is now [`DveFate::Defined`].
    Eliminated,
    /// Its elimination would break a bound, so it stays.
    Kept,
    /// It occurs in no live clause.
    Absent,
}

/// Which certificate the current phase eliminates under.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Members of the probed set, each eliminable whatever its clauses.
    Probed,
    /// Any unfrozen variable, eliminable only through a gate definition.
    Gate,
}

/// The per-variable elimination engine over one round's clause store.
pub(super) struct Eliminator<'a> {
    pub(super) db: ClauseDb,
    fates: &'a mut [DveFate],
    frozen: &'a rustc_hash::FxHashSet<VarId>,
    gate_definitions: bool,
    marks: LitMarks,
    /// Literal visits so far this round.
    pub(super) steps: u64,
    /// How many of [`Self::steps`] have been handed to the meter.
    charged: u64,
    budget: u64,
    /// Bumped whenever a clause containing the variable is added or removed.
    version: Vec<u32>,
    /// Whether the variable may be eliminated in the current phase.
    candidate: Vec<bool>,
    queue: BinaryHeap<Reverse<(u64, u32, u32)>>,
}

impl<'a> Eliminator<'a> {
    pub(super) fn new(
        clauses: Vec<Clause>,
        num_vars: usize,
        fates: &'a mut [DveFate],
        frozen: &'a rustc_hash::FxHashSet<VarId>,
        gate_definitions: bool,
    ) -> Self {
        Eliminator {
            db: ClauseDb::new(num_vars, clauses),
            fates,
            frozen,
            gate_definitions,
            marks: LitMarks::new(num_vars),
            steps: 0,
            charged: 0,
            budget: ELIMINATION_STEP_BUDGET,
            version: vec![0; num_vars],
            candidate: vec![false; num_vars],
            queue: BinaryHeap::new(),
        }
    }

    /// Whether `v` is still in the formula and not frozen.
    fn eligible(&self, v: usize) -> bool {
        !self.fates[v].eliminated() && !self.frozen.contains(&VarId::from_idx(v))
    }

    /// The eligible variables that occur, cheapest to probe first: fewest
    /// occurrences, ties by id.
    pub(super) fn probe_order(&self) -> Vec<u32> {
        let mut order: Vec<u32> = (0..self.db.num_vars())
            .filter(|&v| self.eligible(v) && self.db.occurrences_of_var(VarId::from_idx(v)) > 0)
            .map(|v| v as u32)
            .collect();
        order.sort_by_key(|&v| (self.db.occurrences_of_var(VarId::from_idx(v as usize)), v));
        order
    }

    /// The probe's shortcut: `v` has a gate definition none of whose inputs is
    /// in the set already, so it is defined by the variables the probe still
    /// holds fixed.
    pub(super) fn gate_defined_outside(&mut self, v: u32, in_set: &[bool]) -> bool {
        if !self.gate_definitions {
            return false;
        }
        let x = VarId::from_idx(v as usize);
        match find_definition(&self.db, x, &mut self.marks, &mut self.steps) {
            Some(definition) => definition.inputs(&self.db, x).all(|w| !in_set[w.idx()]),
            None => false,
        }
    }

    /// Eliminate what the phase allows, cheapest first, until the queue is
    /// empty or a budget is spent. `out_of_time` is handed the steps taken
    /// since its last call, every [`CLOCK_POLL_ATTEMPTS`] attempts and once at
    /// the end. Returns how many went, and whether a budget cut the phase
    /// short.
    fn run_phase(
        &mut self,
        phase: Phase,
        members: impl IntoIterator<Item = u32>,
        out_of_time: &mut dyn FnMut(u64) -> bool,
    ) -> (usize, bool) {
        self.queue.clear();
        self.candidate.fill(false);
        for v in members {
            if self.eligible(v as usize) {
                self.candidate[v as usize] = true;
                self.push(v as usize);
            }
        }
        let mut eliminated = 0;
        let mut attempts = 0usize;
        while let Some(Reverse((_, v, version))) = self.queue.pop() {
            let v = v as usize;
            if version != self.version[v] || !self.candidate[v] || self.fates[v].eliminated() {
                continue;
            }
            if self.steps > self.budget {
                return (eliminated, true);
            }
            attempts += 1;
            if attempts.is_multiple_of(CLOCK_POLL_ATTEMPTS) && out_of_time(self.uncharged()) {
                return (eliminated, true);
            }
            let x = VarId::from_idx(v);
            let definition = if self.gate_definitions {
                find_definition(&self.db, x, &mut self.marks, &mut self.steps)
            } else {
                None
            };
            if phase == Phase::Gate && definition.is_none() {
                continue;
            }
            if self.eliminate(x, definition.as_ref()) == Attempt::Eliminated {
                eliminated += 1;
            }
        }
        out_of_time(self.uncharged());
        (eliminated, false)
    }

    /// The steps not yet handed to the meter, marked handed.
    fn uncharged(&mut self) -> u64 {
        let delta = self.steps - self.charged;
        self.charged = self.steps;
        delta
    }

    /// Queue `v` at its current cost, if the phase may eliminate it.
    fn push(&mut self, v: usize) {
        if self.candidate[v] && !self.fates[v].eliminated() {
            let cost = self.db.occurrence_product(VarId::from_idx(v));
            self.queue.push(Reverse((cost, v as u32, self.version[v])));
        }
    }

    /// Note that the clauses of every variable of `literals` changed.
    fn touch(&mut self, literals: &[Literal]) {
        for lit in literals {
            let v = lit.var.idx();
            self.version[v] = self.version[v].wrapping_add(1);
            self.push(v);
        }
    }

    /// Resolve `x` away if the bounds allow, resolving only the pairs with a
    /// clause of `definition` on one side when one is given. The caller owes
    /// the soundness: `x` must be defined in the current formula (`definition`,
    /// when given, must be a gate definition of `x` among its live clauses).
    pub(super) fn eliminate(&mut self, x: VarId, definition: Option<&Definition>) -> Attempt {
        let pos: Vec<ClauseId> = self.db.occurrences(Literal::pos(x)).collect();
        let neg: Vec<ClauseId> = self.db.occurrences(Literal::neg(x)).collect();
        let removed = pos.len() + neg.len();
        if removed == 0 {
            return Attempt::Absent;
        }
        let limit = removed + RESOLVENT_GROWTH_BOUND;
        let resolves =
            |a: ClauseId, b: ClauseId| definition.is_none_or(|d| d.contains(a) || d.contains(b));
        let mut pairs = 0u64;
        let mut kept: Vec<Vec<Literal>> = Vec::new();
        for &a in &pos {
            for &b in &neg {
                if !resolves(a, b) {
                    continue;
                }
                pairs += 1;
                if pairs > MAX_PAIRS_PER_VARIABLE || self.steps > self.budget {
                    return Attempt::Kept;
                }
                let (ca, cb) = (self.db.clause(a), self.db.clause(b));
                self.steps += (ca.len() + cb.len()) as u64;
                let Some(resolvent) = crate::cnf::resolve_sorted(ca, cb, x) else {
                    continue;
                };
                if self.subsumed_by_live(&resolvent) || !self.insert_minimal(&mut kept, resolvent) {
                    continue;
                }
                if kept.len() > limit {
                    return Attempt::Kept;
                }
            }
        }
        if kept.iter().any(|r| r.len() > MAX_RESOLVENT_LEN) {
            return Attempt::Kept;
        }

        for &id in pos.iter().chain(&neg) {
            self.db.remove(id);
        }
        self.fates[x.idx()] = DveFate::Defined;
        for &id in pos.iter().chain(&neg) {
            let literals = self.db.clause(id).to_vec();
            self.touch(&literals);
        }
        for resolvent in kept {
            self.remove_subsumed_by(&resolvent);
            self.touch(&resolvent);
            self.db.add(resolvent);
        }
        Attempt::Eliminated
    }

    /// Whether a live clause is a subset of `resolvent`. Each candidate is
    /// reached once, through the list of its own first literal.
    fn subsumed_by_live(&mut self, resolvent: &[Literal]) -> bool {
        self.marks.clear();
        for &lit in resolvent {
            self.marks.mark(lit);
        }
        for &lit in resolvent {
            for id in self.db.first_occurrences(lit) {
                let clause = self.db.clause(id);
                self.steps += 1;
                if clause.len() > resolvent.len() {
                    continue;
                }
                self.steps += clause.len() as u64;
                if clause.iter().all(|&l| self.marks.is_marked(l)) {
                    return true;
                }
            }
        }
        false
    }

    /// Add `resolvent` to this elimination's resolvents unless one of them is a
    /// subset of it, dropping those it is a subset of. Returns whether it went
    /// in.
    fn insert_minimal(&mut self, kept: &mut Vec<Vec<Literal>>, resolvent: Vec<Literal>) -> bool {
        self.steps += (kept.len() * resolvent.len()) as u64;
        if kept.iter().any(|k| subset_sorted(k, &resolvent)) {
            return false;
        }
        kept.retain(|k| !subset_sorted(&resolvent, k));
        kept.push(resolvent);
        true
    }

    /// Remove every live clause that contains all of `resolvent`'s literals,
    /// which the resolvent makes redundant.
    fn remove_subsumed_by(&mut self, resolvent: &[Literal]) {
        let Some(&rarest) = resolvent
            .iter()
            .min_by_key(|&&l| self.db.occurrence_count(l))
        else {
            return;
        };
        self.marks.clear();
        for &lit in resolvent {
            self.marks.mark(lit);
        }
        let mut subsumed: Vec<ClauseId> = Vec::new();
        for id in self.db.occurrences(rarest) {
            let clause = self.db.clause(id);
            self.steps += clause.len() as u64;
            if clause.len() >= resolvent.len()
                && clause.iter().filter(|&&l| self.marks.is_marked(l)).count() == resolvent.len()
            {
                subsumed.push(id);
            }
        }
        for id in subsumed {
            self.db.remove(id);
            let literals = self.db.clause(id).to_vec();
            self.touch(&literals);
        }
    }
}

/// One round of definability elimination over `clauses`: the probe, then the
/// probed and gate phases described in the module doc.
///
/// The probe gets at most half of `time_limit_ms`, so the phases that act on
/// what it found are never left without time; the phases stop at
/// [`ELIMINATION_STEP_BUDGET`] or when the round has spent `time_limit_ms`.
/// The probe is skipped above [`MAX_DUAL_CNF_CLAUSES`], which leaves the gate
/// phase.
pub(super) fn dve_round(
    clauses: &mut Vec<Clause>,
    num_vars: usize,
    fates: &mut [DveFate],
    time_limit_ms: u64,
    gate_definitions: bool,
    frozen: &rustc_hash::FxHashSet<VarId>,
    meter: &mut crate::preprocess::meter::PreprocessMeter,
) -> RoundElimination {
    let mark = meter.begin(
        crate::bundle::PreprocessPhase::Dve,
        std::time::Duration::from_millis(time_limit_ms),
    );
    let mut engine = Eliminator::new(
        std::mem::take(clauses),
        num_vars,
        fates,
        frozen,
        gate_definitions,
    );
    let mut round = RoundElimination::default();

    let order = engine.probe_order();
    let proven: Vec<u32> = if !order.is_empty() && engine.db.live_count() <= MAX_DUAL_CNF_CLAUSES {
        let snapshot = engine.db.to_clauses();
        pick_def_vars_with_meter(
            &snapshot,
            num_vars,
            &order,
            time_limit_ms / 2,
            meter,
            |v, in_set| engine.gate_defined_outside(v, in_set),
        )
    } else {
        Vec::new()
    };
    round.proven_defined = proven.len();

    let mut out_of_time = |steps: u64| {
        meter.charge(steps);
        meter.elapsed_ms(mark) > time_limit_ms
    };
    let (probed, cut) = engine.run_phase(Phase::Probed, proven, &mut out_of_time);
    round.probed = probed;
    round.budget_hit = cut;
    if gate_definitions && !cut {
        let all = (0..num_vars as u32).collect::<Vec<_>>();
        let (by_gate, cut) = engine.run_phase(Phase::Gate, all, &mut out_of_time);
        round.by_gate = by_gate;
        round.budget_hit = cut;
    }

    *clauses = engine.db.into_clauses();
    round
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

    // Each elimination grows the clause count by at most
    // `RESOLVENT_GROWTH_BOUND`; this catches the drift that accumulates across
    // eliminations and rounds.
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
        .flat_map(|c| c.literals.iter().map(|l| l.var.get()))
        .collect::<std::collections::HashSet<_>>()
        .len()
}

/// Length-first sort order is what makes `dedup()` catch every duplicate.
pub(super) fn dedup_clauses(clauses: &mut Vec<Clause>) {
    clauses.retain_mut(|clause| {
        match crate::cnf::normalize_literals(std::mem::take(&mut clause.literals)) {
            Some(literals) => {
                clause.literals = literals;
                true
            }
            None => false,
        }
    });
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
