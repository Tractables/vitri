//! [`Renumber`]: the correspondence a contiguous renumbering leaves behind.
//!
//! Several preprocessing stages drop variables and then close the gaps so the
//! survivors occupy `0..K-1` again, each needing the same two lookups
//! afterwards — old id to new id, and new id back to old. They are built here,
//! once.
//!
//! What varies between the stages is how CLAUSES are rewritten, not how
//! variables are numbered — some refuse to drop a literal, others drop them
//! freely or replace a variable by its representative before renumbering at
//! all. So the numbering lives on [`Renumber`], and [`renumber_clauses`] is
//! the shared drop-literals rewrite for the stages whose rewrite really is
//! that.
//!
//! # Not a [`VarMap`](super::VarMap)
//!
//! [`VarMap`](super::VarMap) is the correspondence that reaches disk: signed,
//! 1-based DIMACS, the export record's `reduced_to_original_dimacs`. It can say
//! two things a `Renumber` cannot — that a variable stands for the NEGATION of
//! its counterpart (equivalent-literal replacement flips polarity), and that a
//! target variable was INTRODUCED by preprocessing and names no source variable
//! at all.
//!
//! `Renumber` is the internal counterpart, and the line between them is which of
//! those a stage needs. A stage that only DROPS variables and closes the gaps
//! never flips a polarity and never invents a variable, so its record is a
//! plain bijection between the variables it kept and `0..K-1`: unsigned,
//! 0-based, total on the kept set. That is a `Renumber`, and it stays inside the
//! crate. A boundary that has to serialize a correspondence, or a stage that
//! does flip polarities, produces a `VarMap`. Neither is expressed through the
//! other: the conversion would be lossy in one direction and meaningless in the
//! other.

use crate::cnf::VarId;
use crate::cnf::{Clause, CnfFormula, Literal};

/// A contiguous renumbering: which variables of an OLD formula survived into a
/// NEW one, and what each survivor is called on the other side.
///
/// Survivors keep their relative order — the `n`-th smallest old id that
/// survives is new id `n` — which is what makes a renumbering reproducible from
/// the kept set alone.
///
/// A stage that renumbered NOTHING says so with `None` in place of a
/// `Renumber`, never with an empty one: an empty `Renumber` is the renumbering
/// that kept no variable at all, which is a different statement.
#[derive(Clone, Debug)]
pub(crate) struct Renumber {
    /// Indexed by old variable id: the new id, or `None` if the variable did not
    /// survive. One entry per variable of the old formula.
    to_new: Vec<Option<VarId>>,
    /// Indexed by new variable id: the old variable it stands for. Strictly
    /// ascending, and exactly the kept set.
    to_old: Vec<VarId>,
}

impl Renumber {
    /// The renumbering that keeps the variables of an `num_old_vars`-variable
    /// formula that `keep` accepts.
    pub(crate) fn keeping(num_old_vars: usize, keep: impl Fn(VarId) -> bool) -> Self {
        Renumber::of_kept(
            num_old_vars,
            (0..num_old_vars as u32).map(VarId).filter(|&v| keep(v)),
        )
    }

    /// The renumbering that keeps exactly `kept`, which must be strictly
    /// ascending old ids below `num_old_vars` — the order a contiguous
    /// renumbering assigns new ids in.
    pub(crate) fn of_kept(num_old_vars: usize, kept: impl IntoIterator<Item = VarId>) -> Self {
        let mut to_new: Vec<Option<VarId>> = vec![None; num_old_vars];
        let mut to_old: Vec<VarId> = Vec::new();
        for old in kept {
            debug_assert!(
                old.idx() < num_old_vars,
                "kept variable {old:?} is not a variable of the old formula",
            );
            debug_assert!(
                to_old.last().is_none_or(|prev| prev.0 < old.0),
                "kept variables must be strictly ascending, got {old:?} after {:?}",
                to_old.last(),
            );
            to_new[old.idx()] = Some(VarId(to_old.len() as u32));
            to_old.push(old);
        }
        Renumber { to_new, to_old }
    }

    pub(crate) fn num_old_vars(&self) -> usize {
        self.to_new.len()
    }

    pub(crate) fn num_new_vars(&self) -> u32 {
        self.to_old.len() as u32
    }

    /// What old variable `old` is called in the new formula, or `None` if it did
    /// not survive (and `None` too for an id the old formula did not have).
    pub(crate) fn new_id(&self, old: VarId) -> Option<VarId> {
        self.to_new.get(old.idx()).copied().flatten()
    }

    pub(crate) fn old_id(&self, new: VarId) -> VarId {
        self.to_old[new.idx()]
    }

    /// The kept old variables, in new-id order: `kept()[n]` is new variable `n`.
    pub(crate) fn kept(&self) -> &[VarId] {
        &self.to_old
    }

    /// This renumbering followed by `inner`, which renumbers the new space this
    /// one produced: the result names each of `inner`'s new variables by the old
    /// variable of `self` it ultimately stands for.
    pub(crate) fn compose(&self, inner: &Renumber) -> Renumber {
        debug_assert_eq!(
            inner.num_old_vars(),
            self.num_new_vars() as usize,
            "`inner` must renumber the new space this renumbering produced",
        );
        // Ascending in, ascending out: `old_id` is strictly increasing, so the
        // composed kept set is still the ordering `of_kept` requires.
        Renumber::of_kept(
            self.num_old_vars(),
            inner.kept().iter().map(|&mid| self.old_id(mid)),
        )
    }

    /// `lit` renumbered into the new formula, or `None` if its variable did not
    /// survive. Polarity is never touched — a renumbering does not flip.
    pub(crate) fn apply_lit(&self, lit: Literal) -> Option<Literal> {
        self.new_id(lit.var).map(|v| Literal::new(v, lit.positive))
    }

    /// `lit`, read in the new formula, named back in the old one.
    pub(crate) fn apply_inverse_lit(&self, lit: Literal) -> Literal {
        Literal::new(self.old_id(lit.var), lit.positive)
    }
}

/// Renumber `clauses` (over `num_old_vars` variables) onto the variables `keep`
/// accepts, DROPPING the literals of the ones it does not.
///
/// The rewrite for stages that eliminate a variable by resolving it away: every
/// clause that mentioned it is already gone, so a literal still naming one is a
/// literal the elimination made redundant.
///
/// A clause that loses every literal is a genuine UNSAT certificate — it was
/// already the empty clause, since a sound elimination removes the clauses that
/// mention what it eliminated. Such a clause is PRESERVED, as a single empty
/// clause appended to the result, so the caller's has-empty-clause check fires
/// and settles count 0. Filtering it out instead silently turns an unsatisfiable
/// formula into a nonzero count.
pub(crate) fn renumber_clauses(
    num_old_vars: usize,
    clauses: Vec<Clause>,
    keep: impl Fn(VarId) -> bool,
) -> (CnfFormula, Renumber) {
    let renumbering = Renumber::keeping(num_old_vars, keep);

    let mut new_clauses: Vec<Clause> = Vec::with_capacity(clauses.len());
    let mut unsat = false;
    for clause in clauses {
        let new_lits: Vec<Literal> = clause
            .literals
            .iter()
            .filter_map(|&lit| renumbering.apply_lit(lit))
            .collect();
        if new_lits.is_empty() {
            unsat = true;
        } else {
            new_clauses.push(Clause::new(new_lits));
        }
    }
    if unsat {
        new_clauses.push(Clause::new(Vec::new()));
    }

    let formula = CnfFormula {
        num_vars: renumbering.num_new_vars(),
        clauses: new_clauses,
    };
    (formula, renumbering)
}
