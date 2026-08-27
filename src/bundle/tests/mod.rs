//! Tests of items `bundle` keeps to itself. Everything reachable from the
//! crate root is tested from `src/tests/bundle/` instead; this tree is for the
//! `pub(super)` and `pub(crate)` decisions the chains make internally, which
//! the privacy rules put out of reach from there.

mod chains;
mod stage_budget;

#[test]
fn full_run_selection_uses_the_profile_owned_by_the_run() {
    let measured = crate::score::StructureProfile::from_coefficients(0.25, 0.5);
    let wrong = crate::score::StructureProfile::from_coefficients(9.0, 9.0);
    let mut caller = crate::decompose::SelectionCtx::plain();
    caller.source_profile = Some(wrong);

    let selection = super::run_selection(&caller, measured, None, 4);

    assert_eq!(
        selection.source_profile,
        Some(measured),
        "the construction path must receive the profile measured by run",
    );
}
