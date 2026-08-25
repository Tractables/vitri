//! Tree decomposition to vtree conversion.
//!
//! The construction that carries a decomposition's width over to the tree: each
//! bag becomes a subtree, so a narrow decomposition converts to a vtree with
//! narrow separators.
//!
//! One decomposition names many vtrees, so a conversion is a SEARCH over
//! [`Reading`]s of it — see `search`, which every family in this crate reaches
//! through [`convert_td`].

mod algo;
mod combiners;
mod meta;
mod reading;
mod search;

pub(crate) use algo::ConversionInput;
pub use meta::BagMetadata;
pub(crate) use reading::{FOLDS, PLACES, ROOTS};
pub use reading::{Fold, Place, Reading, Root};
pub(crate) use search::{ConversionRequest, TdConversionMeta};

use std::sync::Arc;
use std::time::Instant;

use super::{TdConversion, TreeDecomposition};
use crate::cnf::CnfFormula;
use crate::vtree::Vtree;

/// Convert a tree decomposition into a vtree over variables `1..=num_vars`,
/// under the one reading a conversion with nothing to score against can pick.
///
/// Where a decomposition from any source — [`parse_pace_td`](crate::decompose::parse_pace_td),
/// a solver this crate does not wrap, one you built yourself — becomes a vtree.
/// Use [`td_to_vtree_reading`] to hand over the CNF, which is what lets the
/// conversion search readings and keep the cheapest, or to name a reading
/// yourself.
pub fn td_to_vtree(td: &TreeDecomposition, num_vars: u32) -> Vtree {
    td_to_vtree_reading(td, num_vars, Reading::default(), None, None)
}

/// The TD → vtree conversion with the reading in the caller's hands: every
/// dimension of `reading` left `None` is one this searches over, scoring each
/// tree against `formula` and keeping the cheapest.
///
/// `formula` is what makes the search possible and what the clause-aware folds
/// read. Passing `None` leaves nothing to score and nothing to order by, so the
/// conversion builds exactly one reading whatever was left open. Pass
/// `Some(formula)` whenever the decomposition came from a CNF that is still in
/// hand: this is then the same conversion, under the same rule, that every
/// construction in this crate reaches.
///
/// `deadline` bounds the search, never its result: it is tested between
/// readings and only once one has been adopted, so an already-expired deadline
/// still returns the first reading's tree.
///
/// Runs at the baseline construction effort. The folds that bisect scale how
/// hard they search with the wall-clock hint a whole run was given, which a
/// single conversion of a decomposition already in hand has no share of.
pub fn td_to_vtree_reading(
    td: &TreeDecomposition,
    num_vars: u32,
    reading: Reading,
    formula: Option<&CnfFormula>,
    deadline: Option<Instant>,
) -> Vtree {
    search::convert(
        ConversionInput {
            td,
            num_vars,
            formula,
            effort_scale: 1.0,
        },
        ConversionRequest::open(reading, deadline),
    )
    .0
}

/// THE conversion every construction in this crate reaches: search `td` the way
/// `request` asks, and pair the winning tree with the bag metadata describing
/// IT.
///
/// The pairing is written once, here, so no backend can hand back a tree with a
/// different reading's metadata beside it.
pub(crate) fn convert_td(
    formula: &CnfFormula,
    td: &TreeDecomposition,
    request: ConversionRequest<'_>,
) -> TdConversion {
    let (vtree, td_info) = search::convert(
        ConversionInput {
            td,
            num_vars: formula.num_vars,
            formula: Some(formula),
            effort_scale: request.effort_scale,
        },
        request,
    );
    TdConversion {
        vtree: Arc::new(vtree),
        td: td_info,
    }
}

#[cfg(test)]
mod tests;
