//! Unified probing engine — ONE CaDiCaL session shared between
//! backbone detection and literal-equivalence detection.
//!
//! Backbone detection and equivalence detection are both **partition refinement
//! over literals by their value-vector across every model seen**:
//!
//! - Two literals are equivalence candidates iff they have agreed in every model
//!   so far — i.e. they sit in the same class.
//! - A literal is a backbone candidate iff it has been TRUE in every model so far
//!   — i.e. it sits in the distinguished class whose value-vector is all-ones
//!   (the **⊤-class**).
//!
//! A single shared partition and a single solver session make each backbone/
//! equivalence pass reinforce the other:
//!   1. Backbone counter-models refine equivalence classes (free).
//!   2. `flippable(lit)` splits `lit` from its class — a free equivalence
//!      refutation (a literal individually flippable in the current model cannot
//!      be equivalent to any class member that stays put).
//!   3. `fixed()` harvests and pinned backbone units strengthen the shared solver
//!      for every subsequent probe of both kinds.
//!   4. Phase-4 Tarjan substitutions are ingested as class merges.
//!
//! **Variable space.** The engine lives in phase-2 space (the Tarjan-reduced
//! formula of pipeline phase 1) for its whole life — the solver is loaded once and
//! never rebuilt. Phase-4 Tarjan substitutions are fed in via
//! [`ProbeEngine::ingest_tarjan_equivs`] (the eliminated vars are dropped from
//! the partition so the engine neither probes nor emits them). On emit, confirmed
//! equivalences are mapped through the phase-4 `EquivMapping` and any pair whose
//! two members collapse to the same representative is dropped (already known — no
//! tautology is injected).
//!
//! **Soundness**: confirmed facts come ONLY from UNSAT probes and `fixed()`;
//! models only ever REFUTE candidates. A missed refinement is a wasted probe,
//! never a wrong fact. Backbone units and equivalences are consequences of the
//! formula; the downstream injection/stripping machinery is untouched.
//!
//! The two passes return the [`crate::preprocess::backbone::BackboneResult`] /
//! [`crate::preprocess::backbone::EquivResult`] shapes the pipeline's stats
//! lines and `BackboneStats` consume — both live in
//! [`crate::preprocess::backbone`].

use std::collections::HashSet;
use std::time::{Duration, Instant};

use super::cadical_ffi::{Bounded, CaDiCal, Status};

use crate::cnf::{CnfFormula, Literal, VarId};

use super::backbone::{BackboneResult, EquivResult, read_model, refine_candidates};
use super::cadical::WallClockTerminator;
use super::equivalence::EquivMapping;
use crate::cnf::occ;

/// Per-probe conflict budget for single-var backbone probes. A single
/// individual probe can drive CaDiCaL's CDCL into a long conflict grind;
/// capping it lets an over-budget probe return UNKNOWN short of the budget
/// instead of eating the whole compile budget. An UNKNOWN var is DEFERRED to
/// the back of the queue, not discarded: once some later probe lands an UNSAT
/// and pins a unit, propagation typically decides the deferred vars in ~0ms
/// (measured on the backbone-dominated slow solves), so revisiting them is
/// nearly free and recovers backbones a skip-forever policy would lose.
///
/// Do NOT replace the flat cap with low-cap deepening/rotation: on the
/// probe-grind instances the first decidable var needs just-under-the-cap
/// conflicts (so low caps decide nothing), and rotating past it in queue
/// order delays the first unit-pin catastrophically — measured across
/// benchmark CNFs as turning a healthy run into a budget length one.
const MAX_CONFLICTS: i32 = 64_000;

/// True iff signed DIMACS literal `lit` is TRUE under `model`. `model[var-1]` is
/// the signed value (0 = unassigned, treated as "not true" — matching
/// `backbone::refine_candidates`). An unassigned var pushing a literal out of the
/// ⊤-class only costs a missed backbone (sound), never a wrong fact.
#[inline]
fn lit_true_in_model(lit: i32, model: &[i32]) -> bool {
    let mv = model[VarId::from_dimacs(lit).idx()];
    (lit > 0 && mv > 0) || (lit < 0 && mv < 0)
}

/// Map a signed DIMACS literal (phase-2 space) through the phase-4 `EquivMapping`
/// to its representative literal. `None` mapping = identity.
fn map_lit(d: i32, mapping: &Option<EquivMapping>) -> Literal {
    let lit = Literal::from(d);
    match mapping {
        None => lit,
        Some(m) => {
            let rep = m.var_to_rep[lit.var.idx()];
            if lit.positive { rep } else { rep.negated() }
        }
    }
}

/// One CaDiCaL session + one literal partition shared across the backbone and
/// equivalence passes.
pub(super) struct ProbeEngine {
    /// The single solver: formula loaded once (phase-2 space), seed-solved once.
    solver: CaDiCal,
    /// Variable count of the loaded (phase-2) formula.
    num_vars: usize,
    /// Clause frequency per variable, for the backbone candidate ordering.
    freq: Vec<u32>,
    /// The partition and everything probing has confirmed about it.
    pub(super) partition: Partition,
}

/// The literal partition and the facts probing has established over it.
///
/// Refining it is pure bookkeeping — no solver, no `unsafe` — so it is separate
/// from the session that produces the models: the refinement rules are testable
/// on their own, and a probing loop can rewrite the partition while the solver
/// is bounded by a terminator guard that borrows the session.
pub(super) struct Partition {
    /// Partition of DIMACS "true literals" by value-vector across all models seen.
    /// Each class holds signed DIMACS literals that have agreed in every model.
    pub(super) classes: Vec<Vec<i32>>,
    /// Index of the ⊤-class (the all-ones class = current backbone candidates).
    /// `usize::MAX` once backbone probing is done (no distinguished class).
    pub(super) top: usize,
    /// Whether the seed solve succeeded (a model exists and classes are seeded).
    seeded: bool,
    /// Confirmed backbone literals (from UNSAT probes and `fixed()`).
    pub(super) confirmed_backbone: Vec<Literal>,
    /// Confirmed equivalences as raw phase-2-space DIMACS pairs (`a ≡ b`).
    confirmed_equiv: Vec<(i32, i32)>,
}

impl Partition {
    /// An empty partition: nothing seeded, no distinguished ⊤-class.
    pub(super) fn new() -> Self {
        Partition {
            classes: Vec::new(),
            top: usize::MAX,
            seeded: false,
            confirmed_backbone: Vec::new(),
            confirmed_equiv: Vec::new(),
        }
    }

    /// Refine every class by `model`'s bit: split each into (true-in-model,
    /// false-in-model). The ⊤-class's true-half stays the ⊤-class (its all-ones
    /// value-vector continues); its false-half becomes a new class. Non-⊤ classes
    /// keep only halves of size ≥ 2 (singletons can no longer yield equivalences).
    /// O(vars), no SAT.
    pub(super) fn observe_model(&mut self, model: &[i32]) {
        let top = self.top;
        let old = std::mem::take(&mut self.classes);
        let mut new_top = usize::MAX;
        for (ci, class) in old.into_iter().enumerate() {
            let mut t_half = Vec::new();
            let mut f_half = Vec::new();
            for lit in class {
                if lit_true_in_model(lit, model) {
                    t_half.push(lit);
                } else {
                    f_half.push(lit);
                }
            }
            if ci == top {
                // The ⊤-class true-half is the anchor: keep it even when small so
                // its index stays trackable; the false-half is an ordinary class.
                new_top = self.classes.len();
                self.classes.push(t_half);
                if f_half.len() >= 2 {
                    self.classes.push(f_half);
                }
            } else {
                if t_half.len() >= 2 {
                    self.classes.push(t_half);
                }
                if f_half.len() >= 2 {
                    self.classes.push(f_half);
                }
            }
        }
        self.top = new_top;
    }

    /// Remove `lits` from every class in place (retain), preserving class indices
    /// (so `self.top` stays valid). Used to drop confirmed backbone literals and
    /// phase-4-eliminated literals from the partition.
    fn remove_lits(&mut self, lits: &HashSet<i32>) {
        for class in &mut self.classes {
            class.retain(|l| !lits.contains(l));
        }
    }
}

impl ProbeEngine {
    /// Load `formula` (phase-2 space) into a fresh CaDiCaL session. No solve yet
    /// — the seed solve happens in [`ProbeEngine::run_backbone`] so seed + probing
    /// share one budget window.
    ///
    /// `None` when no solver could be allocated: there is no session to probe
    /// in, and the caller's stage has nothing to report.
    pub(super) fn new(formula: &CnfFormula) -> Option<Self> {
        let mut solver = CaDiCal::new()?;
        // Do NOT configure("sat") here even though the seed solve is
        // expected-SAT: measured on the backbone-dominated slow solves, the
        // preset shifts the whole search trajectory chaotically (one seed 60s
        // → 2s, another 3s → 21s, probe phases swinging ±40s) and flipped a
        // solving instance to a timeout.
        for clause in &formula.clauses {
            for lit in &clause.literals {
                solver.add(lit.to_dimacs());
            }
            solver.add(0);
        }
        // Conflict-bound the seed solve so an adversarial formula can't drive
        // solve() into an unbounded conflict loop before the budget check.
        solver.limit(c"conflicts", 1_000_000);
        let num_vars = formula.num_vars as usize;
        let freq = occ::frequency(&formula.clauses, num_vars);
        Some(ProbeEngine {
            solver,
            num_vars,
            freq,
            partition: Partition::new(),
        })
    }

    /// Phase-2: seed solve + backbone probing on the ⊤-class. Returns the
    /// [`BackboneResult`] shape the pipeline's shared stats code prints. Every
    /// SAT counter-model is routed through `ProbeEngine::observe_model`, refining
    /// the equivalence classes for free (win #1). Confirmed literals are pinned
    /// as units in the shared solver.
    pub(super) fn run_backbone(&mut self, budget: Duration) -> BackboneResult {
        let start = Instant::now();
        let nv = self.num_vars;
        let empty = |solve_ms: u64, unsat: bool| BackboneResult {
            forced: Vec::new(),
            probes_completed: 0,
            solve_ms,
            unsat,
            fixed_found: 0,
            flippable_eliminated: 0,
            model_eliminated: 0,
        };
        if nv == 0 {
            return empty(0, false);
        }

        // Seed solve, bounded by a wall-clock terminator (ceiling = budget).
        // The guard owns the terminator and disconnects it when it drops, so
        // the early returns below need no cleanup of their own.
        let mut solver = Bounded::new(&mut self.solver, WallClockTerminator::new(budget));
        let status = solver.solve();
        let solve_ms = start.elapsed().as_millis() as u64;
        match status {
            Status::Unsatisfiable => return empty(solve_ms, true),
            Status::Satisfiable => {}
            _ => return empty(solve_ms, false),
        }

        let model = read_model(&mut solver, nv);

        // Seed the partition: one class of every assigned true-literal — this is
        // the initial ⊤-class (all-ones so far). Vars unassigned in the seed
        // (val == 0, e.g. eliminated by inprocessing) are excluded (an unassigned
        // var can't be a backbone or equivalence candidate).
        let mut top_class = Vec::new();
        for &lit in &model {
            if lit != 0 {
                top_class.push(lit);
            }
        }
        self.partition.classes = vec![top_class];
        self.partition.top = 0;
        self.partition.seeded = true;

        let (fixed_found, flippable_eliminated) =
            harvest_fixed_and_flippable(&mut self.partition, &mut solver, &model, nv);

        // Frequency-sorted backbone candidate list = the ⊤-class, high-frequency
        // first (high-frequency vars are more likely backbone / more impactful).
        let mut candidates: Vec<i32> = self.partition.classes[self.partition.top].clone();
        candidates.sort_unstable_by(|&a, &b| {
            let fa = self.freq[VarId::from_dimacs(a).idx()];
            let fb = self.freq[VarId::from_dimacs(b).idx()];
            fb.cmp(&fa)
        });

        let probed = probe_loop(
            &mut self.partition,
            &mut solver,
            &mut candidates,
            nv,
            start,
            budget,
        );
        let recovered = recover_deferred(
            &mut self.partition,
            &mut solver,
            &probed.deferred,
            nv,
            start,
            budget,
        );

        BackboneResult {
            forced: self.partition.confirmed_backbone.clone(),
            probes_completed: probed.probes_completed + recovered,
            solve_ms,
            unsat: false,
            fixed_found,
            flippable_eliminated,
            model_eliminated: probed.model_eliminated,
        }
    }

    /// Ingest phase-4 Tarjan substitutions as class merges (win #4): the
    /// eliminated variables no longer appear in the pipeline formula `f`, so their
    /// literals are dropped from the partition — the engine must neither probe nor
    /// emit them (they are already substituted into `f`).
    pub(super) fn ingest_tarjan_equivs(&mut self, mapping: &EquivMapping) {
        let mut drop: HashSet<i32> = HashSet::new();
        for (v, &rep) in mapping.var_to_rep.iter().enumerate() {
            if rep.var.0 != v as u32 {
                let d = (v as i32) + 1;
                drop.insert(d);
                drop.insert(-d);
            }
        }
        if !drop.is_empty() {
            self.partition.remove_lits(&drop);
        }
    }

    /// Phase-5: equivalence probing on the ALREADY-REFINED classes (pre-refined by
    /// the backbone pass's counter-models — this is where the probe-heavy
    /// instances should collapse). Largest-class-first, two-direction probes,
    /// rep-relative refinement — on the engine's shared partition, routing every
    /// counter-model through `observe_model` so it refines the OTHER classes too.
    /// Confirmed equivalences are mapped through the phase-4 `EquivMapping`
    /// (`mapping2`); same-representative pairs are dropped (already known — no
    /// tautology injected). Returns the [`EquivResult`] shape the pipeline's
    /// phase-5 stats code consumes.
    pub(super) fn run_equiv(
        &mut self,
        budget: Duration,
        mapping2: &Option<EquivMapping>,
    ) -> EquivResult {
        let start = Instant::now();
        let nv = self.num_vars;
        if !self.partition.seeded {
            // No seed model (seed solve timed out / no backbone pass) — nothing to
            // probe. The engine deliberately spends exactly ONE seed solve (in
            // run_backbone) and never re-seeds here.
            return EquivResult {
                equivalences: Vec::new(),
                probes_completed: 0,
                unsat: false,
            };
        }
        // The ⊤-class is no longer distinguished; all classes are treated
        // uniformly for equivalence probing.
        self.partition.top = usize::MAX;

        let mut solver = Bounded::new(&mut self.solver, WallClockTerminator::new(budget));

        let mut probes_completed = 0;
        // Process classes largest-first (more equivalences per probe; a failed
        // probe refines the whole partition at once).
        loop {
            if start.elapsed() >= budget {
                break;
            }
            self.partition
                .classes
                .sort_unstable_by_key(|c| std::cmp::Reverse(c.len()));
            while self.partition.classes.last().is_some_and(|p| p.len() < 2) {
                self.partition.classes.pop();
            }
            if self.partition.classes.is_empty() || self.partition.classes[0].len() < 2 {
                break;
            }

            // Take the largest class OUT to probe it; its refinement is handled
            // locally, while observe_model (below) refines the classes that remain.
            let class = self.partition.classes.remove(0);
            let rep = class[0];
            let mut confirmed = vec![rep];
            let mut remaining: Vec<i32> = class[1..].to_vec();

            let mut i = 0;
            while i < remaining.len() {
                if start.elapsed() >= budget {
                    break;
                }
                let candidate = remaining[i];
                probes_completed += 1;

                // Direction 1: rep ∧ ¬candidate → UNSAT?
                solver.assume(rep);
                solver.assume(-candidate);
                match solver.solve() {
                    Status::Satisfiable => {
                        // rep is TRUE in this model (we assumed `rep`).
                        let new_model = read_model(&mut solver, nv);
                        self.partition.observe_model(&new_model);
                        let (stay, split) = refine_candidates(&remaining[i..], &new_model, true);
                        remaining.truncate(i);
                        remaining.extend(stay);
                        if split.len() >= 2 {
                            self.partition.classes.push(split);
                        }
                        continue; // remaining restructured — don't advance i
                    }
                    Status::Unsatisfiable => {} // fall through to direction 2
                    _ => {
                        i += 1;
                        continue;
                    }
                }

                // Direction 2: ¬rep ∧ candidate → UNSAT?
                probes_completed += 1;
                solver.assume(-rep);
                solver.assume(candidate);
                match solver.solve() {
                    Status::Unsatisfiable => {
                        // Confirmed: rep ↔ candidate.
                        confirmed.push(candidate);
                        remaining.remove(i);
                    }
                    Status::Satisfiable => {
                        // rep is FALSE in this model (we assumed `-rep`); refine
                        // with the rep-false criterion (guarantees progress, no
                        // livelock).
                        let new_model = read_model(&mut solver, nv);
                        self.partition.observe_model(&new_model);
                        let (stay, split) = refine_candidates(&remaining[i..], &new_model, false);
                        remaining.truncate(i);
                        remaining.extend(stay);
                        if split.len() >= 2 {
                            self.partition.classes.push(split);
                        }
                        continue; // remaining restructured — don't advance i
                    }
                    _ => {
                        i += 1;
                    }
                }
            }

            if confirmed.len() >= 2 {
                for &other in &confirmed[1..] {
                    self.partition.confirmed_equiv.push((rep, other));
                }
            }
            if remaining.len() >= 2 {
                self.partition.classes.push(remaining);
            }
        }

        // Map confirmed equivalences through the phase-4 EquivMapping; drop any
        // pair whose two literals collapse to the same representative — a
        // tautology if same polarity, a contradiction if opposite, either way
        // not a new fact to inject.
        let mut equivalences = Vec::new();
        for &(a, b) in &self.partition.confirmed_equiv {
            let la = map_lit(a, mapping2);
            let lb = map_lit(b, mapping2);
            if la.var == lb.var {
                continue;
            }
            equivalences.push((la, lb));
        }

        EquivResult {
            equivalences,
            probes_completed,
            unsat: false,
        }
    }
}

/// Confirm `lits` as backbone literals: record each one, pin it as a unit in
/// the shared solver so every later probe is strengthened by it, and drop it
/// from the partition — a confirmed literal is a constant, not a member of an
/// equivalence class. The three steps are one soundness contract, so both probe
/// loops discharge it here rather than each spelling it out.
fn confirm_backbone(
    partition: &mut Partition,
    solver: &mut Bounded<'_, WallClockTerminator>,
    lits: impl IntoIterator<Item = i32>,
) {
    let mut confirmed: HashSet<i32> = HashSet::new();
    for lit in lits {
        partition.confirmed_backbone.push(Literal::from(lit));
        solver.add(lit);
        solver.add(0);
        confirmed.insert(lit);
    }
    partition.remove_lits(&confirmed);
}

/// Phases 0 and 1 of backbone probing, both free of a solve: harvest the
/// literals CaDiCaL has already fixed, then the ones it reports individually
/// flippable in `model`. Returns `(fixed_found, flippable_eliminated)`.
fn harvest_fixed_and_flippable(
    partition: &mut Partition,
    solver: &mut Bounded<'_, WallClockTerminator>,
    model: &[i32],
    nv: usize,
) -> (usize, usize) {
    let mut fixed_found = 0;
    let mut flippable_eliminated = 0;

    // Phase 0: harvest backbone literals CaDiCaL already knows (free).
    let mut remove: HashSet<i32> = HashSet::new();
    for i in 0..nv {
        let dimacs = VarId(i as u32).to_dimacs();
        let f = solver.fixed(dimacs);
        if f != 0 {
            let lit = if f > 0 { dimacs } else { -dimacs };
            partition.confirmed_backbone.push(Literal::from(lit));
            fixed_found += 1;
            remove.insert(lit);
        }
    }

    // Phase 1: flippable() harvest — a literal individually flippable in the
    // current model is not backbone AND is not equivalent to anything still in
    // the ⊤-class (flipping it alone yields a model where it disagrees). So it
    // splits out of the partition entirely (win #2).
    for (i, &val) in model.iter().enumerate() {
        if val == 0 {
            continue;
        }
        if solver.fixed(VarId(i as u32).to_dimacs()) != 0 {
            continue; // already fixed above
        }
        if solver.flippable(-val) {
            flippable_eliminated += 1;
            remove.insert(val);
        }
    }
    if !remove.is_empty() {
        partition.remove_lits(&remove);
    }

    (fixed_found, flippable_eliminated)
}

/// What the probing loop leaves for the caller: the two counters the stats line
/// reports, and the candidates it could not decide inside the conflict cap.
struct ProbeRun {
    probes_completed: usize,
    model_eliminated: usize,
    /// Vars whose probe returned UNKNOWN (conflict-cap exhaustion, see
    /// `MAX_CONFLICTS`), for [`recover_deferred`] to retry once at a tiny cap.
    deferred: Vec<i32>,
}

/// Phase 2: SAT probing with CadiBack-style adaptive chunking (chunk_limit
/// starts at 1, grows 8× after each UNSAT burst, resets to 1 after a SAT
/// counter-model). Every counter-model refines ALL classes, not just the
/// backbone candidates, and recompacts `candidates` to those still in the
/// ⊤-class.
fn probe_loop(
    partition: &mut Partition,
    solver: &mut Bounded<'_, WallClockTerminator>,
    candidates: &mut Vec<i32>,
    nv: usize,
    start: Instant,
    budget: Duration,
) -> ProbeRun {
    let mut probes_completed = 0;
    let mut model_eliminated = 0;
    let mut chunk_limit: usize = 1;
    let mut pos = 0;
    let mut deferred: Vec<i32> = Vec::new();

    while pos < candidates.len() {
        if start.elapsed() >= budget {
            break;
        }
        let remaining = candidates.len() - pos;
        let chunk_size = chunk_limit.min(remaining);
        probes_completed += 1;

        let probe = if chunk_size == 1 {
            solver.limit(c"conflicts", MAX_CONFLICTS);
            solver.assume(-candidates[pos]);
            solver.solve()
        } else {
            solver.limit(c"conflicts", 1_000_000);
            for &cand in &candidates[pos..pos + chunk_size] {
                solver.constrain(-cand);
            }
            solver.constrain(0);
            solver.solve()
        };

        match probe {
            Status::Unsatisfiable => {
                // All candidates in this chunk are backbone.
                confirm_backbone(
                    partition,
                    solver,
                    candidates[pos..pos + chunk_size].iter().copied(),
                );
                pos += chunk_size;
                chunk_limit = chunk_limit.saturating_mul(8).max(1);
            }
            Status::Satisfiable => {
                // Counter-model: refine ALL classes (win #1), then recompact
                // the candidate list to those still in the ⊤-class (drop any
                // var the counter-model just proved non-backbone).
                let new_model = read_model(solver, nv);
                partition.observe_model(&new_model);
                let top_set: HashSet<i32> =
                    partition.classes[partition.top].iter().copied().collect();
                let mut write = pos;
                for read in pos..candidates.len() {
                    if top_set.contains(&candidates[read]) {
                        candidates[write] = candidates[read];
                        write += 1;
                    } else {
                        model_eliminated += 1;
                    }
                }
                candidates.truncate(write);
                chunk_limit = 1;
            }
            _ => {
                // UNKNOWN: conflict-cap exhaustion (short of the budget) →
                // set the probed vars aside for the recovery pass and keep
                // draining; otherwise the real deadline → stop. Chunk
                // probes land here too when they exhaust their 1M cap
                // (observed on the probe-grind instances) — the whole
                // chunk defers.
                if start.elapsed() < budget {
                    deferred.extend(candidates.drain(pos..pos + chunk_size));
                    chunk_limit = 1;
                } else {
                    break;
                }
            }
        }
    }

    ProbeRun {
        probes_completed,
        model_eliminated,
        deferred,
    }
}

/// Recovery pass: retry each deferred var once at a tiny conflict cap (see
/// `MAX_CONFLICTS` for why this recovers backbones cheaply). A still-hard var
/// burns at most `RECOVERY_CAP` conflicts (~milliseconds) and stays undecided.
/// Returns how many probes it ran.
fn recover_deferred(
    partition: &mut Partition,
    solver: &mut Bounded<'_, WallClockTerminator>,
    deferred: &[i32],
    nv: usize,
    start: Instant,
    budget: Duration,
) -> usize {
    const RECOVERY_CAP: i32 = 1_000;
    let mut probes_completed = 0;
    for &lit in deferred {
        if start.elapsed() >= budget {
            break;
        }
        probes_completed += 1;
        solver.limit(c"conflicts", RECOVERY_CAP);
        solver.assume(-lit);
        match solver.solve() {
            Status::Unsatisfiable => confirm_backbone(partition, solver, [lit]),
            Status::Satisfiable => {
                let new_model = read_model(solver, nv);
                partition.observe_model(&new_model);
            }
            _ => {}
        }
    }
    probes_completed
}

// ── Tests ──────────────────────────────────────────────────────────────────
