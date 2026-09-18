//! The construction deadline, at the two families that had been building past
//! it: recursive bisection, which stops at the deadline, and a single
//! elimination order, which bounds its pass by what is left of it.

use std::time::{Duration, Instant};

use crate::cnf::{Clause, CnfFormula, Literal};
use crate::decompose::{
    BisectDials, ConversionRequest, INTERNAL_ELIMINATION_SEED, Reading,
    guided_bisect_from_incidence_td, vtree_from_elimination, vtree_from_hg_bisect,
};
use crate::tests::td_fixture::make_test_td;
use crate::vtree::VarId;

/// A path formula over the six variables the shared decomposition fixture
/// covers.
fn path_formula() -> CnfFormula {
    let edge = |a: u32, b: u32| {
        Clause::new(vec![
            Literal {
                var: VarId::new(a).unwrap(),
                positive: true,
            },
            Literal {
                var: VarId::new(b).unwrap(),
                positive: false,
            },
        ])
    };
    CnfFormula::from_parts(6, (1..=5).map(|v| edge(v, v + 1)).collect())
}

fn already_passed() -> Instant {
    Instant::now() - Duration::from_secs(1)
}

/// The recursion checks the deadline before every split, so a construction
/// handed one that has already passed reports that and builds nothing. The
/// solvers carry it on their dials, which is the only way the framework can see
/// it.
#[test]
fn a_bisection_past_its_deadline_builds_no_vtree() {
    let formula = path_formula();
    let dials = BisectDials {
        imbalance: 0.30,
        base_seed: 0,
        deadline: Some(already_passed()),
    };

    let hg = vtree_from_hg_bisect(&formula, dials, 1.0)
        .expect_err("the hypergraph bisection is past its deadline");
    assert!(hg.contains("deadline"), "{hg}");

    let td = make_test_td();
    let Err(guided) = guided_bisect_from_incidence_td(
        &formula,
        &td,
        ConversionRequest::open(Reading::default(), Some(already_passed())),
    ) else {
        panic!("the guided bisection is past its deadline");
    };
    assert!(guided.contains("deadline"), "{guided}");
}

/// An elimination build caps its own ceiling at what is left of the
/// construction deadline, and still answers once that deadline has passed: a
/// caller that asked for a named construction under a budget too small for it
/// gets the tree late rather than nothing at all. What the pass is allowed to
/// spend is pinned beside the module.
#[test]
fn an_elimination_order_past_its_deadline_still_builds() {
    let formula = path_formula();
    let request = ConversionRequest::open(Reading::default(), Some(already_passed()));
    assert!(
        vtree_from_elimination(
            &formula,
            "minfill",
            false,
            false,
            INTERNAL_ELIMINATION_SEED,
            request,
        )
        .is_ok()
    );
}

/// The same two constructions with no deadline at all still build, which is
/// what the dial's `None` means.
#[test]
fn an_unbounded_construction_still_builds() {
    let formula = path_formula();
    let dials = BisectDials {
        imbalance: 0.30,
        base_seed: 0,
        deadline: None,
    };
    assert!(vtree_from_hg_bisect(&formula, dials, 1.0).is_ok());
    assert!(
        vtree_from_elimination(
            &formula,
            "minfill",
            false,
            false,
            INTERNAL_ELIMINATION_SEED,
            ConversionRequest::open(Reading::default(), None),
        )
        .is_ok()
    );
}
