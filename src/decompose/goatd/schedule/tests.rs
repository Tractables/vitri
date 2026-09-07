use ::goatd::portfolio::Stage;

use super::{CandidateKind, varied_prefix};

const SAMPLE: CandidateKind = CandidateKind {
    stage: Stage::Sample,
    hedged: false,
};
const MIN_FILL_HEDGED: CandidateKind = CandidateKind {
    stage: Stage::MinFill,
    hedged: true,
};
const MIN_DEGREE: CandidateKind = CandidateKind {
    stage: Stage::MinDegree,
    hedged: false,
};

/// Goatd's order, as a run with a hedge of one stage lists it: the pick, its
/// hedge's passes, then the sampled restarts and a min-degree tree.
const RUN: [(CandidateKind, u8); 9] = [
    (MIN_DEGREE, 0),
    (MIN_FILL_HEDGED, 1),
    (MIN_FILL_HEDGED, 2),
    (MIN_FILL_HEDGED, 3),
    (MIN_FILL_HEDGED, 4),
    (SAMPLE, 5),
    (SAMPLE, 6),
    (MIN_DEGREE, 7),
    (MIN_FILL_HEDGED, 8),
];

fn prefix(count: usize) -> Vec<u8> {
    varied_prefix(RUN.to_vec(), count, |&(kind, _)| kind)
        .into_iter()
        .map(|(_, index)| index)
        .collect()
}

/// The pick leads, then each kind in turn in goatd's order until the slots
/// are full; kinds that run out are skipped.
#[test]
fn the_slots_go_round_the_kinds_after_the_pick() {
    assert_eq!(prefix(1), [0]);
    assert_eq!(prefix(2), [0, 1]);
    assert_eq!(prefix(4), [0, 1, 5, 7]);
    assert_eq!(prefix(6), [0, 1, 5, 7, 2, 6]);
    assert_eq!(prefix(8), [0, 1, 5, 7, 2, 6, 3, 4]);
}

/// A run with no more candidates than slots is offered as it came.
#[test]
fn a_run_that_fits_keeps_its_order() {
    assert_eq!(prefix(9), [0, 1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(prefix(20), [0, 1, 2, 3, 4, 5, 6, 7, 8]);
}
