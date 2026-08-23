//! A deterministic, self-generated dense-clause CNF fixture.
//!
//! Same motivation as [`circuit_fixture`](crate::tests::circuit_fixture): **no
//! third-party test data ships with the crate**. Where that module generates
//! genuine circuit structure, this one generates the opposite extreme — a
//! handful of variables carrying tens of thousands of clauses — which is the
//! shape that makes an *incidence* tree decomposition pathological.
//!
//! [`bag_adjacency_explosion`] is a pure function: a fixed linear congruential
//! sequence drives every choice, so the formula is byte-identical on every run
//! and every platform. No RNG crate, no clock, no I/O.
//!
//! # Why this shape
//!
//! The incidence graph of an `n`-variable, `m`-clause formula has `n + m`
//! vertices, and every clause vertex has degree equal to its width. With `n`
//! tiny and `m` enormous, a min-degree elimination order removes all `m` clause
//! vertices first, so the decomposition ends up with roughly `m` bags — and
//! since every bag draws its variables from the same `n`-element pool, nearly
//! every PAIR of bags intersects. The junction-tree construction inside the
//! vendored FlowCutter materialises that intersection graph explicitly, so its
//! arc count grows as `m²`: at the sizes below that is over a billion arcs,
//! tens of gigabytes, which is what used to take the process down.
//!
//! The treewidth here is small (bounded by the variable count). It is the bag
//! ADJACENCY, not the width, that explodes — which is exactly the case the
//! regression guard in `decompose::flowcutter` exists for.

use crate::cnf::{Clause, CnfFormula};
use crate::tests::common::Lcg;
use crate::vtree::Literal;

/// Variables in the generated formula. Small on purpose: this is the `n` that
/// forces every bag to overlap nearly every other one.
const NUM_VARS: u32 = 43;

/// Clauses in the generated formula. This is the `m` whose square drives the
/// bag-adjacency arc count — around 1.9e9 pairs here, far past the vendored
/// builder's arc budget, which is what makes the guard meaningful. Lowering it
/// by an order of magnitude would stop reproducing the failure shape.
const NUM_CLAUSES: usize = 43_162;

/// A dense CNF over [`NUM_VARS`] variables whose incidence graph blows up the
/// bag-adjacency structure of a min-degree tree decomposition.
///
/// Clause widths are 4, 5 or 6 (mixed roughly 1:7:12), each clause holding that
/// many DISTINCT variables so a clause vertex's incidence degree is exactly its
/// width. Polarities alternate off the same sequence. The formula's
/// satisfiability is irrelevant and untested — decomposition reads structure
/// only.
pub(super) fn bag_adjacency_explosion() -> CnfFormula {
    let mut rng = Lcg::new(0x5eed_1234_9abc_def0);
    // Reused across clauses: a partial Fisher-Yates shuffle only touches the
    // prefix it draws, and leaving the rest permuted from the previous clause
    // is harmless — every draw is still uniform over what remains.
    let mut pool: Vec<u32> = (1..=NUM_VARS as i32).map(|v| v as u32).collect();
    let mut clauses = Vec::with_capacity(NUM_CLAUSES);

    for _ in 0..NUM_CLAUSES {
        let width = match rng.below(20) {
            0 => 4,
            1..=7 => 5,
            _ => 6,
        };
        let mut lits = Vec::with_capacity(width);
        for i in 0..width {
            // Draw position `i` from the untouched suffix, so the prefix
            // 0..width holds `width` distinct variables.
            let j = i + rng.below((pool.len() - i) as u64) as usize;
            pool.swap(i, j);
            let var = pool[i] as i32;
            lits.push(if rng.below(2) == 0 { -var } else { var });
        }
        clauses.push(Clause::new(lits.into_iter().map(Literal::from).collect()));
    }

    CnfFormula {
        num_vars: NUM_VARS,
        clauses,
    }
}

/// The fixture is a pure function: two calls must produce the identical
/// formula, or the regression guard built on it is guarding noise.
#[test]
fn generation_is_deterministic() {
    let (a, b) = (bag_adjacency_explosion(), bag_adjacency_explosion());
    assert_eq!(a.num_vars, b.num_vars);
    assert_eq!(a.clauses, b.clauses);
}

#[test]
fn clauses_are_wide_and_variable_distinct() {
    let f = bag_adjacency_explosion();
    assert_eq!(f.num_vars, NUM_VARS);
    assert_eq!(f.clauses.len(), NUM_CLAUSES);

    let mut occ = vec![0usize; f.num_vars as usize];
    for c in &f.clauses {
        assert!(
            (4..=6).contains(&c.literals.len()),
            "clause width outside 4..=6: {}",
            c.literals.len()
        );
        let mut seen = Vec::with_capacity(c.literals.len());
        for l in &c.literals {
            assert!(
                !seen.contains(&l.var.idx()),
                "repeated variable in a clause drops its incidence degree"
            );
            seen.push(l.var.idx());
            occ[l.var.idx()] += 1;
        }
    }
    assert!(
        occ.iter().all(|&o| o > 0),
        "declared but unused variable shrinks the pool every bag draws from"
    );
}

/// THE structural property the fixture exists for, asserted directly rather
/// than by running a decomposition.
///
/// A min-degree order over the incidence graph eliminates all
/// [`NUM_CLAUSES`] clause vertices before any variable, giving one bag per
/// clause. The junction-tree build then materialises, for every bag, an arc to
/// each OTHER bag sharing a variable with it — so the arc count is at least
/// `NUM_CLAUSES × (fewest bags any single variable appears in) − NUM_CLAUSES`.
/// That lower bound has to clear the vendored builder's 64Mi-arc budget by a
/// wide margin.
#[test]
fn bag_adjacency_lower_bound_clears_the_arc_budget() {
    let f = bag_adjacency_explosion();
    let mut occ = vec![0usize; f.num_vars as usize];
    for c in &f.clauses {
        for l in &c.literals {
            occ[l.var.idx()] += 1;
        }
    }
    let least = *occ.iter().min().expect("at least one variable");
    let arcs_lower_bound = f.clauses.len() * least - f.clauses.len();
    // `kMaxBagAdjacencyArcs` in the vendored tree decomposition builder.
    const ARC_BUDGET: usize = 64 * 1024 * 1024;
    assert!(
        arcs_lower_bound > 3 * ARC_BUDGET,
        "bag-adjacency lower bound {arcs_lower_bound} does not clearly exceed the \
         {ARC_BUDGET}-arc budget — the fixture no longer reproduces the failure shape",
    );
}
