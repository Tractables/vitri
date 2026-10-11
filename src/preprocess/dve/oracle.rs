//! The oracle pass: SAT-based clause vivification, then removal of redundant
//! clauses, run once on the clauses DVE leaves.
//!
//! Both operations keep the formula's models exactly, over every variable, so
//! they are sound for every counting mode — weighted ones included — and need
//! no frozen set. A variable they leave in no clause is named free by
//! [`super::pipeline`], the same way as one the elimination emptied.
//!
//! **Vivification.** All clauses go into one incremental CaDiCaL solver. For a
//! clause `C` and a literal `l` of it, the solver is asked whether the formula
//! plus the negation of `C ∖ {l}` is satisfiable. If not, the formula implies
//! `C ∖ {l}`, which replaces `C`. A satisfying answer is a model of the
//! formula in which `l` is the only true literal of `C`; the most recent
//! models are kept, and any of them that has this property for a literal
//! answers the question for that literal without a solver call. Clauses are
//! visited longest first.
//!
//! **Redundant-clause removal.** A fresh solver holds `C_i ∨ s_i` for each
//! clause, with a fresh indicator `s_i`. Assuming `¬s_j` for every other clause
//! still present switches those on, and assuming the negation of `C_i` then
//! asks whether the rest implies `C_i`; if so the clause is dropped for good by
//! asserting `s_i`. Clauses whose removal would delete the most edges of the
//! primal graph are tried first.
//!
//! Every solver call is bounded by a conflict limit, each operation by a total
//! conflict budget, and the pass by the DVE stage's wall. A call that hits a
//! limit changes nothing.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use rustc_hash::FxHashMap;

use crate::cnf::{Clause, Literal};
use crate::diagnostics::diag;
use crate::preprocess::cadical::WallClockTerminator;
use crate::preprocess::cadical_ffi::{Bounded, CaDiCal, Status, note_solver_unavailable};
use crate::preprocess::meter::{PhaseMark, PreprocessMeter};

use super::clause_db::{ClauseDb, subset_sorted};

/// Conflicts one vivification solver call may spend before it gives up on its
/// literal.
const VIVIFY_CONFLICTS_PER_CALL: i64 = 1_000;

/// Conflicts all vivification calls of one run may spend together.
const VIVIFY_CONFLICTS_TOTAL: i64 = 200_000;

/// Conflicts one redundancy call may spend before it keeps its clause.
const SPARSIFY_CONFLICTS_PER_CALL: i64 = 2_000;

/// Conflicts all redundancy calls of one run may spend together.
const SPARSIFY_CONFLICTS_TOTAL: i64 = 200_000;

/// How many of the most recent models vivification keeps to certify literals
/// without a solver call.
const MODEL_CACHE: usize = 32;

/// Clause count above which redundancy removal is skipped: each of its calls
/// assumes one indicator per clause still present.
const SPARSIFY_MAX_CLAUSES: usize = 20_000;

/// Variable count above which redundancy removal tries clauses in index order
/// instead of by the primal-graph edges their removal would delete: counting
/// the edges costs a hash entry per pair of variables sharing a clause.
const ORDER_MAX_VARS: usize = 20_000;

/// Clause length above which a clause adds no pairs to the edge count, since a
/// clause of `k` literals adds `k(k-1)/2` of them.
const ORDER_MAX_CLAUSE_LEN: usize = 32;

/// Subsumption checks (one per candidate clause literal visited) after which
/// the final subsumption sweep stops and keeps the clauses it has not reached.
const SUBSUME_STEP_CAP: u64 = 50_000_000;

/// The wall-clock a stand-alone terminator is given when the stage has no
/// deadline (deterministic preprocessing), long enough never to fire.
const NO_DEADLINE: Duration = Duration::from_secs(30 * 24 * 3600);

/// Research seam, to be removed once the pass is judged: `VITRI_RESEARCH_ORACLE=0`
/// turns the pass off for an A/B; anything else, or unset, leaves it on. This is
/// the one place the variable is read.
pub(super) fn enabled() -> bool {
    crate::env::env_opt("VITRI_RESEARCH_ORACLE").is_none_or(|v| v.trim() != "0")
}

/// How long the pass may run: the DVE stage's budget, as the meter reads it,
/// and the same budget as a wall instant for the solver's terminator.
#[derive(Clone, Copy)]
pub(super) struct Window {
    pub(super) mark: PhaseMark,
    pub(super) limit_ms: u64,
    pub(super) deadline: Option<Instant>,
}

impl Window {
    fn remaining_ms(&self, meter: &PreprocessMeter) -> u64 {
        self.limit_ms.saturating_sub(meter.elapsed_ms(self.mark))
    }
}

/// What a run did to the clauses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    /// The clauses are as they were given (up to order).
    Unchanged,
    /// A literal or a clause was removed.
    Rewritten,
    /// The formula has no model. The clauses are now the single empty clause,
    /// the representation the equivalence merge uses.
    Unsatisfiable,
}

/// One operation's budget: where it stops in the meter's currency, the wall
/// instant that cuts a running solver call, and its conflict allowance.
struct Slice {
    until_ms: u64,
    wall: Duration,
    per_call: i64,
    total: i64,
}

impl Slice {
    fn out_of_time(&self, win: &Window, meter: &PreprocessMeter) -> bool {
        meter.elapsed_ms(win.mark) >= self.until_ms
    }
}

/// What a run measured, for the one diagnostic line.
#[derive(Default)]
struct Report {
    clauses_before: usize,
    literals_before: usize,
    clauses_vivified: usize,
    literals_vivified: usize,
    sparsified_away: usize,
    conflicts: i64,
    calls: u64,
    vivify_ms: u128,
    sparsify_ms: u128,
}

/// Run vivification and redundant-clause removal on `clauses` over variables
/// `0..num_vars`.
pub(super) fn run(
    clauses: &mut Vec<Clause>,
    num_vars: usize,
    win: Window,
    meter: &mut PreprocessMeter,
) -> Outcome {
    if clauses.is_empty() || clauses.iter().any(|c| c.literals.is_empty()) {
        return Outcome::Unchanged;
    }
    let mut report = Report {
        clauses_before: clauses.len(),
        literals_before: clauses.iter().map(|c| c.literals.len()).sum(),
        ..Report::default()
    };
    meter.charge_scan(|| report.literals_before);

    // Normalized and duplicate-free, which both operations and the sorted
    // subset test rely on.
    let mut work: Vec<Vec<Literal>> = {
        let mut seen = rustc_hash::FxHashSet::default();
        clauses
            .iter()
            .filter_map(|c| crate::cnf::normalize_literals(c.literals.clone()))
            .filter(|lits| seen.insert(lits.clone()))
            .collect()
    };

    // Vivification gets half of what is left, so removal has its share.
    let remaining = win.remaining_ms(meter);
    let now = Instant::now();
    let half_wall = win
        .deadline
        .map_or(NO_DEADLINE, |d| d.saturating_duration_since(now) / 2);
    let vivify_slice = Slice {
        until_ms: meter.elapsed_ms(win.mark) + remaining / 2,
        wall: half_wall,
        per_call: VIVIFY_CONFLICTS_PER_CALL,
        total: VIVIFY_CONFLICTS_TOTAL,
    };
    let started = Instant::now();
    let unsat = vivify(&mut work, num_vars, &win, &vivify_slice, meter, &mut report);
    report.vivify_ms = started.elapsed().as_millis();
    if unsat {
        diag!("[oracle] the formula is unsatisfiable");
        *clauses = vec![Clause::new(vec![])];
        return Outcome::Unsatisfiable;
    }
    work = drop_subsumed(num_vars, work, meter);
    report.clauses_vivified = work.len();
    report.literals_vivified = work.iter().map(Vec::len).sum();

    let started = Instant::now();
    let sparse_slice = Slice {
        until_ms: win.limit_ms,
        wall: win
            .deadline
            .map_or(NO_DEADLINE, |d| d.saturating_duration_since(Instant::now())),
        per_call: SPARSIFY_CONFLICTS_PER_CALL,
        total: SPARSIFY_CONFLICTS_TOTAL,
    };
    let kept = sparsify(&work, num_vars, &win, &sparse_slice, meter, &mut report);
    report.sparsify_ms = started.elapsed().as_millis();
    report.sparsified_away = work.len() - kept.len();

    let literals_after: usize = kept.iter().map(Vec::len).sum();
    diag!(
        "[oracle] vivify: {} -> {} clauses, {} -> {} literals; sparsify: {} clauses removed; {} conflicts, {} calls; {} ms + {} ms",
        report.clauses_before,
        report.clauses_vivified,
        report.literals_before,
        report.literals_vivified,
        report.sparsified_away,
        report.conflicts,
        report.calls,
        report.vivify_ms,
        report.sparsify_ms,
    );

    if kept.len() == report.clauses_before && literals_after == report.literals_before {
        return Outcome::Unchanged;
    }
    *clauses = kept.into_iter().map(Clause::new).collect();
    Outcome::Rewritten
}

/// A bit per variable: a model of the formula.
type Model = Vec<u64>;

fn model_true(model: &Model, lit: Literal) -> bool {
    let v = lit.var.idx();
    ((model[v / 64] >> (v % 64)) & 1 == 1) == lit.positive
}

/// Whether `model` makes `clause[at]` true and every other literal false: it
/// satisfies the clause only through that literal.
fn certifies(model: &Model, clause: &[Literal], at: usize) -> bool {
    clause
        .iter()
        .enumerate()
        .all(|(k, &l)| model_true(model, l) == (k == at))
}

/// Solve under the slice's per-call limit (capped by what is left of its total)
/// and count the conflicts. `None` when the total is spent.
fn bounded_solve(
    solver: &mut Bounded<'_, WallClockTerminator>,
    slice: &Slice,
    used: &mut i64,
    win: &Window,
    meter: &mut PreprocessMeter,
    report: &mut Report,
) -> Option<Status> {
    let left = slice.total - *used;
    if left <= 0 || slice.out_of_time(win, meter) {
        return None;
    }
    let before = solver.search_stats().conflicts;
    solver.limit(c"conflicts", slice.per_call.min(left) as i32);
    let status = meter.solve(crate::bundle::PreprocessPhase::Dve, &mut *solver);
    let spent = (solver.search_stats().conflicts - before).max(0);
    *used += spent;
    report.conflicts += spent;
    report.calls += 1;
    Some(status)
}

/// Shorten the clauses of `work` that the rest of the formula makes longer than
/// necessary. Returns true when the formula is found unsatisfiable (and `work`
/// is then not to be used).
fn vivify(
    work: &mut [Vec<Literal>],
    num_vars: usize,
    win: &Window,
    slice: &Slice,
    meter: &mut PreprocessMeter,
    report: &mut Report,
) -> bool {
    let Some(mut solver) = CaDiCal::new() else {
        note_solver_unavailable("oracle vivification", "no clause is vivified");
        return false;
    };
    solver.reserve(num_vars as i32);
    for clause in work.iter() {
        for lit in clause {
            solver.add(lit.to_dimacs());
        }
        solver.add(0);
    }
    meter.charge_scan(|| work.iter().map(Vec::len).sum());
    let mut solver = Bounded::new(&mut solver, WallClockTerminator::new(slice.wall));

    // The variables a model must be read for: the ones some clause mentions.
    let mut appears = vec![false; num_vars];
    for lit in work.iter().flatten() {
        appears[lit.var.idx()] = true;
    }
    let appearing: Vec<usize> = (0..num_vars).filter(|&v| appears[v]).collect();
    let words = num_vars.div_ceil(64).max(1);

    let mut cache: VecDeque<Model> = VecDeque::with_capacity(MODEL_CACHE);
    let mut used = 0i64;

    let read_model = |solver: &mut Bounded<'_, WallClockTerminator>,
                      cache: &mut VecDeque<Model>,
                      meter: &mut PreprocessMeter| {
        let mut model = vec![0u64; words];
        for &v in &appearing {
            if solver.val(crate::cnf::VarId::from_idx(v).to_dimacs()) > 0 {
                model[v / 64] |= 1 << (v % 64);
            }
        }
        meter.charge(appearing.len() as u64);
        if cache.len() == MODEL_CACHE {
            cache.pop_front();
        }
        cache.push_back(model);
    };

    // With nothing assumed, an unsatisfiable answer is the formula's own.
    match bounded_solve(&mut solver, slice, &mut used, win, meter, report) {
        Some(Status::Unsatisfiable) => return true,
        Some(Status::Satisfiable) => read_model(&mut solver, &mut cache, meter),
        _ => {}
    }

    let mut order: Vec<usize> = (0..work.len()).filter(|&i| work[i].len() > 1).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(work[i].len()), i));

    'clauses: for i in order {
        let mut at = 0;
        while work[i].len() > 1 && at < work[i].len() {
            if cache.iter().any(|m| certifies(m, &work[i], at)) {
                at += 1;
                continue;
            }
            for (k, lit) in work[i].iter().enumerate() {
                if k != at {
                    solver.assume(-lit.to_dimacs());
                }
            }
            meter.charge(work[i].len() as u64);
            match bounded_solve(&mut solver, slice, &mut used, win, meter, report) {
                None => break 'clauses,
                Some(Status::Unsatisfiable) => {
                    work[i].remove(at);
                    for lit in &work[i] {
                        solver.add(lit.to_dimacs());
                    }
                    solver.add(0);
                    at = 0;
                }
                Some(Status::Satisfiable) => {
                    read_model(&mut solver, &mut cache, meter);
                    at += 1;
                }
                Some(Status::Unknown) => at += 1,
            }
        }
    }
    false
}

/// Remove the clauses another clause subsumes. Duplicates are already gone.
fn drop_subsumed(
    num_vars: usize,
    work: Vec<Vec<Literal>>,
    meter: &mut PreprocessMeter,
) -> Vec<Vec<Literal>> {
    let mut db = ClauseDb::new(num_vars, work.into_iter().map(Clause::new));
    let total = db.live_count();
    let mut ids: Vec<u32> = (0..total as u32).collect();
    ids.sort_by_key(|&id| (db.clause(id).len(), id));
    let mut steps = 0u64;
    for id in ids {
        if steps > SUBSUME_STEP_CAP {
            break;
        }
        let clause = db.clause(id).to_vec();
        let subsumed = clause.iter().any(|&lit| {
            db.first_occurrences(lit).any(|other| {
                steps += 1;
                other != id && subset_sorted(db.clause(other), &clause)
            })
        });
        if subsumed {
            db.remove(id);
        }
    }
    meter.charge(steps);
    db.to_clauses().into_iter().map(|c| c.literals).collect()
}

/// The order redundancy removal tries clauses in: those whose removal would
/// delete the most primal-graph edges first. A clause scores the number of its
/// variable pairs shared with no other clause, then with exactly one other,
/// then exactly two others; larger first, ties by index.
fn removal_order(
    clauses: &[Vec<Literal>],
    num_vars: usize,
    meter: &mut PreprocessMeter,
) -> Vec<usize> {
    let mut order: Vec<usize> = (0..clauses.len()).collect();
    if num_vars > ORDER_MAX_VARS {
        return order;
    }
    let pairs = |c: &Vec<Literal>| {
        let k = c.len();
        (k <= ORDER_MAX_CLAUSE_LEN)
            .then(move || (0..k).flat_map(move |a| (a + 1..k).map(move |b| (a, b))))
    };
    let key = |c: &[Literal], a: usize, b: usize| {
        let (u, v) = (c[a].var.idx() as u64, c[b].var.idx() as u64);
        (u.min(v) << 32) | u.max(v)
    };
    let mut multiplicity: FxHashMap<u64, u32> = FxHashMap::default();
    for c in clauses {
        for (a, b) in pairs(c).into_iter().flatten() {
            *multiplicity.entry(key(c, a, b)).or_insert(0) += 1;
        }
    }
    meter.charge(multiplicity.len() as u64);
    let score: Vec<[u32; 3]> = clauses
        .iter()
        .map(|c| {
            let mut s = [0u32; 3];
            for (a, b) in pairs(c).into_iter().flatten() {
                let m = multiplicity[&key(c, a, b)] as usize;
                if m <= 3 {
                    s[m - 1] += 1;
                }
            }
            s
        })
        .collect();
    order.sort_by(|&i, &j| score[j].cmp(&score[i]).then(i.cmp(&j)));
    order
}

/// The clauses of `work` that the others do not imply, in their original order.
fn sparsify(
    work: &[Vec<Literal>],
    num_vars: usize,
    win: &Window,
    slice: &Slice,
    meter: &mut PreprocessMeter,
    report: &mut Report,
) -> Vec<Vec<Literal>> {
    let n = work.len();
    if !(2..=SPARSIFY_MAX_CLAUSES).contains(&n) {
        return work.to_vec();
    }
    let Some(mut solver) = CaDiCal::new() else {
        note_solver_unavailable("oracle redundancy removal", "no clause is removed");
        return work.to_vec();
    };
    // Indicator `s_i` of clause `i` is the variable after the formula's own.
    let indicator = |i: usize| (num_vars + 1 + i) as i32;
    solver.reserve((num_vars + n) as i32);
    for (i, clause) in work.iter().enumerate() {
        for lit in clause {
            solver.add(lit.to_dimacs());
        }
        solver.add(indicator(i));
        solver.add(0);
    }
    meter.charge_scan(|| work.iter().map(|c| c.len() + 1).sum());
    let mut solver = Bounded::new(&mut solver, WallClockTerminator::new(slice.wall));

    let mut used = 0i64;
    let mut active = vec![true; n];

    // A formula with no model implies every clause; leave it alone.
    for i in 0..n {
        solver.assume(-indicator(i));
    }
    if bounded_solve(&mut solver, slice, &mut used, win, meter, report)
        == Some(Status::Unsatisfiable)
    {
        return work.to_vec();
    }

    for i in removal_order(work, num_vars, meter) {
        for (j, &on) in active.iter().enumerate() {
            if on && j != i {
                solver.assume(-indicator(j));
            }
        }
        for lit in &work[i] {
            solver.assume(-lit.to_dimacs());
        }
        meter.charge((n + work[i].len()) as u64);
        match bounded_solve(&mut solver, slice, &mut used, win, meter, report) {
            None => break,
            Some(Status::Unsatisfiable) => {
                active[i] = false;
                solver.add(indicator(i));
                solver.add(0);
            }
            Some(_) => {}
        }
    }
    work.iter()
        .zip(&active)
        .filter(|(_, on)| **on)
        .map(|(c, _)| c.clone())
        .collect()
}
