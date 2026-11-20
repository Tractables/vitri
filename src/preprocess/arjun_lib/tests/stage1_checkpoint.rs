use super::*;
use crate::tests::pmc_oracle::{brute_force_mc, brute_force_wmc};

/// One wide clause: no variable is determined by the others (with the rest
/// all false the first is forced, but with the rest all true it is free), so
/// independent-support minimization cannot shrink the support at all.
const NOTHING_DEFINED: &[&[i32]] = &[&[1, 2, 3, 4, 5, 6, 7, 8]];

/// Four equivalence pairs: in each pair one variable is determined by the
/// other, so half the support is removable.
const HALF_DEFINED: &[&[i32]] = &[
    &[1, -2],
    &[-1, 2],
    &[3, -4],
    &[-3, 4],
    &[5, -6],
    &[-5, 6],
    &[7, -8],
    &[-7, 8],
];

/// Feed `clauses` over eight variables, with every variable in the sampling
/// set, and run stage 1 under `checkpoint` (`None`: none set). Returns whether
/// stage 1 stopped for lack of progress, the reduced formula and its support.
fn run_stage1(
    clauses: &[&[i32]],
    checkpoint: Option<(Duration, f64)>,
) -> (bool, CnfFormula, Vec<crate::cnf::VarId>) {
    let mut a = ArjunLib::new(ArjunOptions::default().seed).expect("shim ctor");
    a.new_vars(8);
    for c in clauses {
        a.add_clause_dimacs(c);
    }
    a.set_sampl(&VarId::all(8).collect::<Vec<_>>());
    if let Some((after, min_progress)) = checkpoint {
        a.set_progress_checkpoint(after, min_progress);
    }
    assert!(a.stage_minimize_indep(true), "minimize stage failed");
    (a.stopped_no_progress(), a.cur_formula(), a.cur_sampl())
}

#[test]
fn a_passed_checkpoint_stops_stage_one_when_no_variable_is_defined() {
    let (stopped, _, _) = run_stage1(NOTHING_DEFINED, Some((Duration::ZERO, 0.05)));
    assert!(stopped, "stage 1 should stop: nothing can shrink");
}

#[test]
fn a_passed_checkpoint_with_no_required_progress_does_not_stop_stage_one() {
    let (stopped, _, _) = run_stage1(NOTHING_DEFINED, Some((Duration::ZERO, 0.0)));
    assert!(!stopped);
}

#[test]
fn a_checkpoint_far_in_the_future_changes_nothing() {
    for clauses in [NOTHING_DEFINED, HALF_DEFINED] {
        let plain = run_stage1(clauses, None);
        let far = run_stage1(clauses, Some((Duration::from_secs(3600), 0.05)));
        assert!(!plain.0);
        assert!(!far.0);
        assert_eq!(plain.1, far.1, "reduced formula changed");
        assert_eq!(plain.2, far.2, "support changed");
    }
}

#[test]
fn stage_one_that_can_shrink_the_support_is_not_stopped_when_no_progress_is_required() {
    // The reference run shows the support does shrink on this formula; a
    // checkpoint asking for less than that is never a reason to stop. (The
    // check reads the clock only at a loop top, and the first one sees the
    // support untouched, so the requirement here is zero: the progress rule
    // itself is exercised by the two zero/non-zero cases above.)
    let (_, _, support) = run_stage1(HALF_DEFINED, None);
    assert!(support.len() < 8, "expected a smaller support");
    let (stopped, _, _) = run_stage1(HALF_DEFINED, Some((Duration::ZERO, 0.0)));
    assert!(!stopped);
}

/// Variables stage 1 finds free: 2 (its one clause is satisfied by the unit
/// on 1), the class {3, 4} (equivalent to each other and in no other clause)
/// and 5 (in no clause), beside a part that stays constrained (6, 7, 8).
const WITH_FREE_VARIABLES: &[&[i32]] = &[&[1], &[1, 2], &[3, -4], &[-3, 4], &[6, 7], &[-6, 8]];

/// Stage 1 folds the variables it finds free into the multiplier. Its
/// checkpoint is what a reduction returns when the heavy stage is skipped or
/// fails, and the reductions read it as a formula over all its variables, so
/// its count times the multiplier must be the input's count.
#[test]
fn the_stage_one_checkpoint_counts_its_free_variables_once() {
    let input = formula_over(8, WITH_FREE_VARIABLES);
    let mut a = ArjunLib::new(ArjunOptions::default().seed).expect("shim ctor");
    feed(&mut a, &input);
    a.set_sampl(&VarId::all(8).collect::<Vec<_>>());
    assert!(a.stage_minimize_indep(true), "minimize stage failed");
    let exp = read_multiplier_exp(&a, Spent::Unmeasured).expect("a power of two");
    assert!(exp > 0, "stage 1 should fold the free variables in");
    assert_eq!(
        brute_force_mc(&a.cur_formula()) << exp,
        brute_force_mc(&input)
    );
}

/// The same for the weighted reduction, whose multiplier takes each free
/// variable's two weights summed.
#[test]
fn the_weighted_stage_one_checkpoint_counts_its_free_variables_once() {
    let input = formula_over(8, WITH_FREE_VARIABLES);
    let weight = |v: u32, val: bool| {
        let den: i64 = if val { 3 } else { 5 };
        BigRational::new((v as i64 + 2).into(), den.into())
    };
    let mut a = ArjunLib::new_weighted(ArjunOptions::default().seed).expect("shim ctor");
    feed(&mut a, &input);
    for v in 0..8 {
        for val in [true, false] {
            let w = weight(v, val);
            let lit = Literal::new(VarId::from_idx(v as usize), val);
            a.set_lit_weight(lit, &format!("{}/{}", w.numer(), w.denom()))
                .expect("a rational weight");
        }
    }
    a.clean_sampl();
    assert!(a.stage_minimize_indep(true), "minimize stage failed");
    let multiplier = read_multiplier_weight(&a).expect("a rational multiplier");
    let reduced = a.cur_formula();
    let weights =
        crate::cnf::Weights::<crate::cnf::Reduced>::try_from_dimacs_lits(reduced.num_vars(), |l| {
            lit_weight_or_giveup(&a, l)
        })
        .expect("every literal has a weight");
    let reduced_count = brute_force_wmc(&reduced, |v, val| {
        let (neg, pos) = &weights.as_pairs()[v as usize];
        if val { pos.clone() } else { neg.clone() }
    });
    assert_eq!(reduced_count * multiplier, brute_force_wmc(&input, weight));
}

fn formula_over(num_vars: u32, clauses: &[&[i32]]) -> CnfFormula {
    CnfFormula::from_parts(
        num_vars,
        clauses
            .iter()
            .map(|c| Clause::new(c.iter().map(|&l| Literal::from(l)).collect()))
            .collect(),
    )
}

fn feed(a: &mut ArjunLib, formula: &CnfFormula) {
    a.new_vars(formula.num_vars());
    for c in formula.clauses() {
        let lits: Vec<i32> = c.literals.iter().map(|l| l.to_dimacs()).collect();
        a.add_clause_dimacs(&lits);
    }
}
