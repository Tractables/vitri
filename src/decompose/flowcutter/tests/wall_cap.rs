//! What a wall cap is allowed to change about the search it bounds.

use crate::cnf::{Clause, CnfFormula, Literal};
use crate::decompose::flowcutter::flowcutter_td;
use crate::decompose::{FcBudget, GraphKind, TreeDecomposition, WallCapMode};
use crate::tests::td_fixture::assert_valid_td;
use crate::vtree::VarId;

/// A circulant 3-CNF over `n` variables: variable `i` shares a clause with
/// `i+1`, `i+7`, `i+13`, `i+31`, `i+57` and `i+101` (mod `n`).
///
/// Three clauses per variable, so the incidence graph has `4n` vertices and the
/// primal graph `n`. The strides are coprime spreads rather than a local band,
/// which keeps the decomposition from collapsing to a path a single greedy pass
/// finds instantly.
fn circulant_3cnf(n: u32) -> CnfFormula {
    let mut clauses: Vec<Clause> = Vec::with_capacity(3 * n as usize);
    for i in 0..n {
        for (a, b) in [(1u32, 7u32), (13, 31), (57, 101)] {
            clauses.push(Clause::new(vec![
                Literal::new(VarId(i), true),
                Literal::new(VarId((i + a) % n), false),
                Literal::new(VarId((i + b) % n), true),
            ]));
        }
    }
    CnfFormula {
        num_vars: n,
        clauses,
    }
}

/// The bags of `td`, each sorted, so two decompositions can be compared as
/// values.
fn bag_sets(td: &TreeDecomposition) -> Vec<Vec<u32>> {
    td.bags
        .iter()
        .map(|b| {
            let mut v = b.vertices.clone();
            v.sort_unstable();
            v
        })
        .collect()
}

/// A [`WallCapMode::BoundOnly`] cap the build never reaches must produce the
/// same decomposition as no cap at all, bag for bag.
///
/// This is what lets a construction budget be enforced without changing what
/// construction does. The vendored timed entry differs from the step-budgeted
/// one in more than when it stops — it also tightens the pre-loop heuristic node
/// gates and drops the step clamp — so a cap that carried tightness with it
/// would change the tree on every instance, in service of bounding the few that
/// overrun.
///
/// The fixture is sized past the tight min-degree gate (700 variables, so 2 800
/// incidence vertices, over the 2 000-vertex tight limit and under the
/// 50 000-vertex loose one) on purpose: below it both modes agree trivially.
/// Substituting [`WallCapMode::Tight`] here fails the assertion.
#[test]
fn a_bound_only_wall_the_build_never_reaches_decomposes_exactly_as_no_wall_does() {
    let formula = circulant_3cnf(700);

    let unbounded = flowcutter_td(
        &formula,
        GraphKind::Incidence,
        FcBudget::Steps {
            steps: 20_000,
            iters: 4,
        },
    )
    .expect("the step-budgeted search decomposes this formula");

    // Ten minutes: a real bound, and one this build finishes far inside.
    let bounded = flowcutter_td(
        &formula,
        GraphKind::Incidence,
        FcBudget::Timed {
            timeout_ms: 600_000,
            patience_ms: 0,
            iters: 4,
            steps: 20_000,
            cap_mode: WallCapMode::BoundOnly,
        },
    )
    .expect("the bound-only search decomposes this formula");

    assert_eq!(
        unbounded.treewidth(),
        bounded.treewidth(),
        "a bound-only wall changed the width found",
    );
    assert_eq!(
        bag_sets(&unbounded),
        bag_sets(&bounded),
        "a bound-only wall changed the decomposition itself",
    );
}

/// A greedy elimination pass that runs out of time is dropped whole, so the
/// decomposition still covers every vertex.
///
/// The two passes are abandoned mid-way under a tight wall, and an elimination
/// order missing its tail is not a permutation: handing one on would decompose a
/// subset of the graph and leave the rest of the variables in no bag at all.
/// The contract is that an abandoned pass returns nothing and is skipped exactly
/// as the size gates skip it, which is what this asserts — whether or not the
/// wall happens to cut a pass short on any given run.
#[test]
fn a_wall_that_cuts_a_greedy_pass_short_still_yields_a_decomposition_of_the_whole_graph() {
    let n = 1_500;
    let formula = circulant_3cnf(n);
    let edges: Vec<(u32, u32)> = formula
        .clauses
        .iter()
        .flat_map(|c| {
            let v: Vec<u32> = c.literals.iter().map(|l| l.var.0).collect();
            [(v[0], v[1]), (v[0], v[2]), (v[1], v[2])]
        })
        .collect();

    let td = flowcutter_td(
        &formula,
        GraphKind::Primal,
        FcBudget::Timed {
            timeout_ms: 1,
            patience_ms: 0,
            iters: 4,
            steps: 20_000,
            cap_mode: WallCapMode::Tight,
        },
    )
    .expect("a wall that expires immediately still yields a decomposition");

    assert_valid_td(&td, n, &edges);
}
