//! Stripping variables out of the formula and renumbering what is
//! left.
//!
//! One pass finds the variables that are forced or dead and rewrites the
//! clauses without them; if a forced variable still occurs in a longer
//! clause the pass is redone after unit propagation. Equivalences are
//! folded separately by [`apply_equiv_reduction`].

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
    let (forced, backbone) = collect_forced_vars(formula);
    let occurs = occurs_outside_backbone_units(formula, &forced);
    // Ascending, which the vtree builder relies on: it appends a leaf per dead
    // var in this order, so the order shapes the vtree and the compilation.
    let dead: Vec<VarId> = VarId::all(formula.num_vars())
        .filter(|v| !forced[v.idx()] && !occurs[v.idx()])
        .collect();

    // Nothing to strip: no forced vars, and either no dead vars or every var
    // is dead (an all-dead formula is left to compile trivially, not
    // stripped to zero variables).
    if backbone.is_empty() && (dead.is_empty() || dead.len() == formula.num_vars() as usize) {
        return StripOutcome::Nothing;
    }

    let renumbering = Renumber::keeping(formula.num_vars() as usize, |v| {
        !forced[v.idx()] && occurs[v.idx()]
    });
    let Some(stripped_clauses) = rewrite_clauses(formula, &forced, &renumbering) else {
        return StripOutcome::Incomplete;
    };

    let stripped = CnfFormula::from_parts(renumbering.num_new_vars(), stripped_clauses);

    if !dead.is_empty() {
        diag!(
            "[free-variable-stripping] {} vars with zero occurrences removed",
            dead.len(),
        );
    }

    let reduction = VariableStripping {
        backbone,
        dead,
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
            let (propagated, forced_lits) = crate::preprocess::unit_propagation::propagate(
                formula.clauses(),
                formula.num_vars(),
            );
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
            let cleaned = CnfFormula::from_parts(formula.num_vars(), clauses);

            match strip_once(&cleaned) {
                StripOutcome::Stripped(f, r) => {
                    diag!(
                        "[backbone-stripping] recovered via unit-propagation cleanup \
                         (incomplete preprocessing): {} → {} vars",
                        formula.num_vars(),
                        f.num_vars(),
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
/// `backbone`. `forced` is [`collect_forced_vars`]'s per-variable flag.
pub(super) fn is_backbone_unit(clause: &Clause, forced: &[bool]) -> bool {
    clause.literals.len() == 1 && forced[clause.literals[0].var.idx()]
}

/// Returns `(forced, backbone)`: per variable, whether a unit clause forces it,
/// and the forced literals in first-occurrence order for
/// `VariableStripping::backbone`.
pub(super) fn collect_forced_vars(formula: &CnfFormula) -> (Vec<bool>, Vec<(VarId, bool)>) {
    let mut forced = vec![false; formula.num_vars() as usize];
    let mut backbone = Vec::new();
    for clause in formula.clauses() {
        if let [lit] = *clause.literals.as_slice()
            && !std::mem::replace(&mut forced[lit.var.idx()], true)
        {
            backbone.push((lit.var, lit.positive));
        }
    }
    (forced, backbone)
}

/// Per variable, whether it occurs in a clause other than a backbone unit. A
/// variable that is neither forced nor occurs is dead: CaDiCaL may eliminate
/// variables entirely during preprocessing, and those remain in `num_vars`
/// without appearing in any clause.
pub(super) fn occurs_outside_backbone_units(formula: &CnfFormula, forced: &[bool]) -> Vec<bool> {
    let mut occurs = vec![false; formula.num_vars() as usize];
    for clause in formula.clauses() {
        if is_backbone_unit(clause, forced) {
            continue;
        }
        for lit in &clause.literals {
            occurs[lit.var.idx()] = true;
        }
    }
    occurs
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
    forced: &[bool],
    renumbering: &Renumber,
) -> Option<Vec<Clause>> {
    let mut out = Vec::new();
    for clause in formula.clauses() {
        if is_backbone_unit(clause, forced) {
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
        formula.num_vars(),
        reduced.num_vars(),
    );
    Some(EquivReduction {
        formula: reduced,
        mapping,
        renumbering,
    })
}
