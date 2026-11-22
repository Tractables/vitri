//! Per-chain preprocessing work meter.
//!
//! The meter is an owned value created by one simplify chain and threaded
//! through every covered preprocessing call. It is deliberately neither
//! process-global nor thread-local: two callers in one process may choose
//! different clock policies without sharing charges or decisions.

use std::cell::Cell;
use std::time::{Duration, Instant};

use super::cadical_ffi::{CaDiCal, SearchStats, Status};
use crate::bundle::{
    PreprocessDecisionTrace, PreprocessPhase, PreprocessPhaseTrace, ProbeDecisionCounts,
};
use crate::config::PreprocessClock;

impl PreprocessPhase {
    /// Charged work units corresponding to one millisecond of this phase's
    /// existing policy allowance.
    ///
    /// Every phase has one rate. Each phase charges its solver calls with its
    /// own [`charge_weights`](Self::charge_weights), set so that a unit costs
    /// about the same wall in every phase. The rate is what an otherwise idle
    /// machine does per millisecond, so on one a phase's wall is about its
    /// allowance.
    pub(crate) const fn units_per_ms(self) -> u64 {
        15_900
    }

    /// The weights [`solver_units`] charges this phase's solver calls with.
    const fn charge_weights(self) -> ChargeWeights {
        match self {
            PreprocessPhase::Backbone | PreprocessPhase::Equivalence | PreprocessPhase::Dve => {
                PROBE_WEIGHTS
            }
            PreprocessPhase::Implicates => PASS_WEIGHTS,
        }
    }

    const fn index(self) -> usize {
        match self {
            PreprocessPhase::Backbone => 0,
            PreprocessPhase::Equivalence => 1,
            PreprocessPhase::Dve => 2,
            PreprocessPhase::Implicates => 3,
        }
    }
}

/// What a solver call asks of the solver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SolverCall {
    /// A search to a model, a refutation or a limit.
    Solve,
    /// Propagation of the assumptions alone.
    Propagate,
}

/// What a solver call is charged, in tenths of a work unit, for each count of
/// its search and for the call itself. Each term is rounded down to whole
/// units on its own.
#[derive(Clone, Copy)]
struct ChargeWeights {
    /// Per propagation.
    propagation: u64,
    /// Per search tick. A propagation costs more wall the longer the watch
    /// lists it walks, which the ticks count and the propagations do not.
    tick: u64,
    /// Per conflict: analysing it and learning its clause.
    conflict: u64,
    /// Per solve call: starting and ending a search.
    solve: u64,
    /// Per propagate call.
    propagate: u64,
}

/// The implied-clause pass's weights, fitted to the wall of its solver calls:
/// half a unit per propagation, three tenths per tick, 77 per conflict, and
/// 230 per solve or 1 per propagate call.
const PASS_WEIGHTS: ChargeWeights = ChargeWeights {
    propagation: 5,
    tick: 3,
    conflict: 770,
    solve: 2_300,
    propagate: 10,
};

/// Backbone, equivalence and DVE weights: a unit per propagation, a tenth per
/// tick, 200 per conflict (about the wall of two hundred propagations) and
/// nothing per call.
const PROBE_WEIGHTS: ChargeWeights = ChargeWeights {
    propagation: 10,
    tick: 1,
    conflict: 2_000,
    solve: 0,
    propagate: 0,
};

/// The work units a solver call of `phase` is charged for what its search did,
/// with the phase's [`ChargeWeights`].
fn solver_units(phase: PreprocessPhase, call: SolverCall, done: SearchStats) -> u64 {
    let w = phase.charge_weights();
    let units = |n: i64, tenths: u64| (n.max(0) as u64).saturating_mul(tenths) / 10;
    let per_call = match call {
        SolverCall::Solve => w.solve,
        SolverCall::Propagate => w.propagate,
    };
    units(done.propagations, w.propagation)
        .saturating_add(units(done.search_ticks, w.tick))
        .saturating_add(units(done.conflicts, w.conflict))
        .saturating_add(per_call / 10)
}

/// How many times its nominal allowance a deterministic phase may run in wall
/// time before it is stopped with work left. The charge is an estimate of the
/// wall a phase takes on an idle machine, and even there some phases run about
/// twice their charge; past three times a phase is a runaway. An Arjun
/// reduction on the work clock is stopped at the same multiple.
pub(super) const RUNAWAY_FACTOR: u32 = 3;

#[derive(Clone, Copy)]
pub(super) struct PhaseMark {
    phase: PreprocessPhase,
    started: Instant,
    units: u64,
    budget_ms: u64,
    probes: ProbeDecisionCounts,
    caps: u32,
    solver: SolverWork,
}

/// What the solver calls of a deterministic chain did, per phase, for the
/// diagnostic line [`PreprocessMeter::finish_phase`] prints: the counts a
/// charge could be made of, beside the wall they took.
#[derive(Clone, Copy, Default)]
struct SolverWork {
    calls: u64,
    wall: Duration,
    stats: SearchStats,
}

impl SolverWork {
    fn add(&mut self, wall: Duration, done: SearchStats) {
        let s = &mut self.stats;
        self.calls += 1;
        self.wall += wall;
        s.conflicts += done.conflicts;
        s.decisions += done.decisions;
        s.propagations += done.propagations;
        s.learned_literals += done.learned_literals;
        s.inprocessing_propagations += done.inprocessing_propagations;
        s.subsumption_checks += done.subsumption_checks;
        s.resolvents += done.resolvents;
        s.search_ticks += done.search_ticks;
    }
}

/// The deterministic decisions made by one DVE invocation before they are
/// aggregated into the chain trace.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct DvePassDecisions {
    pub rounds: usize,
    pub aggressive_passes: usize,
    pub defined_eliminated: usize,
    pub equivalence_eliminated: usize,
    pub budget_hit: bool,
}

pub(super) struct PreprocessMeter {
    clock: PreprocessClock,
    /// The run's wall deadline, which stops a deterministic chain even with
    /// work left; see [`Self::with_backstop`].
    backstop: Option<Instant>,
    units: u64,
    probes: [ProbeDecisionCounts; 4],
    /// Per phase, how often [`Self::elapsed`] read an allowance as spent
    /// because the phase's wall reached its nominal allowance first.
    caps: Cell<[u32; 4]>,
    solver: [SolverWork; 4],
    trace: Option<PreprocessDecisionTrace>,
}

impl PreprocessMeter {
    pub(super) fn new(clock: PreprocessClock) -> Self {
        PreprocessMeter {
            clock,
            backstop: None,
            units: 0,
            probes: [ProbeDecisionCounts::default(); 4],
            caps: Cell::new([0; 4]),
            solver: [SolverWork::default(); 4],
            trace: matches!(clock, PreprocessClock::Deterministic { .. })
                .then(PreprocessDecisionTrace::default),
        }
    }

    /// Keep `deadline`, the run's wall deadline, as a hard stop under the
    /// deterministic clock.
    ///
    /// Charged work is only an estimate of wall time, and on some formulas a
    /// phase costs far more wall than it is charged, so work alone can carry a
    /// chain past the run's deadline. Once the deadline passes, every phase
    /// reads its allowance as spent and every solver call is stopped. A chain
    /// the backstop never reached made the same decisions it makes on any
    /// machine; the decision trace says when it was reached. Under the wall
    /// clock the deadline is already what the phases are clamped to.
    pub(super) fn with_backstop(mut self, deadline: Option<Instant>) -> Self {
        self.backstop = deadline;
        self
    }

    #[inline]
    pub(super) fn deterministic(&self) -> bool {
        matches!(self.clock, PreprocessClock::Deterministic { .. })
    }

    /// Clamp `nominal` against the appropriate enclosing wall. Wall-clock mode
    /// reads what remains at this phase's start; deterministic mode reads only
    /// the configured wall value captured in the run configuration.
    pub(super) fn clamp(&self, nominal: Duration, deadline: Option<Instant>) -> Duration {
        match self.clock {
            PreprocessClock::WallClock => deadline
                .map(|d| nominal.min(d.saturating_duration_since(Instant::now())))
                .unwrap_or(nominal),
            PreprocessClock::Deterministic { configured_wall_ms } => configured_wall_ms
                .map(|ms| nominal.min(Duration::from_millis(ms)))
                .unwrap_or(nominal),
        }
    }

    pub(super) fn begin(&self, phase: PreprocessPhase, budget: Duration) -> PhaseMark {
        PhaseMark {
            phase,
            started: Instant::now(),
            units: self.units,
            budget_ms: budget.as_millis().min(u64::MAX as u128) as u64,
            probes: self.probes[phase.index()],
            caps: self.caps.get()[phase.index()],
            solver: self.solver[phase.index()],
        }
    }

    /// Elapsed time in the currency this run selected.
    ///
    /// Under the deterministic clock that is the work charged since `mark`,
    /// with two wall stops: once the backstop has passed, and once the phase
    /// has run [`RUNAWAY_FACTOR`] times its nominal allowance in wall time
    /// before its work reached it, the allowance reads as spent. The decision
    /// trace says when the wall ended a phase.
    pub(super) fn elapsed(&self, mark: PhaseMark) -> Duration {
        if !self.deterministic() {
            return mark.started.elapsed();
        }
        if self.past_backstop() {
            return Duration::MAX;
        }
        let work = Duration::from_millis(
            self.units.saturating_sub(mark.units) / mark.phase.units_per_ms(),
        );
        if wall_first(mark, work) {
            let mut caps = self.caps.get();
            caps[mark.phase.index()] += 1;
            self.caps.set(caps);
            return Duration::MAX;
        }
        work
    }

    /// Whether the run's wall deadline kept by [`Self::with_backstop`] has
    /// passed.
    pub(super) fn past_backstop(&self) -> bool {
        self.backstop.is_some_and(|d| Instant::now() >= d)
    }

    pub(super) fn elapsed_ms(&self, mark: PhaseMark) -> u64 {
        self.elapsed(mark).as_millis().min(u64::MAX as u128) as u64
    }

    pub(super) fn finish_phase(&mut self, mark: PhaseMark) {
        let Some(trace) = self.trace.as_mut() else {
            return;
        };
        let index = mark.phase.index();
        let after = self.probes[index];
        let spent_units = self.units.saturating_sub(mark.units);
        let work = Duration::from_millis(spent_units / mark.phase.units_per_ms());
        // A solver call the cap cut short is caught here when no allowance
        // check followed it.
        let wall_capped = self.caps.get()[index] > mark.caps || wall_first(mark, work);
        trace.phases.push(PreprocessPhaseTrace {
            phase: mark.phase,
            budget_ms: mark.budget_ms,
            budget_units: mark.budget_ms.saturating_mul(mark.phase.units_per_ms()),
            spent_units,
            probes: probe_delta(after, mark.probes),
            wall_capped,
        });
        let solver = self.solver[index];
        let s = solver.stats;
        let b = mark.solver.stats;
        crate::diagnostics::diag!(
            "[meter] phase={:?} budget_ms={} wall_ms={} charged_units={} capped={} solver_calls={} \
             solver_wall_ms={} propagations={} conflicts={} decisions={} learned_literals={} \
             inprocessing_propagations={} subsumption_checks={} resolvents={} search_ticks={}",
            mark.phase,
            mark.budget_ms,
            mark.started.elapsed().as_millis(),
            spent_units,
            wall_capped,
            solver.calls - mark.solver.calls,
            (solver.wall - mark.solver.wall).as_millis(),
            s.propagations - b.propagations,
            s.conflicts - b.conflicts,
            s.decisions - b.decisions,
            s.learned_literals - b.learned_literals,
            s.inprocessing_propagations - b.inprocessing_propagations,
            s.subsumption_checks - b.subsumption_checks,
            s.resolvents - b.resolvents,
            s.search_ticks - b.search_ticks,
        );
    }

    /// The wall instant a solver call of a phase that began at `started` with
    /// a `nominal` allowance is stopped at: the end of that allowance under the
    /// wall clock, its runaway stop (see [`Self::elapsed`]) or the backstop,
    /// whichever comes first, under the deterministic clock.
    pub(super) fn phase_stop(&self, started: Instant, nominal: Duration) -> Option<Instant> {
        let wall = if self.deterministic() {
            runaway_wall(nominal)
        } else {
            nominal
        };
        self.wall_stop(started.checked_add(wall))
    }

    /// The wall instant a solver call is stopped at: `deadline`, and under
    /// the deterministic clock the backstop too if it comes first.
    pub(super) fn wall_stop(&self, deadline: Option<Instant>) -> Option<Instant> {
        if self.deterministic() {
            crate::budget::earliest(self.backstop, deadline)
        } else {
            deadline
        }
    }

    #[inline]
    pub(super) fn charge(&mut self, units: u64) {
        if self.deterministic() {
            self.units = self.units.saturating_add(units);
        }
    }

    #[inline]
    pub(super) fn charge_scan(&mut self, literals: impl FnOnce() -> usize) {
        if self.deterministic() {
            self.charge(literals() as u64);
        }
    }

    /// Run one solver call, a solve or a propagation, and, in deterministic
    /// mode only, charge [`solver_units`] for what the search did.
    pub(super) fn call(
        &mut self,
        phase: PreprocessPhase,
        solver: &mut CaDiCal,
        call: SolverCall,
    ) -> Status {
        let run = |solver: &mut CaDiCal| match call {
            SolverCall::Solve => solver.solve(),
            SolverCall::Propagate => solver.propagate(),
        };
        if !self.deterministic() {
            return run(solver);
        }
        let before = solver.search_stats();
        let started = Instant::now();
        let status = run(solver);
        let done = solver.search_stats().since(before);
        self.solver[phase.index()].add(started.elapsed(), done);
        self.charge(solver_units(phase, call, done));
        status
    }

    /// Solve and, in deterministic mode only, charge [`solver_units`] for what
    /// the search did.
    pub(super) fn solve(&mut self, phase: PreprocessPhase, solver: &mut CaDiCal) -> Status {
        if !self.deterministic() {
            return solver.solve();
        }
        let status = self.call(phase, solver, SolverCall::Solve);
        let counts = &mut self.probes[phase.index()];
        counts.completed += 1;
        match status {
            Status::Satisfiable => counts.satisfiable += 1,
            Status::Unsatisfiable => counts.unsatisfiable += 1,
            Status::Unknown => counts.unknown += 1,
        }
        status
    }

    /// Simplify and charge search propagations plus `literals * rounds` for
    /// inprocessing/vivification, whose work is not in CaDiCaL search stats.
    pub(super) fn simplify(
        &mut self,
        solver: &mut CaDiCal,
        rounds: i32,
        literals: impl FnOnce() -> usize,
    ) -> Status {
        if !self.deterministic() {
            return solver.simplify(rounds);
        }
        let before = solver.search_stats();
        let status = solver.simplify(rounds);
        let searched = solver.search_stats().since(before).propagations.max(0) as u64;
        self.charge(
            searched.saturating_add((literals() as u64).saturating_mul(rounds.max(1) as u64)),
        );
        status
    }

    pub(super) fn record_dve(&mut self, pass: DvePassDecisions) {
        let Some(trace) = self.trace.as_mut() else {
            return;
        };
        trace.dve.rounds = trace.dve.rounds.saturating_add(pass.rounds);
        trace.dve.aggressive_passes = trace
            .dve
            .aggressive_passes
            .saturating_add(pass.aggressive_passes);
        trace.dve.defined_eliminated = trace
            .dve
            .defined_eliminated
            .saturating_add(pass.defined_eliminated);
        trace.dve.equivalence_eliminated = trace
            .dve
            .equivalence_eliminated
            .saturating_add(pass.equivalence_eliminated);
        trace.dve.budget_hit |= pass.budget_hit;
    }

    pub(super) fn into_trace(mut self) -> Option<PreprocessDecisionTrace> {
        let backstop_reached = self.past_backstop();
        if let Some(trace) = self.trace.as_mut() {
            trace.total_units = self.units;
            trace.backstop_reached = backstop_reached;
        }
        self.trace
    }
}

/// The wall a deterministic phase with a `nominal` allowance may take.
fn runaway_wall(nominal: Duration) -> Duration {
    nominal.saturating_mul(RUNAWAY_FACTOR)
}

/// Whether the phase took its runaway wall before `work`, its charged work,
/// reached its nominal allowance. Never for the clause pass, whose solver
/// calls are each bounded by a conflict cap and whose share of the allowance
/// can be a few milliseconds.
fn wall_first(mark: PhaseMark, work: Duration) -> bool {
    let nominal = Duration::from_millis(mark.budget_ms);
    mark.phase != PreprocessPhase::Implicates
        && work < nominal
        && mark.started.elapsed() >= runaway_wall(nominal)
}

fn probe_delta(after: ProbeDecisionCounts, before: ProbeDecisionCounts) -> ProbeDecisionCounts {
    ProbeDecisionCounts {
        completed: after.completed.saturating_sub(before.completed),
        satisfiable: after.satisfiable.saturating_sub(before.satisfiable),
        unsatisfiable: after.unsatisfiable.saturating_sub(before.unsatisfiable),
        unknown: after.unknown.saturating_sub(before.unknown),
    }
}
