//! Preprocessing passes tested through items their own modules keep to
//! themselves. A pass with its own directory carries its tests there instead;
//! what reaches only the crate root is tested from `src/tests/preprocess/`.

use crate::cnf::{Clause, CnfFormula, Literal, Reduced, ShowSet, VarId};
use crate::config::PreprocessClock;
use crate::preprocess::meter::PreprocessMeter;

/// The meter every pass takes, on the clock a test wants: wall time, so a
/// budget means what it says and no decision trace is kept.
pub(super) fn wall_meter() -> PreprocessMeter {
    PreprocessMeter::new(PreprocessClock::WallClock)
}

/// Independent blocks on which projected BVE does a great deal of work for
/// little output, so a deadline can land in the middle of it.
///
/// Block `b` has `width` show variables and one hidden variable `h`, and, for
/// each of `patterns` distinct sign patterns `B` over its show variables, the
/// clauses `h ∨ B` and `¬h ∨ B`. Two different patterns disagree on some show
/// variable, so their resolvent on `h` is a tautology: eliminating `h` visits
/// all `patterns²` pairs to find the `patterns` resolvents `B`. Those fit the
/// no-growth bound, so `h` goes and the block is left as its patterns.
///
/// A block is therefore, after the pass, exactly one of two clause sets:
/// [`Self::block`] or [`Self::patterns_of`]. That is what lets a test read a
/// partial elimination off the output.
pub(super) struct PatternBlocks {
    pub(super) blocks: u32,
    pub(super) width: u32,
    pub(super) patterns: u32,
}

impl PatternBlocks {
    pub(super) fn num_vars(&self) -> u32 {
        self.blocks * (self.width + 1)
    }

    /// The block a variable belongs to.
    pub(super) fn block_of(&self, var: VarId) -> u32 {
        var.idx() as u32 / (self.width + 1)
    }

    /// Block `b`'s hidden variable, the last of its `width + 1`.
    fn hidden(&self, b: u32) -> VarId {
        VarId::from_idx(((b + 1) * (self.width + 1) - 1) as usize)
    }

    /// Pattern `i` over block `b`'s show variables: the `j`-th is positive
    /// when bit `j` of `i` is set. Sorted, as the pass keeps its clauses.
    fn pattern(&self, b: u32, i: u32) -> Vec<Literal> {
        (0..self.width)
            .map(|j| {
                let var = VarId::from_idx((b * (self.width + 1) + j) as usize);
                Literal::new(var, (i >> j) & 1 == 1)
            })
            .collect()
    }

    /// Block `b` as built: `h ∨ B` and `¬h ∨ B` for every pattern.
    pub(super) fn block(&self, b: u32) -> Vec<Vec<Literal>> {
        let h = self.hidden(b);
        (0..self.patterns)
            .flat_map(|i| {
                [true, false].map(|positive| {
                    let mut c = self.pattern(b, i);
                    c.push(Literal::new(h, positive));
                    c
                })
            })
            .collect()
    }

    /// Block `b` once its hidden variable is eliminated: its patterns.
    pub(super) fn patterns_of(&self, b: u32) -> Vec<Vec<Literal>> {
        (0..self.patterns).map(|i| self.pattern(b, i)).collect()
    }

    pub(super) fn formula(&self) -> CnfFormula {
        let clauses = (0..self.blocks)
            .flat_map(|b| self.block(b))
            .map(Clause::new)
            .collect();
        CnfFormula::from_parts(self.num_vars(), clauses)
    }

    /// Every variable but the hidden ones.
    pub(super) fn show(&self) -> ShowSet<Reduced> {
        ShowSet::from_vars(
            VarId::all(self.num_vars()).filter(|&v| v != self.hidden(self.block_of(v))),
        )
    }
}

mod backbone_pipeline;
mod bve_project;
mod cadical;
mod cadical_ffi;
mod count_preserve;
mod equivalence;
mod fork_budget;
mod gates;
mod meter;
mod pipelines;
mod probe_engine;
mod projected;
mod renumber;
mod simplify;
mod tarjan;
mod unit_propagation;
mod weighted_lift;
