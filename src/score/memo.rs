//! Costs already computed against one formula, so a tree met again is not
//! scored again.
//!
//! A refinement loop converts one proposal after another, and each conversion
//! scores several readings of its decomposition. Proposals that differ little
//! convert to many of the same trees, so most of the scoring such a loop does is
//! scoring a tree it has scored before. [`CostMemo`] answers those from what it
//! kept.
//!
//! It keys on the tree as numbered, not on its shape: a tree is a hit only when
//! [`Vtree::identical_to`] says so, which is when [`vtree_cost`] cannot tell the
//! two apart. A hit therefore returns the very number scoring would have, and
//! whoever reads it decides exactly what it would have decided.

use std::cell::{Cell, RefCell};

use rustc_hash::FxHashMap;

use crate::cnf::CnfFormula;
use crate::error::VitriError;
use crate::vtree::Vtree;

use super::vtree_cost;

/// How many tree nodes a memo keeps, summed over the trees it holds. Past it,
/// trees already kept still answer and new ones are scored without being kept,
/// which bounds the memory a long loop over a large formula can take.
const NODE_CAP: usize = 1 << 20;

/// [`vtree_cost`] against one formula, remembered per tree.
pub(crate) struct CostMemo<'f> {
    formula: &'f CnfFormula,
    /// The trees kept and their costs, bucketed by [`Vtree::fingerprint`].
    kept: RefCell<FxHashMap<u64, Vec<(Vtree, f64)>>>,
    /// Nodes summed over the trees in `kept`, checked against [`NODE_CAP`].
    nodes: Cell<usize>,
}

impl<'f> CostMemo<'f> {
    /// A memo for trees scored against `formula`, holding nothing yet.
    pub(crate) fn new(formula: &'f CnfFormula) -> Self {
        CostMemo {
            formula,
            kept: RefCell::default(),
            nodes: Cell::new(0),
        }
    }

    /// [`vtree_cost`] of `vtree` against `formula`: the kept number when this
    /// tree was scored before, a fresh one otherwise.
    ///
    /// The memo is about the formula it was made for. Any other is scored
    /// directly, and what it scores is not kept.
    ///
    /// # Errors
    ///
    /// Whatever [`vtree_cost`] returns, which is never kept.
    pub(crate) fn cost(&self, vtree: &Vtree, formula: &CnfFormula) -> Result<f64, VitriError> {
        if !std::ptr::eq(formula, self.formula) {
            return vtree_cost(vtree, formula);
        }
        let key = vtree.fingerprint();
        if let Some(bucket) = self.kept.borrow().get(&key)
            && let Some(&(_, cost)) = bucket.iter().find(|(kept, _)| kept.identical_to(vtree))
        {
            return Ok(cost);
        }
        let cost = vtree_cost(vtree, formula)?;
        let nodes = self.nodes.get() + vtree.num_nodes();
        if nodes <= NODE_CAP {
            self.nodes.set(nodes);
            self.kept
                .borrow_mut()
                .entry(key)
                .or_default()
                .push((vtree.clone(), cost));
        }
        Ok(cost)
    }
}
