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
//!   4. The substitutions of the post-backbone Tarjan pass are ingested as
//!      class merges.
//!
//! **Variable space.** The engine is loaded once, with the output of
//! [`Stage::Tarjan`](super::pipelines::Stage), and never rebuilt. It works on
//! the variables that occur in a clause of that formula only, renumbered
//! `1..=K` in their original order: a variable no clause mentions is neither a
//! backbone nor equivalent to anything, and loading it would leave the solver
//! deciding it on every probe and every model read visiting it. Everything the
//! engine reports is named back in the formula's own variables. The
//! post-backbone Tarjan pass runs on a formula the engine never sees, so its
//! substitutions are fed in via [`ProbeEngine::ingest_tarjan_equivs`] (the
//! eliminated vars are dropped from the partition so the engine neither probes
//! nor emits them). On emit, confirmed equivalences are mapped through that
//! pass's `EquivMapping` and any pair whose two members collapse to the same
//! representative is dropped (already known — no tautology is injected).
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

use std::time::{Duration, Instant};

use super::cadical_ffi::{Bounded, CaDiCal, Status};

use crate::cnf::{CnfFormula, Literal, VarId};

use super::backbone::{BackboneResult, EquivResult, read_model, refine_candidates, refresh_model};
use super::cadical::WallClockTerminator;
use super::equivalence::EquivMapping;
use super::renumber::Renumber;
use crate::bundle::PreprocessPhase;
use crate::cnf::occ;

/// One solver session with a real terminator in wall mode and no terminator in
/// deterministic mode. Deref keeps the probing algorithm singular.
enum ProbeSolver<'a> {
    Direct(&'a mut CaDiCal),
    Wall(Bounded<'a, WallClockTerminator>),
}

impl<'a> ProbeSolver<'a> {
    /// The solver a probing phase runs on: bare under deterministic
    /// preprocessing, where the meter's charged units are the bound, and
    /// wrapped in a wall-clock terminator otherwise. The guard owns the
    /// terminator and disconnects it when it drops, so a phase's early returns
    /// need no cleanup of their own.
    fn for_phase(
        solver: &'a mut CaDiCal,
        meter: &super::meter::PreprocessMeter,
        budget: Duration,
    ) -> Self {
        if meter.deterministic() {
            ProbeSolver::Direct(solver)
        } else {
            ProbeSolver::Wall(Bounded::new(solver, WallClockTerminator::new(budget)))
        }
    }
}

impl std::ops::Deref for ProbeSolver<'_> {
    type Target = CaDiCal;

    fn deref(&self) -> &CaDiCal {
        match self {
            ProbeSolver::Direct(solver) => solver,
            ProbeSolver::Wall(solver) => solver,
        }
    }
}

impl std::ops::DerefMut for ProbeSolver<'_> {
    fn deref_mut(&mut self) -> &mut CaDiCal {
        match self {
            ProbeSolver::Direct(solver) => solver,
            ProbeSolver::Wall(solver) => solver,
        }
    }
}

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
pub(super) const MAX_CONFLICTS: i32 = 64_000;

/// The conflict bound on a solve that is meant to run to an answer: the whole-
/// formula seed solve and the chunked probe that asks about many candidates at
/// once. High enough that no ordinary formula reaches it, and there so an
/// adversarial one cannot drive `solve()` into an unbounded conflict loop before
/// the next budget check. Not a tuning knob: a solve that hits it has already
/// gone wrong.
const RUN_TO_ANSWER_CONFLICTS: i32 = 1_000_000;

/// True iff signed DIMACS literal `lit` is TRUE under `model`. `model[var-1]` is
/// the signed value (0 = unassigned, treated as "not true" — matching
/// `backbone::refine_candidates`). An unassigned var pushing a literal out of the
/// ⊤-class only costs a missed backbone (sound), never a wrong fact.
#[inline]
pub(super) fn lit_true_in_model(lit: i32, model: &[i32]) -> bool {
    let mv = model[VarId::from_dimacs(lit).idx()];
    (lit > 0 && mv > 0) || (lit < 0 && mv < 0)
}

/// One equivalence probe: assume both literals, cap the search where the meter
/// says to, and solve. The two directions of a probe differ in what they assume
/// and in what an UNSAT answer proves, not in how the question is asked.
fn probe_pair(
    solver: &mut ProbeSolver<'_>,
    meter: &mut super::meter::PreprocessMeter,
    a: i32,
    b: i32,
) -> Status {
    solver.assume(a);
    solver.assume(b);
    if let Some(cap) = meter.equivalence_conflict_cap() {
        solver.limit(c"conflicts", cap);
    }
    meter.solve(PreprocessPhase::Equivalence, solver)
}

/// A probe came back satisfiable, so the class being probed is not one class:
/// split it against the model. The candidates from `i` on that still agree with
/// the representative stay in `remaining`, the rest leave as a class of their
/// own. `rep_true` is how the model assigned the representative, which is what
/// agreement is judged against. The class being probed is out of the partition,
/// so its candidates are read from the model alongside the partition's.
fn refine_against_model(
    partition: &mut Partition,
    solver: &mut ProbeSolver<'_>,
    model: &mut [i32],
    remaining: &mut Vec<i32>,
    i: usize,
    rep_true: bool,
) {
    let held = partition.classes.iter().flatten().chain(&remaining[i..]);
    refresh_model(solver, held.copied(), model);
    partition.observe_model(model);
    let (stay, split) = refine_candidates(&remaining[i..], model, rep_true);
    remaining.truncate(i);
    remaining.extend(stay);
    if split.len() >= 2 {
        partition.classes.push(split);
    }
}

/// Map a literal of the loaded formula through the post-backbone Tarjan pass's
/// `EquivMapping` to its representative literal. `None` mapping = identity.
fn map_lit(lit: Literal, mapping: &Option<EquivMapping>) -> Literal {
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
    /// The single solver: formula loaded once, seed-solved once.
    solver: CaDiCal,
    /// The loaded formula's occurring variables, renumbered to the compact
    /// space the solver and the partition work in.
    vars: Renumber,
    /// Variable count of the compact space.
    num_vars: usize,
    /// Clause frequency per compact variable, for the backbone candidate
    /// ordering.
    freq: Vec<u32>,
    /// The latest model, indexed by compact variable. The seed fills it whole;
    /// each counter-model after that refreshes only the variables the
    /// partition still holds, which are the only ones read from it.
    model: Vec<i32>,
    /// The partition and everything probing has confirmed about it.
    pub(super) partition: Partition,
}

/// The literal partition and the facts probing has established over it.
///
/// Refining it is pure bookkeeping — no solver, no `unsafe` — so it is separate
/// from the session that produces the models: the refinement rules are testable
/// on their own, and a probing loop can rewrite the partition while the solver
/// is bounded by a terminator guard that borrows the session. Every literal in
/// it is in the engine's compact space.
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
    /// Confirmed equivalences as raw DIMACS pairs (`a ≡ b`).
    confirmed_equiv: Vec<(i32, i32)>,
    /// Per variable: its literal has left the partition for good — confirmed
    /// backbone, flippable in the seed, or substituted away by the
    /// post-backbone Tarjan pass. A removal filters a class by one lookup per
    /// literal here.
    retired: Vec<bool>,
}

impl Partition {
    /// An empty partition over `num_vars` variables: nothing seeded, no
    /// distinguished ⊤-class.
    pub(super) fn new(num_vars: usize) -> Self {
        Partition {
            classes: Vec::new(),
            top: usize::MAX,
            seeded: false,
            confirmed_backbone: Vec::new(),
            confirmed_equiv: Vec::new(),
            retired: vec![false; num_vars],
        }
    }

    /// Refine every class by `model`'s bit: split each into (true-in-model,
    /// false-in-model). The ⊤-class's true-half stays the ⊤-class (its all-ones
    /// value-vector continues); its false-half becomes a new class. Non-⊤ classes
    /// keep only halves of size ≥ 2 (singletons can no longer yield equivalences).
    /// The true half stays in its class's own buffer. Reads `model` only at the
    /// partition's literals; O(partition), no SAT.
    pub(super) fn observe_model(&mut self, model: &[i32]) {
        let top = self.top;
        let old = std::mem::take(&mut self.classes);
        self.classes.reserve(old.len());
        let mut new_top = usize::MAX;
        for (ci, mut t_half) in old.into_iter().enumerate() {
            let mut f_half = Vec::new();
            t_half.retain(|&lit| {
                let stays = lit_true_in_model(lit, model);
                if !stays {
                    f_half.push(lit);
                }
                stays
            });
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

    /// Retire `lits` and drop them from the ⊤-class in place, preserving class
    /// indices. Every literal retired this way is a ⊤-class member: the seed
    /// harvest retires literals of the seed model, whose one class is the
    /// ⊤-class, and a confirmed backbone literal is true in every model, so no
    /// counter-model has split it out. One pass over the ⊤-class, none over the
    /// rest of the partition.
    fn retire_from_top(&mut self, lits: &[i32]) {
        for &lit in lits {
            self.retired[VarId::from_dimacs(lit).idx()] = true;
        }
        let retired = &self.retired;
        let top = &mut self.classes[self.top];
        let before = top.len();
        top.retain(|&l| !retired[VarId::from_dimacs(l).idx()]);
        debug_assert_eq!(
            before - top.len(),
            lits.len(),
            "a retired literal was not a member of the ⊤-class"
        );
    }

    /// Retire `vars` and drop their literals from whichever classes hold them,
    /// in place, preserving class indices.
    fn retire_vars(&mut self, vars: &[VarId]) {
        for var in vars {
            self.retired[var.idx()] = true;
        }
        let retired = &self.retired;
        for class in &mut self.classes {
            class.retain(|&l| !retired[VarId::from_dimacs(l).idx()]);
        }
    }
}

impl ProbeEngine {
    /// Load `formula` into a fresh CaDiCaL session. No solve yet — the seed
    /// solve happens in [`ProbeEngine::run_backbone_with_meter`] so seed +
    /// probing share one budget window.
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
        let declared_freq = occ::frequency(formula.clauses(), formula.num_vars() as usize);
        let vars = Renumber::keeping(declared_freq.len(), |v| declared_freq[v.idx()] > 0);
        for clause in formula.clauses() {
            for &lit in &clause.literals {
                let lit = vars
                    .apply_lit(lit)
                    .expect("a variable a clause mentions occurs in the formula");
                solver.add(lit.to_dimacs());
            }
            solver.add(0);
        }
        solver.limit(c"conflicts", RUN_TO_ANSWER_CONFLICTS);
        let freq: Vec<u32> = vars.kept().iter().map(|v| declared_freq[v.idx()]).collect();
        Some(ProbeEngine {
            solver,
            num_vars: freq.len(),
            vars,
            model: Vec::new(),
            partition: Partition::new(freq.len()),
            freq,
        })
    }

    /// Phase-2: seed solve + backbone probing on the ⊤-class. Returns the
    /// [`BackboneResult`] shape the pipeline's shared stats code prints. Every
    /// SAT counter-model is routed through `Partition::observe_model`, refining
    /// the equivalence classes for free (win #1). Confirmed literals are pinned
    /// as units in the shared solver.
    pub(super) fn run_backbone_with_meter(
        &mut self,
        budget: Duration,
        meter: &mut super::meter::PreprocessMeter,
    ) -> BackboneResult {
        let start = Instant::now();
        let mark = meter.begin(PreprocessPhase::Backbone, budget);
        let nv = self.num_vars;
        let empty = |solve_ms: u64, unsat: bool| BackboneResult {
            forced: Vec::new(),
            probes_completed: 0,
            solve_ms,
            unsat,
            fixed_found: 0,
            flippable_eliminated: 0,
            model_eliminated: 0,
            elapsed_ms: start.elapsed().as_millis() as u64,
        };
        // A formula over no variables is not solved at all. One whose variables
        // all go unmentioned still is: an empty clause refutes it.
        if self.vars.num_old_vars() == 0 {
            let result = empty(0, false);
            meter.finish_phase(mark);
            return result;
        }

        // Seed solve, bounded by the phase's own terminator.
        let mut solver = ProbeSolver::for_phase(&mut self.solver, meter, budget);
        let status = meter.solve(PreprocessPhase::Backbone, &mut solver);
        let solve_ms = start.elapsed().as_millis() as u64;
        match status {
            Status::Unsatisfiable => {
                let result = empty(solve_ms, true);
                meter.finish_phase(mark);
                return result;
            }
            Status::Satisfiable => {}
            _ => {
                let result = empty(solve_ms, false);
                meter.finish_phase(mark);
                return result;
            }
        }

        self.model = read_model(&mut solver, nv);

        // Seed the partition: one class of every assigned true-literal — this is
        // the initial ⊤-class (all-ones so far). Vars unassigned in the seed
        // (val == 0, e.g. eliminated by inprocessing) are excluded (an unassigned
        // var can't be a backbone or equivalence candidate).
        let mut top_class = Vec::new();
        for &lit in &self.model {
            if lit != 0 {
                top_class.push(lit);
            }
        }
        self.partition.classes = vec![top_class];
        self.partition.top = 0;
        self.partition.seeded = true;

        let (fixed_found, flippable_eliminated) =
            harvest_fixed_and_flippable(&mut self.partition, &mut solver, &self.model, nv);
        // A variable no clause mentions flips in every model; it is counted with
        // the flippable ones without being put to the solver.
        let unmentioned = self.vars.num_old_vars() - nv;
        let flippable_eliminated = flippable_eliminated + unmentioned;

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
            &mut self.model,
            mark,
            budget,
            meter,
        );
        let recovered = recover_deferred(
            &mut self.partition,
            &mut solver,
            &probed.deferred,
            &mut self.model,
            mark,
            budget,
            meter,
        );

        let result = BackboneResult {
            forced: self
                .partition
                .confirmed_backbone
                .iter()
                .map(|&lit| self.vars.apply_inverse_lit(lit))
                .collect(),
            probes_completed: probed.probes_completed + recovered,
            solve_ms,
            unsat: false,
            fixed_found,
            flippable_eliminated,
            model_eliminated: probed.model_eliminated,
            elapsed_ms: start.elapsed().as_millis() as u64,
        };
        meter.finish_phase(mark);
        result
    }

    /// Ingest the post-backbone Tarjan pass's substitutions as class merges
    /// (win #4): the
    /// eliminated variables no longer appear in the pipeline formula `f`, so their
    /// literals are dropped from the partition — the engine must neither probe nor
    /// emit them (they are already substituted into `f`).
    pub(super) fn ingest_tarjan_equivs(&mut self, mapping: &EquivMapping) {
        // A variable the engine was never loaded with has nothing to drop.
        let eliminated: Vec<VarId> = mapping
            .var_to_rep
            .iter()
            .enumerate()
            .filter(|&(v, rep)| rep.var.idx() != v)
            .filter_map(|(v, _)| self.vars.new_id(VarId::from_idx(v)))
            .collect();
        if !eliminated.is_empty() {
            self.partition.retire_vars(&eliminated);
        }
    }

    /// Phase-5: equivalence probing on the ALREADY-REFINED classes (pre-refined by
    /// the backbone pass's counter-models — this is where the probe-heavy
    /// instances should collapse). Largest-class-first, two-direction probes,
    /// rep-relative refinement — on the engine's shared partition, routing every
    /// counter-model through `observe_model` so it refines the OTHER classes too.
    /// Confirmed equivalences are mapped through that pass's `EquivMapping`
    /// (`mapping2`); same-representative pairs are dropped (already known — no
    /// tautology injected). Returns the [`EquivResult`] shape the pipeline's
    /// the stage's stats consume.
    pub(super) fn run_equiv_with_meter(
        &mut self,
        budget: Duration,
        mapping2: &Option<EquivMapping>,
        meter: &mut super::meter::PreprocessMeter,
    ) -> EquivResult {
        let start = Instant::now();
        let mark = meter.begin(PreprocessPhase::Equivalence, budget);
        if !self.partition.seeded {
            // No seed model (seed solve timed out / no backbone pass) — nothing to
            // probe. The engine deliberately spends exactly ONE seed solve (in
            // run_backbone) and never re-seeds here.
            let result = EquivResult {
                equivalences: Vec::new(),
                probes_completed: 0,
                elapsed_ms: start.elapsed().as_millis() as u64,
            };
            meter.finish_phase(mark);
            return result;
        }
        // The ⊤-class is no longer distinguished; all classes are treated
        // uniformly for equivalence probing.
        self.partition.top = usize::MAX;

        let mut solver = ProbeSolver::for_phase(&mut self.solver, meter, budget);
        let model = &mut self.model;

        let mut probes_completed = 0;
        // Process classes largest-first (more equivalences per probe; a failed
        // probe refines the whole partition at once).
        loop {
            if meter.elapsed(mark) >= budget {
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
                if meter.elapsed(mark) >= budget {
                    break;
                }
                let candidate = remaining[i];
                probes_completed += 1;

                // Direction 1: rep ∧ ¬candidate → UNSAT?
                match probe_pair(&mut solver, meter, rep, -candidate) {
                    Status::Satisfiable => {
                        // rep is TRUE in this model (we assumed `rep`).
                        refine_against_model(
                            &mut self.partition,
                            &mut solver,
                            model,
                            &mut remaining,
                            i,
                            true,
                        );
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
                match probe_pair(&mut solver, meter, -rep, candidate) {
                    Status::Unsatisfiable => {
                        // Confirmed: rep ↔ candidate.
                        confirmed.push(candidate);
                        remaining.remove(i);
                    }
                    Status::Satisfiable => {
                        // rep is FALSE in this model (we assumed `-rep`); refine
                        // with the rep-false criterion (guarantees progress, no
                        // livelock).
                        refine_against_model(
                            &mut self.partition,
                            &mut solver,
                            model,
                            &mut remaining,
                            i,
                            false,
                        );
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

        // Name confirmed equivalences in the loaded formula's variables, then map
        // them through the post-backbone Tarjan mapping; drop any pair whose two
        // literals collapse to the same representative — a tautology if same
        // polarity, a contradiction if opposite, either way not a new fact to
        // inject.
        let mut equivalences = Vec::new();
        for &(a, b) in &self.partition.confirmed_equiv {
            let la = map_lit(self.vars.apply_inverse_lit(Literal::from(a)), mapping2);
            let lb = map_lit(self.vars.apply_inverse_lit(Literal::from(b)), mapping2);
            if la.var == lb.var {
                continue;
            }
            equivalences.push((la, lb));
        }

        let result = EquivResult {
            equivalences,
            probes_completed,
            elapsed_ms: start.elapsed().as_millis() as u64,
        };
        meter.finish_phase(mark);
        result
    }
}

/// Confirm `lits` as backbone literals: record each one, pin it as a unit in
/// the shared solver so every later probe is strengthened by it, and drop it
/// from the partition — a confirmed literal is a constant, not a member of an
/// equivalence class. The three steps are one soundness contract, so both probe
/// loops discharge it here rather than each spelling it out.
fn confirm_backbone(partition: &mut Partition, solver: &mut ProbeSolver<'_>, lits: &[i32]) {
    for &lit in lits {
        partition.confirmed_backbone.push(Literal::from(lit));
        solver.add(lit);
        solver.add(0);
    }
    partition.retire_from_top(lits);
}

/// The two backbone harvests that cost no solve: the literals CaDiCaL has
/// already fixed, then the ones it reports individually flippable in `model`.
/// Returns `(fixed_found, flippable_eliminated)`.
fn harvest_fixed_and_flippable(
    partition: &mut Partition,
    solver: &mut ProbeSolver<'_>,
    model: &[i32],
    nv: usize,
) -> (usize, usize) {
    let mut fixed_found = 0;
    let mut flippable_eliminated = 0;

    // Harvest the backbone literals CaDiCaL already knows (free). Each
    // variable lands in `remove` at most once.
    let mut remove: Vec<i32> = Vec::new();
    for i in 0..nv {
        let dimacs = VarId::from_idx(i).to_dimacs();
        let f = solver.fixed(dimacs);
        if f != 0 {
            let lit = if f > 0 { dimacs } else { -dimacs };
            partition.confirmed_backbone.push(Literal::from(lit));
            fixed_found += 1;
            remove.push(lit);
        }
    }

    // `flippable()` harvest — a literal individually flippable in the
    // current model is not backbone AND is not equivalent to anything still in
    // the ⊤-class (flipping it alone yields a model where it disagrees). So it
    // splits out of the partition entirely (win #2).
    for (i, &val) in model.iter().enumerate() {
        if val == 0 {
            continue;
        }
        if solver.fixed(VarId::from_idx(i).to_dimacs()) != 0 {
            continue; // already fixed above
        }
        if solver.flippable(-val) {
            flippable_eliminated += 1;
            remove.push(val);
        }
    }
    if !remove.is_empty() {
        partition.retire_from_top(&remove);
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

/// SAT probing with CadiBack-style adaptive chunking (chunk_limit
/// starts at 1, grows 8× after each UNSAT burst, resets to 1 after a SAT
/// counter-model). Every counter-model refines ALL classes, not just the
/// backbone candidates, and recompacts `candidates` to those still in the
/// ⊤-class.
fn probe_loop(
    partition: &mut Partition,
    solver: &mut ProbeSolver<'_>,
    candidates: &mut Vec<i32>,
    model: &mut [i32],
    mark: super::meter::PhaseMark,
    budget: Duration,
    meter: &mut super::meter::PreprocessMeter,
) -> ProbeRun {
    let mut probes_completed = 0;
    let mut model_eliminated = 0;
    let mut chunk_limit: usize = 1;
    let mut pos = 0;
    let mut deferred: Vec<i32> = Vec::new();

    while pos < candidates.len() {
        if meter.elapsed(mark) >= budget {
            break;
        }
        let remaining = candidates.len() - pos;
        let chunk_size = chunk_limit.min(remaining);
        probes_completed += 1;

        let probe = if chunk_size == 1 {
            solver.limit(c"conflicts", MAX_CONFLICTS);
            solver.assume(-candidates[pos]);
            meter.solve(PreprocessPhase::Backbone, solver)
        } else {
            solver.limit(c"conflicts", RUN_TO_ANSWER_CONFLICTS);
            for &cand in &candidates[pos..pos + chunk_size] {
                solver.constrain(-cand);
            }
            solver.constrain(0);
            meter.solve(PreprocessPhase::Backbone, solver)
        };

        match probe {
            Status::Unsatisfiable => {
                // All candidates in this chunk are backbone.
                confirm_backbone(partition, solver, &candidates[pos..pos + chunk_size]);
                pos += chunk_size;
                chunk_limit = chunk_limit.saturating_mul(8).max(1);
            }
            Status::Satisfiable => {
                // Counter-model: refine ALL classes (win #1), then recompact
                // the candidate list to those still in the ⊤-class (drop any
                // var the counter-model just proved non-backbone). The
                // candidates are ⊤-class members and the ⊤-class keeps exactly
                // its members true in the model, so the model says which stay.
                refresh_model(solver, partition.classes.iter().flatten().copied(), model);
                partition.observe_model(model);
                let mut write = pos;
                for read in pos..candidates.len() {
                    if lit_true_in_model(candidates[read], model) {
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
                if meter.elapsed(mark) < budget {
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
    solver: &mut ProbeSolver<'_>,
    deferred: &[i32],
    model: &mut [i32],
    mark: super::meter::PhaseMark,
    budget: Duration,
    meter: &mut super::meter::PreprocessMeter,
) -> usize {
    const RECOVERY_CAP: i32 = 1_000;
    let mut probes_completed = 0;
    for &lit in deferred {
        if meter.elapsed(mark) >= budget {
            break;
        }
        probes_completed += 1;
        solver.limit(c"conflicts", RECOVERY_CAP);
        solver.assume(-lit);
        match meter.solve(PreprocessPhase::Backbone, solver) {
            Status::Unsatisfiable => confirm_backbone(partition, solver, &[lit]),
            Status::Satisfiable => {
                refresh_model(solver, partition.classes.iter().flatten().copied(), model);
                partition.observe_model(model);
            }
            _ => {}
        }
    }
    probes_completed
}

// ── Tests ──────────────────────────────────────────────────────────────────
