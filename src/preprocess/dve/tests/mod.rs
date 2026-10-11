use crate::cnf::occ::appearance_mask;
use crate::cnf::{Clause, CnfFormula, Literal, VarId};
use crate::preprocess::dve::clause_db::LitMarks;
use crate::preprocess::dve::definability::pick_def_vars_with_meter;
use crate::preprocess::dve::elim::{
    Attempt, Eliminator, MAX_RESOLVENT_LEN, RESOLVENT_GROWTH_BOUND, dve_round,
};
use crate::preprocess::dve::gates::find_definition;
use crate::preprocess::dve::pipeline::{DveConfig, preprocess_dve, renumber_formula};
use crate::preprocess::dve::strengthen::FrozenEquiv;
use crate::preprocess::dve::types::{DveFate, DveResult};
use crate::preprocess::tests::wall_meter;
use crate::tests::common::make_formula;
use crate::tests::pmc_oracle::brute_force_mc;
use num_bigint::BigUint;

mod count_preserve;
mod elimination;
mod renumber;
mod strengthen;

/// The pass's configuration with everything a test does not vary fixed.
fn config(frozen: &rustc_hash::FxHashSet<VarId>, gate_definitions: bool) -> DveConfig<'_> {
    DveConfig {
        max_rounds: 10,
        time_limit_ms: 10_000,
        keep_original_vars: false,
        gate_definitions,
        frozen,
        frozen_equiv: FrozenEquiv::Ignore,
    }
}

/// The model count over the original variables that `clauses` and `fates`
/// stand for: variables left in no clause are named free (×2), the rest are
/// renumbered and counted by enumeration.
fn count_after(fates: &mut [DveFate], num_vars: usize, clauses: Vec<Clause>) -> BigUint {
    let appears = appearance_mask(&clauses, num_vars);
    let mut num_free = 0;
    for v in 0..num_vars {
        if !fates[v].eliminated() && !appears[v] {
            fates[v] = DveFate::Free;
            num_free += 1;
        }
    }
    let (reduced, _) = renumber_formula(fates, num_vars, clauses);
    brute_force_mc(&reduced) * BigUint::from(1u128 << num_free)
}

/// The count a pass result stands for: the reduced formula's, times 2 for each
/// variable the pass names free.
fn count_of(result: &DveResult) -> BigUint {
    brute_force_mc(&result.formula) * BigUint::from(1u128 << result.num_free())
}

/// Fail unless `result` stands for as many models as `original` has.
fn assert_count_preserved(original: &CnfFormula, result: &DveResult) {
    assert_eq!(
        count_of(result),
        brute_force_mc(original),
        "model count changed (defined={}, equiv={}, free={}, reduced_vars={}, clauses={:?})",
        result.num_defined(),
        result.num_equiv(),
        result.num_free(),
        result.formula.num_vars(),
        original
            .clauses()
            .iter()
            .map(|c| c.literals.iter().map(|l| l.to_dimacs()).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
    );
}

/// An elimination engine over `formula`, every variable unfrozen unless
/// `frozen` says otherwise.
fn engine<'a>(
    formula: &CnfFormula,
    fates: &'a mut [DveFate],
    frozen: &'a rustc_hash::FxHashSet<VarId>,
    gate_definitions: bool,
) -> Eliminator<'a> {
    Eliminator::new(
        formula.clauses().to_vec(),
        formula.num_vars() as usize,
        fates,
        frozen,
        gate_definitions,
    )
}

/// Eliminate DIMACS variable `x` through the gate definition it has among the
/// engine's live clauses.
fn eliminate_by_gate(engine: &mut Eliminator<'_>, x: i32) -> Attempt {
    let x = VarId::from_dimacs(x);
    let mut marks = LitMarks::new(engine.db.num_vars());
    let mut steps = 0;
    let definition = find_definition(&engine.db, x, &mut marks, &mut steps)
        .unwrap_or_else(|| panic!("variable {} has no gate definition", x.to_dimacs()));
    engine.eliminate(x, Some(&definition))
}

/// The engine's live clauses as sorted DIMACS lists, themselves sorted.
fn live_clauses(engine: &Eliminator<'_>) -> Vec<Vec<i32>> {
    let mut clauses: Vec<Vec<i32>> = engine
        .db
        .to_clauses()
        .iter()
        .map(|c| {
            let mut lits: Vec<i32> = c.literals.iter().map(|l| l.to_dimacs()).collect();
            lits.sort_by_key(|l| (l.abs(), *l));
            lits
        })
        .collect();
    clauses.sort();
    clauses
}
