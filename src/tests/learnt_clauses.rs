//! The one statement a harvested learnt clause has to satisfy, shared by the
//! two places that harvest one: the shim driven directly, and the bundle that
//! carries the harvest out.

use crate::cnf::{Clause, CnfFormula, Literal};
use crate::tests::pmc_oracle::brute_force_mc;

/// A learnt clause is one the formula already entails, so conjoining the whole
/// harvest back onto the formula it came from leaves the model count exactly
/// where it was. `learnts` is written in `reduced`'s own DIMACS numbering,
/// which is what makes conjoining them meaningful at all.
///
/// Both counts come from enumeration, so this says something about the clauses
/// rather than about whichever solver produced them — at the price of needing a
/// `reduced` small enough to enumerate.
pub(crate) fn assert_learnts_are_implied(reduced: &CnfFormula, learnts: &[Vec<i32>]) {
    let mut augmented = reduced.clone();
    for clause in learnts {
        augmented.clauses.push(Clause::new(
            clause.iter().map(|&l| Literal::from(l)).collect(),
        ));
    }
    assert_eq!(
        brute_force_mc(&augmented),
        brute_force_mc(reduced),
        "conjoining the {} harvested learnt clauses changed the model count — they are not \
         implied by the formula they were harvested from",
        learnts.len(),
    );
}
