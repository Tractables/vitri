//! The search under the construction meter, which is private to the crate.
//! What the search selects is tested over the public entry points, in
//! `src/tests/decompose/td_to_vtree/`.

use std::time::{Duration, Instant};

use crate::decompose::td_to_vtree::{Binarization, Place, Reading, Root};
use crate::tests::common::star_td;

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

#[test]
fn a_shared_memo_changes_neither_the_tree_a_conversion_returns_nor_what_it_charges() {
    use crate::decompose::{
        meter,
        td_to_vtree::{ConversionMemo, ConversionRequest, convert_td},
    };
    let (td, formula) = star_td();
    let shared = ConversionMemo::new(&formula);
    let run = |memo: Option<&ConversionMemo<'_>>| {
        let _clock = meter::arm(Instant::now());
        let before = meter::units_spent();
        let built = convert_td(
            &formula,
            &td,
            ConversionRequest {
                memo,
                ..ConversionRequest::open(Reading::default(), None)
            },
        );
        (built.vtree.to_vtree_text(), meter::units_spent() - before)
    };
    let own = run(None);
    assert_eq!(run(Some(&shared)), own, "a conversion filling the memo");
    assert_eq!(run(Some(&shared)), own, "a conversion answered from it");
}
