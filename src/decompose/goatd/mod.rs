//! CNF and vtree adapters for the goatd decomposition library.

mod elimination;
mod polishing;
mod sat_score;
mod schedule;
pub use polishing::{GoatdLift, GoatdPolishing, GoatdCandidateReading};

pub(crate) use elimination::{
    INTERNAL_ELIMINATION_SEED, MINFILL_SPEC, VIEW_SUFFIXES, elimination_order_samples,
    elimination_spec, elimination_spec_names, minfill_td_from_edges, vtree_from_elimination,
    vtree_from_minfill,
};
pub use schedule::GoatdKnobs;
pub(crate) use schedule::{
    MAX_GOATD_CANDIDATES, candidate_param, vtree_from_goatd, vtrees_from_goatd_refined,
};
