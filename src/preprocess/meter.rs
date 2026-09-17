//! Per-chain preprocessing work meter.
//!
//! The meter is an owned value created by one simplify chain and threaded
//! through every covered preprocessing call. It is deliberately neither
//! process-global nor thread-local: two callers in one process may choose
//! different clock policies without sharing charges or decisions.

use std::time::{Duration, Instant};

use super::cadical_ffi::{CaDiCal, Status};
use crate::bundle::{
    PreprocessDecisionTrace, PreprocessPhase, PreprocessPhaseTrace, ProbeDecisionCounts,
};
use crate::config::PreprocessClock;

impl PreprocessPhase {
    /// Charged work units corresponding to one millisecond of this phase's
    /// existing policy allowance.
    pub(crate) const fn units_per_ms(self) -> u64 {
        match self {
            PreprocessPhase::Backbone => 12_500,
            PreprocessPhase::Equivalence => 1_900,
            PreprocessPhase::Dve => 5_300,
        }
    }

    const fn index(self) -> usize {
        match self {
            PreprocessPhase::Backbone => 0,
            PreprocessPhase::Equivalence => 1,
            PreprocessPhase::Dve => 2,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct PhaseMark {
    phase: PreprocessPhase,
    started: Instant,
    units: u64,
    budget_ms: u64,
    probes: ProbeDecisionCounts,
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
    units: u64,
    probes: [ProbeDecisionCounts; 3],
    trace: Option<PreprocessDecisionTrace>,
}

impl PreprocessMeter {
    pub(super) fn new(clock: PreprocessClock) -> Self {
        PreprocessMeter {
            clock,
            units: 0,
            probes: [ProbeDecisionCounts::default(); 3],
            trace: matches!(clock, PreprocessClock::Deterministic { .. })
                .then(PreprocessDecisionTrace::default),
        }
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
        }
    }

    /// Elapsed time in the currency this run selected.
    pub(super) fn elapsed(&self, mark: PhaseMark) -> Duration {
        if self.deterministic() {
            Duration::from_millis(self.units.saturating_sub(mark.units) / mark.phase.units_per_ms())
        } else {
            mark.started.elapsed()
        }
    }

    pub(super) fn elapsed_ms(&self, mark: PhaseMark) -> u64 {
        self.elapsed(mark).as_millis().min(u64::MAX as u128) as u64
    }

    pub(super) fn finish_phase(&mut self, mark: PhaseMark) {
        let Some(trace) = self.trace.as_mut() else {
            return;
        };
        let after = self.probes[mark.phase.index()];
        trace.phases.push(PreprocessPhaseTrace {
            phase: mark.phase,
            budget_ms: mark.budget_ms,
            budget_units: mark.budget_ms.saturating_mul(mark.phase.units_per_ms()),
            spent_units: self.units.saturating_sub(mark.units),
            probes: probe_delta(after, mark.probes),
        });
    }

    /// Remove a wall terminator only under deterministic preprocessing.
    pub(super) fn deadline_or_none(&self, deadline: Option<Instant>) -> Option<Instant> {
        if self.deterministic() { None } else { deadline }
    }

    /// The equivalence solve cap that replaces its wall terminator under
    /// deterministic preprocessing: the same per-probe ceiling backbone's
    /// single-literal probes run under.
    pub(super) fn equivalence_conflict_cap(&self) -> Option<i32> {
        self.deterministic()
            .then_some(super::probe_engine::MAX_CONFLICTS)
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

    /// Solve and charge the propagation delta only in deterministic mode.
    pub(super) fn solve(&mut self, phase: PreprocessPhase, solver: &mut CaDiCal) -> Status {
        if !self.deterministic() {
            return solver.solve();
        }
        let before = solver.search_stats();
        let status = solver.solve();
        self.charge(solver.search_stats().since(before).propagations.max(0) as u64);
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
        if let Some(trace) = self.trace.as_mut() {
            trace.total_units = self.units;
        }
        self.trace
    }
}

fn probe_delta(after: ProbeDecisionCounts, before: ProbeDecisionCounts) -> ProbeDecisionCounts {
    ProbeDecisionCounts {
        completed: after.completed.saturating_sub(before.completed),
        satisfiable: after.satisfiable.saturating_sub(before.satisfiable),
        unsatisfiable: after.unsatisfiable.saturating_sub(before.unsatisfiable),
        unknown: after.unknown.saturating_sub(before.unknown),
    }
}
