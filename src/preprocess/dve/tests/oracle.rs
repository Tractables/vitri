//! The oracle pass: vivification and redundant-clause removal keep exactly the
//! models of the formula, and do what they are for.

use super::*;
use crate::preprocess::dve::oracle::{Outcome, Window, run};

/// The pass over `clauses` with no wall and the full conflict budgets.
fn oracle(num_vars: usize, clauses: &mut Vec<Clause>) -> Outcome {
    let mut meter = wall_meter();
    let limit_ms = 60_000;
    let mark = meter.begin(
        crate::bundle::PreprocessPhase::Dve,
        std::time::Duration::from_millis(limit_ms),
    );
    let window = Window {
        mark,
        limit_ms,
        deadline: None,
    };
    run(clauses, num_vars, window, &mut meter)
}

/// The clauses as sorted DIMACS lists, themselves sorted.
fn dimacs(clauses: &[Clause]) -> Vec<Vec<i32>> {
    let mut out: Vec<Vec<i32>> = clauses
        .iter()
        .map(|c| {
            let mut lits: Vec<i32> = c.literals.iter().map(|l| l.to_dimacs()).collect();
            lits.sort_by_key(|l| (l.abs(), *l));
            lits
        })
        .collect();
    out.sort();
    out
}

/// Every assignment of `num_vars` variables (bit `v` is variable `v + 1`) that
/// satisfies `clauses`.
fn models(num_vars: usize, clauses: &[Clause]) -> Vec<u32> {
    (0..1u32 << num_vars)
        .filter(|bits| {
            clauses.iter().all(|c| {
                c.literals
                    .iter()
                    .any(|l| ((bits >> l.var.idx()) & 1 == 1) == l.positive)
            })
        })
        .collect()
}

fn clauses_of(f: &CnfFormula) -> Vec<Clause> {
    f.clauses().to_vec()
}

#[test]
fn a_clause_implied_by_the_others_is_removed() {
    // (b ∨ c) is the resolvent of the other two.
    let f = make_formula(3, vec![vec![1, 2], vec![-1, 3], vec![2, 3]]);
    let mut clauses = clauses_of(&f);
    assert_eq!(oracle(3, &mut clauses), Outcome::Rewritten);
    assert_eq!(dimacs(&clauses), vec![vec![-1, 3], vec![1, 2]]);
}

#[test]
fn a_literal_the_formula_makes_unnecessary_is_removed_from_its_clause() {
    // With a false, (a ∨ b) forces b, and then (a ∨ ¬b ∨ c) forces c: the
    // formula implies (a ∨ c), which replaces the longer clause.
    let f = make_formula(3, vec![vec![1, 2], vec![1, -2, 3]]);
    let mut clauses = clauses_of(&f);
    assert_eq!(oracle(3, &mut clauses), Outcome::Rewritten);
    assert_eq!(dimacs(&clauses), vec![vec![1, 2], vec![1, 3]]);
}

#[test]
fn a_formula_with_nothing_to_remove_is_left_as_it_is() {
    let f = make_formula(3, vec![vec![1, 2], vec![-1, 3], vec![-2, -3]]);
    let mut clauses = clauses_of(&f);
    assert_eq!(oracle(3, &mut clauses), Outcome::Unchanged);
    assert_eq!(dimacs(&clauses), dimacs(f.clauses()));
}

#[test]
fn an_unsatisfiable_formula_becomes_the_empty_clause() {
    let f = make_formula(2, vec![vec![1, 2], vec![-1], vec![-2]]);
    let mut clauses = clauses_of(&f);
    assert_eq!(oracle(2, &mut clauses), Outcome::Unsatisfiable);
    assert_eq!(clauses.len(), 1);
    assert!(clauses[0].literals.is_empty());
}

/// Over seeded random formulas of up to 10 variables the pass keeps the set of
/// models over the original variables, and it does rewrite some of them.
#[test]
fn the_pass_keeps_the_models_of_random_formulas() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let mut rewritten = 0;
    for case in 0..200 {
        let num_vars = 3 + rng.below(8);
        let mut input = Vec::new();
        for _ in 0..num_vars + rng.below(2 * num_vars + 1) {
            let len = 1 + rng.below(4.min(num_vars));
            input.push(rng.literals(num_vars, len));
        }
        let f = make_formula(num_vars as u32, input);
        let mut clauses = clauses_of(&f);
        let outcome = oracle(num_vars, &mut clauses);
        if outcome == Outcome::Unsatisfiable {
            assert!(models(num_vars, f.clauses()).is_empty(), "case {case}");
            continue;
        }
        rewritten += usize::from(outcome == Outcome::Rewritten);
        assert_eq!(
            models(num_vars, &clauses),
            models(num_vars, f.clauses()),
            "case {case}: {:?} became {:?}",
            dimacs(f.clauses()),
            dimacs(&clauses),
        );
    }
    assert!(
        rewritten > 0,
        "no case was rewritten; the test guards nothing"
    );
}
