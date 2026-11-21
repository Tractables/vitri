//! Models of a formula drawn with a SAT solver, and minimal repairs of them.
//!
//! [`draw`] solves the formula repeatedly in one CaDiCaL session, each time
//! with every variable's decision phase set at random, and keeps the models in
//! a [`ModelMatrix`]. [`repairs`] takes a model m and a variable u and finds a
//! set D of other variables such that m with u and D flipped is again a model,
//! with no proper subset of D doing the same.
//!
//! Both are anytime: they stop when their [`Allowance`] is spent, in the
//! currency of the run's preprocessing clock, or at its [`wall_stop`], and
//! return what they have.

use std::ffi::CStr;
use std::time::{Duration, Instant};

use rand::rngs::SmallRng;
use rand::{RngExt, SeedableRng};

use super::cadical_ffi::{Bounded, CaDiCal, Status, Terminator};
use super::implicates::Allowance;
use super::meter::{PhaseMark, PreprocessMeter};
use crate::bundle::PreprocessPhase;
use crate::cnf::CnfFormula;

/// The conflict cap of each solver call of [`draw`]. The allowance's
/// [`wall_stop`] also stops a call.
const DRAW_CONFLICTS: i32 = 100_000;
/// From this many calls on, a draw ends, out of models, once the mean charge
/// of its calls after the first, times the models still asked, is more than
/// the allowance has left. The first call of a session searches from scratch
/// and costs more than the rest.
const PROJECT_FROM: usize = 8;

/// CaDiCaL's lucky phases try fixed assignments (all false, all true and the
/// like) before any search, and return the same model whatever phases were
/// set. Neither the draws nor the repairs want them.
const NO_LUCKY: (&CStr, i32) = (c"lucky", 0);

/// Models over the same variables, one row of bits per model: variable `v`
/// (0-based) is bit `v % 64` of word `v / 64`, set when it is true.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModelMatrix {
    vars: usize,
    stride: usize,
    rows: usize,
    bits: Vec<u64>,
}

impl ModelMatrix {
    /// No models over `vars` variables.
    pub(crate) fn new(vars: usize) -> Self {
        ModelMatrix {
            vars,
            stride: vars.div_ceil(64),
            rows: 0,
            bits: Vec::new(),
        }
    }

    pub(crate) fn vars(&self) -> usize {
        self.vars
    }

    pub(crate) fn len(&self) -> usize {
        self.rows
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rows == 0
    }

    /// Row `row`'s words.
    pub(crate) fn row(&self, row: usize) -> &[u64] {
        &self.bits[row * self.stride..(row + 1) * self.stride]
    }

    /// The value of variable `var` in model `row`.
    pub(crate) fn get(&self, row: usize, var: usize) -> bool {
        self.row(row)[var / 64] >> (var % 64) & 1 == 1
    }

    /// Append a model given as one value per variable.
    pub(crate) fn push(&mut self, values: impl IntoIterator<Item = bool>) {
        let start = self.bits.len();
        self.bits.resize(start + self.stride, 0);
        let mut set = 0;
        for (v, value) in values.into_iter().enumerate() {
            if value {
                self.bits[start + v / 64] |= 1 << (v % 64);
            }
            set = v + 1;
        }
        assert_eq!(set, self.vars, "a model gives one value per variable");
        self.rows += 1;
    }

    /// Each model as one value per variable.
    pub(crate) fn to_rows(&self) -> Vec<Vec<bool>> {
        (0..self.rows)
            .map(|r| (0..self.vars).map(|v| self.get(r, v)).collect())
            .collect()
    }

    fn push_solver_model(&mut self, solver: &mut CaDiCal) {
        let vars = self.vars as i32;
        self.push((1..=vars).map(|v| solver.val(v) > 0));
    }
}

/// What `allowance` comes to in its clock's currency: the nominal allowance,
/// clamped to the run's deadline (under the wall clock, to what is left of
/// it now).
pub(crate) fn limit(allowance: &Allowance) -> Duration {
    PreprocessMeter::new(allowance.clock).clamp(allowance.nominal, allowance.deadline)
}

/// The wall instant work begun at `started` within `allowance` stops at.
/// Under the wall clock that is the end of the allowance. Charged work is
/// only an estimate of wall time: on a formula whose solver calls each run to
/// their conflict cap, a call costs far more wall than its propagations are
/// charged. So under charged work it stops at the phases' runaway wall
/// ([`PreprocessMeter::phase_stop`]) or at the allowance's deadline, whichever
/// comes first. Work the stop never reached is the same on every machine.
pub(crate) fn wall_stop(allowance: &Allowance, started: Instant) -> Option<Instant> {
    PreprocessMeter::new(allowance.clock)
        .with_backstop(allowance.deadline)
        .phase_stop(started, limit(allowance))
}

/// The phase whose rate the draws and repairs are charged at.
const CHARGED_AS: PreprocessPhase = PreprocessPhase::Implicates;

/// What the allowance has been charged, and when it is spent.
struct Budget {
    meter: PreprocessMeter,
    mark: PhaseMark,
    limit: Duration,
    wall: Option<Instant>,
    /// Solver calls made.
    calls: u64,
}

impl Budget {
    fn new(allowance: &Allowance) -> Self {
        let meter = PreprocessMeter::new(allowance.clock);
        let limit = limit(allowance);
        let mark = meter.begin(CHARGED_AS, limit);
        Budget {
            meter,
            mark,
            limit,
            wall: wall_stop(allowance, Instant::now()),
            calls: 0,
        }
    }

    fn used(&self) -> Duration {
        self.meter.elapsed(self.mark)
    }

    fn out(&self) -> bool {
        self.used() >= self.limit || self.wall.is_some_and(|w| Instant::now() >= w)
    }

    /// Stops a solver call at the allowance's [`wall_stop`].
    fn stop(&self) -> Deadline {
        Deadline(self.wall)
    }

    /// Solve under the solver's assumptions, charging the search's work
    /// ([`PreprocessMeter::solve`]) and the `assumed` literals.
    fn solve(&mut self, solver: &mut CaDiCal, assumed: usize) -> Status {
        self.calls += 1;
        let status = self.meter.solve(CHARGED_AS, solver);
        self.meter.charge(assumed as u64);
        status
    }
}

struct Deadline(Option<Instant>);

impl Terminator for Deadline {
    fn terminated(&mut self) -> bool {
        self.0.is_some_and(|d| Instant::now() >= d)
    }
}

fn solver_for(formula: &CnfFormula, options: &[(&CStr, i32)]) -> Option<CaDiCal> {
    let mut solver = CaDiCal::new()?;
    for &(name, val) in options {
        assert!(
            solver.set_option(name, val),
            "CaDiCaL has no option {name:?}"
        );
    }
    solver.reserve(formula.num_vars() as i32);
    for clause in formula.clauses() {
        for lit in &clause.literals {
            solver.add(lit.to_dimacs());
        }
        solver.add(0);
    }
    Some(solver)
}

/// The models one call of [`draw`] found.
pub(crate) struct Draw {
    pub models: ModelMatrix,
    /// The formula has no model.
    pub unsatisfiable: bool,
    /// The allowance ran out, or was projected to, before `count` models.
    pub ran_out: bool,
    /// The projection ended the draw ([`PROJECT_FROM`]).
    pub projected: bool,
    /// Calls stopped by the conflict cap.
    pub capped: usize,
    /// What the draw spent, in the clock's currency.
    pub spent: Duration,
}

/// Up to `count` models of `formula`, each from one solve in a single CaDiCaL
/// session with every decision phase set from a generator seeded with `seed`.
/// `then` more models will be asked of a later draw at about the same cost,
/// and the projection that ends the draw early ([`PROJECT_FROM`]) counts them.
///
/// Models may repeat. A call stopped by its conflict cap gives no model and
/// the draw goes on; the allowance ends it early.
pub(crate) fn draw(
    formula: &CnfFormula,
    count: usize,
    then: usize,
    seed: u64,
    allowance: &Allowance,
) -> Draw {
    let n = formula.num_vars() as usize;
    let mut out = Draw {
        models: ModelMatrix::new(n),
        unsatisfiable: false,
        ran_out: false,
        projected: false,
        capped: 0,
        spent: Duration::ZERO,
    };
    let solver_seed = (seed % 2_000_000_000) as i32;
    let Some(mut solver) = solver_for(formula, &[NO_LUCKY, (c"seed", solver_seed)]) else {
        return out;
    };
    let mut budget = Budget::new(allowance);
    let mut rng = SmallRng::seed_from_u64(seed);
    let mut first = Duration::ZERO;
    for call in 1..=count {
        if budget.out() {
            break;
        }
        for v in 1..=n as i32 {
            solver.phase(if rng.random::<bool>() { v } else { -v });
        }
        budget.meter.charge(n as u64);
        let mut bounded = Bounded::new(&mut solver, budget.stop());
        bounded.limit(c"conflicts", DRAW_CONFLICTS);
        match budget.solve(&mut bounded, 0) {
            Status::Satisfiable => out.models.push_solver_model(&mut bounded),
            Status::Unsatisfiable => {
                out.unsatisfiable = true;
                break;
            }
            Status::Unknown => out.capped += 1,
        }
        let used = budget.used();
        if call == 1 {
            first = used;
        } else if call >= PROJECT_FROM && call < count {
            let per = used.saturating_sub(first) / (call as u32 - 1);
            let asked = (count - call + then) as u32;
            if per.saturating_mul(asked) > budget.limit.saturating_sub(used) {
                out.projected = true;
                break;
            }
        }
    }
    out.ran_out = out.models.len() < count && !out.unsatisfiable && (budget.out() || out.projected);
    out.spent = budget.used();
    out
}

/// How a repair of one model and variable came out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Repair {
    /// No model gives the variable its other value.
    Backbone,
    /// The first call was stopped by its conflict cap or the allowance.
    Unknown,
    /// An inclusion-minimal repair: the other variables to flip, ascending.
    Minimal(Vec<u32>),
    /// A repair some call of the minimisation could not settle, so a proper
    /// subset of it may also be one.
    Uncertified(Vec<u32>),
}

/// The repairs one call of [`RepairSession::run`] found.
pub(crate) struct Repairs {
    /// One entry per (model, variable) pair handled, in the order given.
    pub found: Vec<Repair>,
    /// The allowance ran out before every pair was handled.
    pub ran_out: bool,
    /// What the repairs spent, in the clock's currency.
    pub spent: Duration,
    /// Solver calls made, the first call of each pair among them.
    pub calls: u64,
}

/// A CaDiCaL session that finds, for each `(model, var)` pair, a minimal set
/// D of other variables such that the model with `var` and D flipped
/// satisfies the formula.
///
/// The decision phases are set to the pair's model. The first call assumes
/// `var` flipped; its model gives a repair D. Each variable of D is then
/// tried at the model's value, every variable outside D assumed there too,
/// one call each and in an order drawn from the session's seed; a model found
/// shrinks D to the variables it still flips. A call that ends neither way
/// leaves D [`Repair::Uncertified`]. Every call is capped at the session's
/// conflicts.
///
/// The rows the pairs name must be models of the formula. The session stays
/// open, so its pairs can be handed over in parts: the parts in turn give
/// what one call on all of them gives.
pub(crate) struct RepairSession<'a> {
    models: &'a ModelMatrix,
    solver: CaDiCal,
    conflicts: i32,
    rng: SmallRng,
    /// The model the phases are set to, and its literals.
    phased: Option<usize>,
    lits: Vec<i32>,
    in_d: Vec<bool>,
}

impl<'a> RepairSession<'a> {
    /// A session over `formula` and its `models`, or none when the solver
    /// cannot be made.
    pub(crate) fn new(
        formula: &CnfFormula,
        models: &'a ModelMatrix,
        conflicts: i32,
        seed: u64,
    ) -> Option<Self> {
        let n = formula.num_vars() as usize;
        assert_eq!(models.vars(), n, "models over the formula's variables");
        Some(RepairSession {
            models,
            solver: solver_for(formula, &[NO_LUCKY])?,
            conflicts,
            rng: SmallRng::seed_from_u64(seed),
            phased: None,
            lits: Vec::new(),
            in_d: vec![false; n],
        })
    }

    /// The repairs of `pairs`, within `allowance`.
    pub(crate) fn run(&mut self, pairs: &[(usize, u32)], allowance: &Allowance) -> Repairs {
        let n = self.models.vars();
        let mut out = Repairs {
            found: Vec::with_capacity(pairs.len()),
            ran_out: false,
            spent: Duration::ZERO,
            calls: 0,
        };
        let mut budget = Budget::new(allowance);
        for &(row, u) in pairs {
            if budget.out() {
                out.ran_out = true;
                break;
            }
            if self.phased != Some(row) {
                let models = self.models;
                self.lits = (0..n)
                    .map(|v| {
                        if models.get(row, v) {
                            v as i32 + 1
                        } else {
                            -(v as i32 + 1)
                        }
                    })
                    .collect();
                for &l in &self.lits {
                    self.solver.phase(l);
                }
                budget.meter.charge(n as u64);
                self.phased = Some(row);
            }
            let mut bounded = Bounded::new(&mut self.solver, budget.stop());
            let (repair, stopped) = repair_one(
                &mut bounded,
                &mut budget,
                &self.lits,
                u as usize,
                self.conflicts,
                &mut self.rng,
                &mut self.in_d,
            );
            out.found.push(repair);
            if stopped {
                out.ran_out = true;
                break;
            }
        }
        out.spent = budget.used();
        out.calls = budget.calls;
        out
    }
}

/// One pair of [`repairs`]; also whether the allowance ran out during it.
fn repair_one(
    solver: &mut CaDiCal,
    budget: &mut Budget,
    lits: &[i32],
    u: usize,
    conflicts: i32,
    rng: &mut SmallRng,
    in_d: &mut [bool],
) -> (Repair, bool) {
    let flip = -lits[u];
    let differs = |solver: &mut CaDiCal, v: usize| (solver.val(v as i32 + 1) > 0) != (lits[v] > 0);
    solver.limit(c"conflicts", conflicts);
    solver.assume(flip);
    let mut d: Vec<u32> = match budget.solve(solver, 1) {
        Status::Unsatisfiable => return (Repair::Backbone, false),
        Status::Unknown => return (Repair::Unknown, budget.out()),
        Status::Satisfiable => (0..lits.len())
            .filter(|&v| v != u && differs(solver, v))
            .map(|v| v as u32)
            .collect(),
    };
    budget.meter.charge(lits.len() as u64);
    let mut tested: Vec<u32> = Vec::new();
    let mut certified = true;
    loop {
        let left: Vec<u32> = d.iter().copied().filter(|v| !tested.contains(v)).collect();
        if left.is_empty() {
            break;
        }
        if budget.out() {
            return (Repair::Uncertified(d), true);
        }
        let v = left[rng.random_range(0..left.len())];
        tested.push(v);
        for &w in &d {
            in_d[w as usize] = w != v;
        }
        solver.assume(flip);
        let mut assumed = 1;
        for (w, &l) in lits.iter().enumerate() {
            if w != u && !in_d[w] {
                solver.assume(l);
                assumed += 1;
            }
        }
        for &w in &d {
            in_d[w as usize] = false;
        }
        solver.limit(c"conflicts", conflicts);
        match budget.solve(solver, assumed) {
            Status::Satisfiable => {
                d.retain(|&w| differs(solver, w as usize));
                budget.meter.charge(d.len() as u64);
            }
            Status::Unsatisfiable => {}
            Status::Unknown => certified = false,
        }
    }
    let repair = if certified {
        Repair::Minimal(d)
    } else {
        Repair::Uncertified(d)
    };
    (repair, false)
}
