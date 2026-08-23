//! Stripping variables out of the formula and renumbering what is
//! left.
//!
//! One pass finds the variables that are forced, dead or equivalent to
//! another, and rewrites the clauses without them; the loop repeats
//! until a pass finds nothing, because each removal can expose the
//! next.

use super::*;

/// Outcome of a single stripping attempt on a formula.
pub(super) enum StripOutcome {
    /// Stripping succeeded: the reduced formula plus the record to undo it.
    Stripped(CnfFormula, VariableStripping),
    /// Nothing worth stripping (no backbone and no strippable dead vars).
    Nothing,
    /// A forced/dead var still occurs in a surviving non-unit clause — upstream
    /// elimination was incomplete (typically a deadline hit mid-preprocess, before
    /// unit-propagation reached fixpoint). Recoverable by finishing that UP.
    Incomplete,
}

/// One stripping pass over `formula`. Pure: no fallback, no cleanup — callers
/// decide what to do with an `Incomplete` result.
pub(super) fn strip_once(formula: &CnfFormula) -> StripOutcome {
    let (forced_vars, backbone) = collect_forced_vars(formula);
    let dead_vars = collect_dead_vars(formula, &forced_vars);

    // Nothing to strip: no forced vars, and either no dead vars or every var
    // is dead (an all-dead formula is left to compile trivially, not
    // stripped to zero variables).
    if backbone.is_empty() && (dead_vars.is_empty() || dead_vars.len() == formula.num_vars as usize)
    {
        return StripOutcome::Nothing;
    }

    let renumbering = Renumber::keeping(formula.num_vars as usize, |v| {
        !forced_vars.contains(&v) && !dead_vars.contains(&v)
    });
    let Some(stripped_clauses) = rewrite_clauses(formula, &forced_vars, &renumbering) else {
        return StripOutcome::Incomplete;
    };

    let stripped = CnfFormula {
        num_vars: renumbering.num_new_vars(),
        clauses: stripped_clauses,
    };

    if !dead_vars.is_empty() {
        diag!(
            "[free-variable-stripping] {} vars with zero occurrences removed",
            dead_vars.len(),
        );
    }

    // HashSet iteration order is non-deterministic, and the vtree builder
    // appends a leaf per dead var in that order, so it affects the resulting
    // vtree shape and TDD compilation.
    let mut dead_sorted: Vec<VarId> = dead_vars.into_iter().collect();
    dead_sorted.sort_unstable_by_key(|v| v.0);

    let reduction = VariableStripping {
        backbone,
        dead: dead_sorted,
        renumbering,
    };

    StripOutcome::Stripped(stripped, reduction)
}

/// Strip forced (backbone) and dead variables out of the preprocessed formula
/// before vtree construction. Returns `None` when there is nothing to strip or a
/// stripping cannot be produced safely (the caller then compiles the un-stripped
/// formula — count-safe either way).
///
/// If a forced var still occurs in a longer clause (`Incomplete`), we don't
/// just bail: we finish the missing unit-propagation with the shared
/// count-preserving pass (`unit_propagation::propagate`) and strip the
/// fully-propagated formula, so the variable reduction is recovered rather
/// than discarded. The cleanup is cheap (O(literal occurrences)); the
/// downstream vtree/compile still honor the budget, so an already-exhausted
/// instance just aborts there — never here.
pub(super) fn strip_backbone_vars(formula: &CnfFormula) -> Option<(CnfFormula, VariableStripping)> {
    match strip_once(formula) {
        StripOutcome::Stripped(f, r) => Some((f, r)),
        StripOutcome::Nothing => None,
        StripOutcome::Incomplete => {
            // Removes every forced literal from the longer clauses and
            // returns the complete backbone — count-preserving.
            let (propagated, forced_lits) =
                crate::preprocess::unit_propagation::propagate(&formula.clauses, formula.num_vars);
            if crate::cnf::contains_empty_clause(&propagated) {
                // UP derived UNSAT (an empty clause). Hand back the un-stripped
                // formula and let the consumer settle count 0 on it, rather than
                // fabricating a stripping for a formula with no models.
                diag!(
                    "[backbone-stripping] skipped: unit-propagation cleanup found UNSAT; \
                     compiling the un-stripped formula",
                );
                return None;
            }
            // Re-materialize the fully-propagated backbone as unit clauses so the shared
            // strip path sees the complete forced set over the same variable space.
            let mut clauses = propagated;
            clauses.extend(forced_lits.iter().map(|l| Clause::new(vec![*l])));
            let cleaned = CnfFormula {
                num_vars: formula.num_vars,
                clauses,
            };

            match strip_once(&cleaned) {
                StripOutcome::Stripped(f, r) => {
                    diag!(
                        "[backbone-stripping] recovered via unit-propagation cleanup \
                         (incomplete preprocessing): {} → {} vars",
                        formula.num_vars,
                        f.num_vars,
                    );
                    Some((f, r))
                }
                // The cleaned formula satisfies the strip invariant, so this arm is not
                // expected; fall back to the un-stripped compile if it ever triggers.
                StripOutcome::Nothing | StripOutcome::Incomplete => None,
            }
        }
    }
}

/// Such clauses are removed during stripping — the value is recorded in
/// `backbone`.
pub(super) fn is_backbone_unit(
    clause: &Clause,
    forced_vars: &std::collections::HashSet<VarId>,
) -> bool {
    clause.literals.len() == 1 && forced_vars.contains(&clause.literals[0].var)
}

/// Returns `(forced_set, forced_list)` — the set is for O(1) lookup during
/// stripping; the list preserves first-occurrence order for
/// `VariableStripping::backbone`.
pub(super) fn collect_forced_vars(
    formula: &CnfFormula,
) -> (std::collections::HashSet<VarId>, Vec<(VarId, bool)>) {
    let mut forced_vars = std::collections::HashSet::new();
    let mut backbone = Vec::new();
    for clause in &formula.clauses {
        if clause.literals.len() == 1 {
            let lit = clause.literals[0];
            if forced_vars.insert(lit.var) {
                backbone.push((lit.var, lit.positive));
            }
        }
    }
    (forced_vars, backbone)
}

/// Detect dead variables: those with zero occurrences in any non-backbone
/// clause. CaDiCaL may eliminate variables entirely during preprocessing;
/// these remain in `num_vars` but don't appear in any clause.
pub(super) fn collect_dead_vars(
    formula: &CnfFormula,
    forced_vars: &std::collections::HashSet<VarId>,
) -> std::collections::HashSet<VarId> {
    let mut var_occurs = vec![false; formula.num_vars as usize];
    for clause in &formula.clauses {
        if is_backbone_unit(clause, forced_vars) {
            continue;
        }
        for lit in &clause.literals {
            var_occurs[lit.var.idx()] = true;
        }
    }

    (0..formula.num_vars)
        .map(VarId)
        .filter(|v| !forced_vars.contains(v) && !var_occurs[v.idx()])
        .collect()
}

/// Rewrite clauses for the stripped variable space: drop backbone units,
/// remap the rest.
///
/// Returns `None` if any surviving (non-backbone-unit) clause still
/// references a variable the stripping renumbering did not keep (a forced or
/// dead one). That invariant holds only when upstream preprocessing fully
/// eliminated every forced/dead var from the longer clauses; it is violated
/// when preprocessing is interrupted (e.g. the deadline hits
/// mid-backbone-elimination) and leaves a forced var inside a non-unit
/// clause.
///
/// This is why stripping cannot use the shared
/// [`renumber_clauses`](crate::preprocess::renumber::renumber_clauses)
/// rewrite: dropping a literal is an error here, not the normal case.
pub(super) fn rewrite_clauses(
    formula: &CnfFormula,
    forced_vars: &std::collections::HashSet<VarId>,
    renumbering: &Renumber,
) -> Option<Vec<Clause>> {
    let mut out = Vec::new();
    for clause in &formula.clauses {
        if is_backbone_unit(clause, forced_vars) {
            continue;
        }
        let mut new_lits = Vec::with_capacity(clause.literals.len());
        for lit in &clause.literals {
            // A forced/dead var surviving in a kept clause means
            // preprocessing did not eliminate it — bubble `None`.
            new_lits.push(renumbering.apply_lit(*lit)?);
        }
        out.push(Clause::new(new_lits));
    }
    Some(out)
}

/// The equivalence-reduced layer for `formula` under `mapping`. `None` when
/// the contract does not reduce equivalences, or no mapping was found.
pub(super) fn apply_equiv_reduction(
    formula: &CnfFormula,
    mapping: Option<EquivMapping>,
    reduce_equivalences: bool,
) -> Option<EquivReduction> {
    if !reduce_equivalences {
        return None;
    }
    let mapping = mapping?;

    let (reduced, renumbering) = mapping.reduce_formula(formula);
    diag!(
        "[equiv-reduction] {} → {} representative vars",
        formula.num_vars,
        reduced.num_vars,
    );
    Some(EquivReduction {
        formula: reduced,
        mapping,
        renumbering,
    })
}
