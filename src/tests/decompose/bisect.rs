//! The split a partitioner's answer describes.

use crate::decompose::Bisection;

/// A partitioner answers with one side bit per variable. A vector of some other
/// length is not an answer about this subset: read pairwise, it drops every
/// variable past its end, and the vtree built from the split would be missing
/// them — reported as no split at all, which the framework already recovers
/// from.
#[test]
fn a_side_bit_vector_shorter_than_the_subset_is_not_a_split() {
    let vars = [10u32, 11, 12, 13];

    assert!(
        Bisection::from_side_bits(&vars, &[0, 1]).is_none(),
        "two bits say nothing about the last two variables",
    );
    assert!(
        Bisection::from_side_bits(&vars, &[0, 1, 0, 1, 0]).is_none(),
        "a bit past the end of the subset belongs to another question",
    );
    assert!(
        Bisection::from_side_bits(&vars, &[]).is_none(),
        "no bits at all is no split",
    );

    // One bit per variable, both sides used: the answer, with each side in the
    // order the subset listed them.
    let split = Bisection::from_side_bits(&vars, &[0, 1, 1, 0]).expect("one bit per variable");
    assert_eq!(split.left, vec![10, 13]);
    assert_eq!(split.right, vec![11, 12]);
}

/// The type exists so the recursion can descend into both sides without
/// checking either: a partition putting everything on one side is not a split.
#[test]
fn every_variable_on_one_side_is_not_a_split() {
    let vars = [10u32, 11, 12];
    assert!(Bisection::from_side_bits(&vars, &[0, 0, 0]).is_none());
    assert!(Bisection::from_side_bits(&vars, &[1, 1, 1]).is_none());
    assert!(Bisection::from_side_bits(&vars, &[0, 0, 1]).is_some());
}
