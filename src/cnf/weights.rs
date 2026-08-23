//! The literal weights a weighted count weighs each assignment by.
//!
//! # Three types, and why the split
//!
//! [`Weights`] is the working form and the one everything downstream reads:
//! `(w⁻, w⁺)` for EVERY variable of one named formula, unspecified literals
//! already resolved to 1. It is keyed by a [`Space`] marker, so a table
//! expressed over one formula's variables cannot be handed to a consumer
//! expecting another's — and the only way to change that space is
//! [`VarMap::carry_weights`](crate::preprocess::VarMap::carry_weights), which
//! swaps the two polarities when the correspondence does.
//!
//! [`WeightTable`] is what `c p weight` parses into, and it is SPARSE on
//! purpose: it records which literals the file actually named. The weighted
//! projected reduction is handed the DECLARED literals only, and a table of
//! every literal (weight 1 included) would mean something else there — so that
//! list cannot be reconstructed from a resolved [`Weights`], and the parse
//! table survives as its own type with one exit, [`WeightTable::resolve`].
//!
//! [`LiteralWeight`] is one written row, the form the `c p weight` lines and
//! `preprocess.json`'s `reduced_weights` array both carry.
//!
//! # Numbering
//!
//! [`Weights`] is indexed by 0-based [`VarId`]. The 1-based signed DIMACS form
//! every written artifact carries exists only at this module's edges — the parse
//! table's own writer and [`Weights::from_dimacs_pairs`] on the way in,
//! [`WeightTable::to_literal_pairs`], [`Weights::to_dimacs_pairs`] and
//! [`Weights::to_record_rows`] on the way out. No other module adds or
//! subtracts the one on a weight literal.

use std::marker::PhantomData;
use std::ops::Index;

use num_rational::BigRational;
use num_traits::One;
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};

use super::rational_string;
use super::space::{Original, Reduced, Space};
use super::{EquivFold, Literal, VarId};

/// Both DIMACS literals over 0-based variable `i`, positive first — the pair
/// and the order every flattened listing of a table walks.
fn dimacs_literals(i: usize) -> [i32; 2] {
    let var = VarId(i as u32);
    [Literal::pos(var).to_dimacs(), Literal::neg(var).to_dimacs()]
}

/// Per-variable literal weights parsed from `c p weight <lit> <w> 0` lines.
///
/// `w_pos[v]` / `w_neg[v]` hold the exact-rational weight of the positive /
/// negative literal of variable `v`, or `None` if the instance left that
/// literal unspecified (resolved to 1 by [`WeightTable::resolve`], the MCC
/// default).
#[derive(Clone, Debug, Default)]
pub struct WeightTable {
    w_pos: Vec<Option<BigRational>>,
    w_neg: Vec<Option<BigRational>>,
}

impl WeightTable {
    fn ensure(&mut self, var: usize) {
        if self.w_pos.len() <= var {
            self.w_pos.resize(var + 1, None);
            self.w_neg.resize(var + 1, None);
        }
    }

    /// Weigh `lit` (a nonzero signed DIMACS literal).
    ///
    /// The table grows to hold `|lit|`, so the caller owes it a literal inside
    /// the instance's declared variable space.
    pub(super) fn set(&mut self, lit: i32, w: BigRational) {
        let var = VarId::from_dimacs(lit).idx();
        self.ensure(var);
        if lit > 0 {
            self.w_pos[var] = Some(w);
        } else {
            self.w_neg[var] = Some(w);
        }
    }

    /// Flatten back to `(signed DIMACS literal, weight)` pairs — exactly the
    /// literals that carried an explicit `c p weight` line (unspecified
    /// literals omitted, defaulting to 1 downstream). Order is by variable then
    /// (positive, negative); consumers must be order-independent.
    pub fn to_literal_pairs(&self) -> Vec<(i32, BigRational)> {
        let mut out = Vec::new();
        let n = self.w_pos.len().max(self.w_neg.len());
        for v in 0..n {
            let [pos, neg] = dimacs_literals(v);
            if let Some(Some(w)) = self.w_pos.get(v) {
                out.push((pos, w.clone()));
            }
            if let Some(Some(w)) = self.w_neg.get(v) {
                out.push((neg, w.clone()));
            }
        }
        out
    }

    /// The table this file declares, read over its own variables: every one of
    /// `num_vars` present, each unspecified literal defaulted to weight 1 (the
    /// MCC convention).
    ///
    /// THE exit from the sparse parse table into the working form. `S` is the
    /// space of the FILE this table was parsed from: an input CNF's own
    /// variables, or `reduced.cnf`'s when that is what was read.
    pub fn resolve<S: Space>(&self, num_vars: usize) -> Weights<S> {
        Weights(
            (0..num_vars)
                .map(|v| {
                    let wn = self
                        .w_neg
                        .get(v)
                        .and_then(|o| o.clone())
                        .unwrap_or_else(BigRational::one);
                    let wp = self
                        .w_pos
                        .get(v)
                        .and_then(|o| o.clone())
                        .unwrap_or_else(BigRational::one);
                    (wn, wp)
                })
                .collect(),
            PhantomData,
        )
    }
}

/// One literal's weight, as written by a `c p weight <lit> <w> 0` line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteralWeight {
    /// Signed 1-based DIMACS literal in the REDUCED variable space.
    pub literal: i32,
    /// The exact weight as `"numerator/denominator"`, in lowest terms with a
    /// positive denominator. A string, not a float: the competition's precision
    /// category A requires exact arithmetic, and every weight this crate handles
    /// is an exact rational — writing it as an `f64` would silently round it.
    pub weight: String,
}

/// The literal weights a count over one named formula is taken under, in the
/// space `S` names: `(w⁻, w⁺)` per 0-based [`VarId`], every variable present.
///
/// `S` is a compile-time marker only — it costs nothing at runtime and appears
/// in no written artifact.
pub struct Weights<S: Space>(Vec<(BigRational, BigRational)>, PhantomData<S>);

impl<S: Space> Weights<S> {
    /// All-ones over `num_vars` — the unweighted instance expressed in the
    /// weighted ring, where every weighted formula in this crate degenerates to
    /// its integer counterpart (`w⁻ + w⁺ == 2`, `w⁺ == 1`).
    pub fn uniform(num_vars: usize) -> Self {
        Weights(
            vec![(BigRational::one(), BigRational::one()); num_vars],
            PhantomData,
        )
    }

    /// The table over no variables at all — what an unweighted track carries,
    /// where no stage may read a weight and none does.
    pub fn empty() -> Self {
        Weights(Vec::new(), PhantomData)
    }

    /// The table over `num_vars` built from `(signed DIMACS literal, weight)`
    /// pairs — the form the vendored reduction reports its own table in, and
    /// one of the two places a written weight literal becomes a variable.
    ///
    /// Sparse input is welcome: a literal no pair names keeps weight 1, which
    /// is the same default the `c p weight` convention already gives it. A pair
    /// naming a variable outside `num_vars`, or naming no variable at all, is
    /// dropped.
    pub fn from_dimacs_pairs(pairs: &[(i32, BigRational)], num_vars: usize) -> Self {
        let mut w = Self::uniform(num_vars);
        for (lit, val) in pairs {
            let Some(idx) = VarId::try_from_dimacs(*lit)
                .map(VarId::idx)
                .filter(|idx| *idx < num_vars)
            else {
                continue;
            };
            if *lit > 0 {
                w.0[idx].1 = val.clone();
            } else {
                w.0[idx].0 = val.clone();
            }
        }
        w
    }

    /// The table a variable correspondence carries onto its own variables: one
    /// entry per variable in order, `None` for a variable the correspondence
    /// does not name, which weighs 1 on both polarities.
    ///
    /// Built here rather than at the map so the neutral default has one owner.
    /// The caller that has such a correspondence is
    /// [`VarMap::carry_weights`](crate::preprocess::VarMap::carry_weights).
    pub(crate) fn from_carried(
        entries: impl IntoIterator<Item = Option<(BigRational, BigRational)>>,
    ) -> Self {
        Weights(
            entries
                .into_iter()
                .map(|e| e.unwrap_or_else(|| (BigRational::one(), BigRational::one())))
                .collect(),
            PhantomData,
        )
    }

    /// The table over `num_vars` read one literal at a time, `read` answering
    /// for a signed DIMACS literal — the shape a per-literal getter on a
    /// reduction has. Every variable is asked for both ways, positive first, and
    /// the first literal `read` cannot answer for abandons the whole table.
    pub(crate) fn try_from_dimacs_lits(
        num_vars: u32,
        mut read: impl FnMut(i32) -> Option<BigRational>,
    ) -> Option<Self> {
        let mut pairs = Vec::with_capacity(num_vars as usize);
        for v in 0..num_vars {
            let [pos, neg] = dimacs_literals(v as usize);
            let wp = read(pos)?;
            let wn = read(neg)?;
            pairs.push((wn, wp));
        }
        Some(Weights(pairs, PhantomData))
    }

    /// How many variables the table covers.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the table covers no variables.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `var`'s pair, or `None` for a variable this table does not cover.
    pub fn get(&self, var: VarId) -> Option<&(BigRational, BigRational)> {
        self.0.get(var.idx())
    }

    /// The table as `(w⁻, w⁺)` per 0-based variable, for a consumer that has to
    /// hand a contiguous table to its own arithmetic — an embedding counter
    /// builds its semiring from one. Everything inside this crate indexes by
    /// [`VarId`] instead, which cannot be given the wrong numbering by accident.
    pub fn as_pairs(&self) -> &[(BigRational, BigRational)] {
        &self.0
    }

    /// The variables whose two literal weights DIFFER.
    ///
    /// Eliminating such a variable is non-scalar — its contribution depends on
    /// the value it takes — so this is the set a caller freezes out of the
    /// stages that eliminate. Equal-weight variables, the default uniform ones
    /// included, stay eliminable.
    pub fn unequal_vars(&self) -> FxHashSet<VarId> {
        self.0
            .iter()
            .enumerate()
            .filter(|(_, (wn, wp))| wn != wp)
            .map(|(i, _)| VarId(i as u32))
            .collect()
    }

    /// The variables carrying a weight of their own — anything but 1 on at
    /// least one polarity. A variable weighing 1 both ways is indistinguishable
    /// from one no `c p weight` line ever mentioned, which is what makes this
    /// the set of variables a weighted count cannot treat as plain.
    pub fn weighted_vars(&self) -> impl Iterator<Item = VarId> + '_ {
        self.0
            .iter()
            .enumerate()
            .filter(|(_, (wn, wp))| !wn.is_one() || !wp.is_one())
            .map(|(i, _)| VarId(i as u32))
    }

    /// The table as `(signed DIMACS literal, weight)` pairs, both polarities of
    /// every variable listed explicitly so nothing depends on the receiver's
    /// own default.
    pub fn to_dimacs_pairs(&self) -> Vec<(i32, BigRational)> {
        self.0
            .iter()
            .enumerate()
            .flat_map(|(i, (wn, wp))| {
                let [pos, neg] = dimacs_literals(i);
                [(pos, wp.clone()), (neg, wn.clone())]
            })
            .collect()
    }

    /// The table as the rows a record and a `c p weight` header are written
    /// from — the same literals in the same order as [`Self::to_dimacs_pairs`],
    /// the weights as exact `numerator/denominator` strings.
    pub fn to_record_rows(&self) -> Vec<LiteralWeight> {
        self.0
            .iter()
            .enumerate()
            .flat_map(|(i, (wn, wp))| {
                let [pos, neg] = dimacs_literals(i);
                [
                    LiteralWeight {
                        literal: pos,
                        weight: rational_string(wp),
                    },
                    LiteralWeight {
                        literal: neg,
                        weight: rational_string(wn),
                    },
                ]
            })
            .collect()
    }

    /// Cover exactly `num_vars` variables: drop the tail beyond it, and give
    /// weight 1 to any variable the table never named.
    ///
    /// For the stages that PRESERVE variable ids while dropping the ones they
    /// eliminate, where the survivors are a prefix and no correspondence is
    /// needed to bring the weights onto them.
    pub fn resize_neutral(&mut self, num_vars: usize) {
        self.0
            .resize(num_vars, (BigRational::one(), BigRational::one()));
    }

    /// Multiply one eliminated variable's literal weights into the survivor
    /// that now stands for it: `e ≡ surv` multiplies `(w⁻, w⁺)` straight
    /// through, `e ≡ ¬surv` swaps them first. THE spelling of the
    /// anti-equivalence swap — getting it backwards is a silent miscount, so
    /// every fold in the crate goes through here.
    ///
    /// `pair` is the eliminated member's `(w⁻, w⁺)` by value rather than an
    /// index, because the callers read it from different places: the
    /// preprocessing-equivalence fold reads the UNFOLDED weights while writing
    /// into this table, the other two read this table itself.
    pub(crate) fn fold_into(&mut self, pair: (BigRational, BigRational), surv: Literal) {
        let (en, ep) = pair;
        let s = surv.var.idx();
        if surv.positive {
            self.0[s].0 *= en;
            self.0[s].1 *= ep;
        } else {
            self.0[s].0 *= ep;
            self.0[s].1 *= en;
        }
    }

    /// Fold a batch of [`EquivFold`]s, each eliminated member's own current
    /// weights going into its survivor.
    pub(crate) fn fold_eliminated(&mut self, folds: &[EquivFold]) {
        for f in folds {
            let pair = self.0[f.eliminated.idx()].clone();
            self.fold_into(pair, f.survivor);
        }
    }
}

impl Weights<Original> {
    /// Read this table as one over the REDUCED space, for the paths where the
    /// two spaces coincide because nothing was renumbered.
    ///
    /// The named alternative to a silent re-typing: every call site owes a
    /// one-line reason why the reduced formula carries the input's own variable
    /// ids. A site that cannot give one has a
    /// [`VarMap`](crate::preprocess::VarMap) to carry the table through instead.
    pub(crate) fn assume_reduced_identity(self) -> Weights<Reduced> {
        Weights(self.0, PhantomData)
    }
}

impl<S: Space> Index<VarId> for Weights<S> {
    type Output = (BigRational, BigRational);

    fn index(&self, var: VarId) -> &Self::Output {
        &self.0[var.idx()]
    }
}

impl<S: Space> Clone for Weights<S> {
    fn clone(&self) -> Self {
        Weights(self.0.clone(), PhantomData)
    }
}

impl<S: Space> std::fmt::Debug for Weights<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Weights").field(&self.0).finish()
    }
}

impl<S: Space> PartialEq for Weights<S> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<S: Space> Eq for Weights<S> {}
