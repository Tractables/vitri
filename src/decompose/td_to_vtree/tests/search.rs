//! The search under the construction meter, which is private to the crate.
//! What the search selects is tested over the public entry points, in
//! `src/tests/decompose/td_to_vtree/`.

use std::time::{Duration, Instant};

use crate::cnf::CnfFormula;
use crate::decompose::TreeDecomposition;
use crate::decompose::td_to_vtree::{Binarization, Place, Reading, Root};
use crate::tests::common::{make_formula, make_td};

/// A star decomposition: a hub bag holding variable 0, and one leaf bag per
/// other variable. Eight leaf bags is enough for the screen to have several
/// candidate roots to rank, so both halves of the search run.
fn star_td() -> (TreeDecomposition, CnfFormula) {
    let num_vars = 9;
    let mut bags = vec![vec![0u32]];
    let mut edges = Vec::new();
    for v in 1..num_vars {
        bags.push(vec![0, v]);
        edges.push((0usize, v as usize));
    }
    let clauses: Vec<Vec<i32>> = (1..num_vars).map(|v| vec![1, v as i32 + 1]).collect();
    (
        make_td(bags, edges, num_vars),
        make_formula(num_vars, clauses),
    )
}

#[test]
fn a_real_cutoff_bounds_conversion_even_when_the_work_clock_is_armed() {
    use crate::decompose::{
        meter,
        td_to_vtree::{ConversionRequest, convert_td},
    };
    let (td, formula) = star_td();
    let run = |request| {
        let _clock = meter::arm(Instant::now());
        let before = meter::units_spent();
        let built = convert_td(&formula, &td, request);
        (built.vtree.to_vtree_text(), meter::units_spent() - before)
    };
    let first = run(ConversionRequest::open(
        Reading {
            root: Some(Root::First),
            place: Some(Place::Shallow),
            binarize: Some(Binarization::Edge),
        },
        None,
    ));
    let expired = run(ConversionRequest {
        real_deadline: Some(Instant::now() - Duration::from_secs(1)),
        ..ConversionRequest::open(Reading::default(), None)
    });
    assert_eq!(
        expired, first,
        "a real cutoff permits exactly the first complete reading"
    );
}
