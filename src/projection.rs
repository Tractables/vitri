//! Projection-preserving operations for derived CNFs.
//!
//! [`crate::bundle::preprocess`] owns the ordinary raw-input preprocessing
//! pipeline. This module is for a compiler that subsequently derives a
//! component, cofactor, or conditioned formula and needs one projection-safe
//! operation without restarting that whole pipeline.

use std::time::{Duration, Instant};

use crate::cnf::{CnfFormula, ShowSet, Space, VarId};
use crate::error::VitriError;
use crate::sat::{Bounded, CaDiCal, Status, WallClockTerminator};

/// Eliminate hidden variables by bounded resolution without growing the
/// clause count.
///
/// Every variable outside `show` is existentially quantified. A variable is
/// eliminated only when the number of unique non-tautological resolvents is no
/// greater than the number of clauses removed, so this operation never adopts
/// a larger clause set. Variable ids and `num_vars` are preserved.
///
/// # Errors
///
/// [`VitriError::Input`] if `show` names a variable outside `formula`.
pub fn eliminate_hidden<S: Space>(
    formula: &CnfFormula,
    show: &ShowSet<S>,
) -> Result<CnfFormula, VitriError> {
    if let Some(var) = show.iter_vars().find(|var| var.0 >= formula.num_vars) {
        return Err(VitriError::input(format!(
            "show variable {} exceeds formula variable count {}",
            var.to_dimacs(),
            formula.num_vars,
        )));
    }
    Ok(crate::preprocess::bve_project::bve_project(
        formula,
        &show.mask(formula.num_vars),
    ))
}

/// Work granted to [`classify_hidden_defined_by_show`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HiddenDefinabilityConfig {
    /// Per-variable SAT conflict limit. The preliminary satisfiability check
    /// needed for absent variables receives the same limit. An exhausted query
    /// is reported as unknown; only a completed refutation proves definability.
    pub max_conflicts_per_var: i32,
    /// Soft whole-sweep wall budget. Each SAT call receives only the remaining
    /// window, and no query starts after the budget is spent. The linear scan
    /// and dual-CNF construction cannot be interrupted, so setup already in
    /// progress may finish after this duration. `None` leaves the sweep
    /// unbounded.
    pub time_budget: Option<Duration>,
}

impl Default for HiddenDefinabilityConfig {
    fn default() -> Self {
        Self {
            max_conflicts_per_var: 2_000,
            time_budget: Some(Duration::from_secs(3)),
        }
    }
}

/// Classification of hidden variables by whether the shown variables
/// functionally determine them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HiddenDefinability {
    /// Variables proved to be functions of `show`.
    pub defined: Vec<VarId>,
    /// Variables for which a satisfying counterexample proved that `show` does
    /// not determine them. An absent variable enters here only after the base
    /// formula itself has been proved satisfiable.
    pub not_defined: Vec<VarId>,
    /// Variables whose probe exhausted its conflict or wall budget, or could
    /// not start because dual-CNF construction was unsafe or the SAT solver was
    /// unavailable.
    pub unknown: Vec<VarId>,
    /// Wall time spent on the classification.
    pub wall: Duration,
}

/// Prove which selected hidden variables are functions of the show set.
///
/// For each hidden variable `x`, the query asks whether two models can agree
/// on every shown variable while disagreeing on `x`; all other hidden
/// variables remain independent between the two models. Refuting that query
/// proves that `x` is determined by `show`. SAT and budget exhaustion never
/// claim definability.
///
/// Repeated `hidden` variables are ignored after their first occurrence.
/// Appearing variables are probed by descending literal incidence, breaking a
/// tie by descending [`VarId`], and retain that order within each result
/// category. Absent variables retain caller order and precede appearing ones in
/// `not_defined` or `unknown`. When absent targets require a preliminary base
/// check and it proves the formula unsatisfiable, every requested variable is
/// vacuously defined in caller order.
///
/// # Errors
///
/// [`VitriError::Input`] if either set names a variable outside `formula`, or a
/// requested hidden variable is also shown. [`VitriError::Config`] if an armed
/// work limit is zero or negative.
pub fn classify_hidden_defined_by_show<S: Space>(
    formula: &CnfFormula,
    show: &ShowSet<S>,
    hidden: impl IntoIterator<Item = VarId>,
    config: HiddenDefinabilityConfig,
) -> Result<HiddenDefinability, VitriError> {
    if config.max_conflicts_per_var <= 0 {
        return Err(VitriError::config(format!(
            "hidden-definability max_conflicts_per_var={} must be positive",
            config.max_conflicts_per_var,
        )));
    }
    if config.time_budget == Some(Duration::ZERO) {
        return Err(VitriError::config(
            "hidden-definability time_budget is armed with zero duration".to_owned(),
        ));
    }
    if let Some(var) = show.iter_vars().find(|var| var.0 >= formula.num_vars) {
        return Err(VitriError::input(format!(
            "show variable {} exceeds formula variable count {}",
            var.to_dimacs(),
            formula.num_vars,
        )));
    }

    let mut seen = vec![false; formula.num_vars as usize];
    let hidden: Vec<VarId> = hidden
        .into_iter()
        .filter(|var| {
            if var.0 >= formula.num_vars {
                return true;
            }
            let first = !seen[var.idx()];
            seen[var.idx()] = true;
            first
        })
        .collect();
    if let Some(var) = hidden.iter().find(|var| var.0 >= formula.num_vars) {
        return Err(VitriError::input(format!(
            "hidden variable {} exceeds formula variable count {}",
            var.to_dimacs(),
            formula.num_vars,
        )));
    }
    if let Some(var) = hidden.iter().find(|&&var| show.contains(var)) {
        return Err(VitriError::input(format!(
            "variable {} is both shown and requested as hidden",
            var.to_dimacs(),
        )));
    }

    let start = Instant::now();
    if hidden.is_empty() || formula.clauses.is_empty() {
        return Ok(HiddenDefinability {
            not_defined: hidden,
            wall: start.elapsed(),
            ..HiddenDefinability::default()
        });
    }
    if formula.clauses.iter().any(|clause| clause.is_empty()) {
        return Ok(HiddenDefinability {
            defined: hidden,
            wall: start.elapsed(),
            ..HiddenDefinability::default()
        });
    }

    let mut appears = vec![false; formula.num_vars as usize];
    let mut incidence = vec![0u32; formula.num_vars as usize];
    for clause in &formula.clauses {
        for literal in &clause.literals {
            appears[literal.var.idx()] = true;
            incidence[literal.var.idx()] = incidence[literal.var.idx()].saturating_add(1);
        }
    }

    let candidates: Vec<u32> = (0..formula.num_vars)
        .filter(|&var| appears[var as usize])
        .collect();
    let appearing_show: Vec<u32> = show
        .iter_vars()
        .filter(|var| appears[var.idx()])
        .map(|var| var.0)
        .collect();
    let mut ordered: Vec<u32> = hidden
        .iter()
        .filter(|var| appears[var.idx()])
        .map(|var| var.0)
        .collect();
    ordered.sort_by_key(|&var| std::cmp::Reverse((incidence[var as usize], var)));

    let absent: Vec<VarId> = hidden
        .iter()
        .copied()
        .filter(|var| !appears[var.idx()])
        .collect();
    let mut result = HiddenDefinability::default();

    // The wall is soft only for setup that is already running. Check on both
    // sides so an expired request neither starts the allocation-heavy dual
    // encoding nor starts a SAT query after that encoding finishes late.
    if budget_spent(start, config.time_budget) {
        result.unknown.extend(absent);
        result.unknown.extend(ordered.into_iter().map(VarId));
        result.wall = start.elapsed();
        return Ok(result);
    }

    let Some(mut dual) = crate::preprocess::build_dual_cnf_with_indicators(
        &formula.clauses,
        formula.num_vars as usize,
        &candidates,
    ) else {
        result.unknown.extend(absent);
        result.unknown.extend(ordered.into_iter().map(VarId));
        result.wall = start.elapsed();
        return Ok(result);
    };

    if budget_spent(start, config.time_budget) {
        result.unknown.extend(absent);
        result.unknown.extend(ordered.into_iter().map(VarId));
        result.wall = start.elapsed();
        return Ok(result);
    }

    // An absent variable is free only when the formula has a model. Ask once,
    // on the dual solver already needed by the appearing-variable probes. If
    // the base is UNSAT, functional determination is vacuous for every target;
    // if this check is cut off, the absent targets stay conservatively unknown.
    if !absent.is_empty() {
        match solve_with_limits(&mut dual.solver, start, config) {
            Status::Unsatisfiable => {
                result.defined = hidden;
                result.wall = start.elapsed();
                return Ok(result);
            }
            Status::Satisfiable => result.not_defined.extend(absent),
            Status::Unknown => result.unknown.extend(absent),
        }
    }

    if budget_spent(start, config.time_budget) {
        result.unknown.extend(ordered.into_iter().map(VarId));
        result.wall = start.elapsed();
        return Ok(result);
    }

    for (at, &var) in ordered.iter().enumerate() {
        if budget_spent(start, config.time_budget) {
            result
                .unknown
                .extend(ordered[at..].iter().copied().map(VarId));
            break;
        }

        dual.solver.assume(dual.layout.original_dimacs(var));
        let candidate = candidates
            .binary_search(&var)
            .expect("an appearing hidden variable is a dual-CNF candidate");
        dual.solver.assume(-dual.layout.primed_dimacs(candidate));
        for &shown in &appearing_show {
            let candidate = candidates
                .binary_search(&shown)
                .expect("an appearing shown variable is a dual-CNF candidate");
            dual.solver.assume(dual.layout.indicator_dimacs(candidate));
        }

        let status = solve_with_limits(&mut dual.solver, start, config);
        match status {
            Status::Unsatisfiable => result.defined.push(VarId(var)),
            Status::Satisfiable => result.not_defined.push(VarId(var)),
            Status::Unknown => result.unknown.push(VarId(var)),
        }
    }

    result.wall = start.elapsed();
    Ok(result)
}

fn budget_spent(start: Instant, budget: Option<Duration>) -> bool {
    budget.is_some_and(|budget| start.elapsed() >= budget)
}

/// Run one query under the shared per-query conflict ceiling and whatever is
/// left of the sweep's soft wall budget.
fn solve_with_limits(
    solver: &mut CaDiCal,
    start: Instant,
    config: HiddenDefinabilityConfig,
) -> Status {
    // Recheck at the call boundary: the caller's bookkeeping and assumptions
    // may have consumed the last sliver after its loop-level check.
    if budget_spent(start, config.time_budget) {
        return Status::Unknown;
    }
    solver.limit(c"conflicts", config.max_conflicts_per_var);
    match config.time_budget {
        Some(budget) => {
            let remaining = budget.saturating_sub(start.elapsed());
            Bounded::new(solver, WallClockTerminator::new(remaining)).solve()
        }
        None => solver.solve(),
    }
}
