//! The two smallest CNF types: a variable identifier and a literal over it.
//!
//! Everything else in this crate is built on them — clauses, formulas, the
//! preprocessing passes, and the vtree leaves that carry a [`VarId`] — so they
//! are defined here, at the bottom, and re-exported by the modules that use
//! them ([`crate::vtree`] among them).

use std::num::NonZeroU32;

/// A variable identifier: DIMACS variable `n`, for `n` at least 1.
///
/// Zero is not a variable — in DIMACS it ends a clause — and this type cannot
/// hold it. That is a type invariant rather than a check each caller repeats:
/// [`VarId::new`] and [`VarId::try_from_dimacs`] are where a number that might
/// be zero turns into a variable, and every other constructor takes one that
/// already cannot be. [`VarId::get`] reads the number back out.
///
/// The number is the one a `.cnf`, a `.vtree` or a record file writes, so
/// the crate's file readers and writers carry no offset. A table sized by
/// variables is indexed through [`VarId::idx`], which is `n - 1`, and a
/// variable recovered from such an index is [`VarId::from_idx`].
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd)]
pub struct VarId(NonZeroU32);

impl VarId {
    /// The variable numbered `n`, or `None` when `n` is 0, which names none.
    ///
    /// ```
    /// use vitri::cnf::VarId;
    /// assert_eq!(VarId::new(42).unwrap().get(), 42);
    /// assert_eq!(VarId::new(0), None);
    /// ```
    #[inline(always)]
    pub const fn new(n: u32) -> Option<Self> {
        match NonZeroU32::new(n) {
            Some(n) => Some(VarId(n)),
            None => None,
        }
    }

    /// This variable's number.
    ///
    /// ```
    /// use vitri::cnf::VarId;
    /// assert_eq!(VarId::from_dimacs(42).get(), 42);
    /// ```
    #[inline(always)]
    pub const fn get(self) -> u32 {
        self.0.get()
    }

    /// Every variable of a space of `num_vars` variables, ascending — 1
    /// through `num_vars`, and nothing for a space of none.
    ///
    /// ```
    /// use vitri::cnf::VarId;
    /// let vars: Vec<u32> = VarId::all(3).map(VarId::get).collect();
    /// assert_eq!(vars, [1, 2, 3]);
    /// assert_eq!(VarId::all(0).count(), 0);
    /// ```
    pub fn all(num_vars: u32) -> impl DoubleEndedIterator<Item = Self> + Clone {
        (1..=num_vars).map(|n| VarId(NonZeroU32::new(n).expect("a range from 1 holds no 0")))
    }

    /// The position of this variable in a table with one slot per variable:
    /// `n - 1`, the one place the offset is spelled.
    ///
    /// ```
    /// use vitri::cnf::VarId;
    /// assert_eq!(VarId::from_dimacs(1).idx(), 0);
    /// assert_eq!(VarId::from_dimacs(42).idx(), 41);
    /// ```
    #[inline(always)]
    pub const fn idx(self) -> usize {
        self.0.get() as usize - 1
    }

    /// The variable at position `idx` of a table with one slot per variable:
    /// the inverse of [`VarId::idx`].
    ///
    /// # Panics
    /// Panics when `idx` is `u32::MAX` or above, which no table this crate
    /// builds reaches: the variable after it has no 32-bit number.
    ///
    /// ```
    /// use vitri::cnf::VarId;
    /// assert_eq!(VarId::from_idx(0), VarId::from_dimacs(1));
    /// assert_eq!(VarId::from_idx(41).idx(), 41);
    /// ```
    #[inline(always)]
    pub fn from_idx(idx: usize) -> Self {
        let n = u32::try_from(idx + 1).unwrap_or_else(|_| panic!("no variable at index {idx}"));
        VarId(NonZeroU32::new(n).expect("idx + 1 is never zero"))
    }

    /// This variable's number as the signed integer DIMACS writes it, which
    /// is the number itself.
    ///
    /// ```
    /// use vitri::cnf::VarId;
    /// assert_eq!(VarId::from_dimacs(1).to_dimacs(), 1);
    /// assert_eq!(VarId::from_dimacs(42).to_dimacs(), 42);
    /// ```
    #[inline(always)]
    pub const fn to_dimacs(self) -> i32 {
        self.0.get() as i32
    }

    /// The variable a **DIMACS** integer names, whatever its sign: `1` and `-1`
    /// both name variable 1.
    ///
    /// For an integer this crate already trusts — one it wrote itself, or one a
    /// reader has validated. [`VarId::try_from_dimacs`] is the entry for one it
    /// has not.
    ///
    /// # Panics
    /// Panics on an integer that names no variable: see
    /// [`try_from_dimacs`](Self::try_from_dimacs).
    ///
    /// ```
    /// use vitri::cnf::VarId;
    /// assert_eq!(VarId::from_dimacs(-42), VarId::from_dimacs(42));
    /// ```
    #[inline(always)]
    pub fn from_dimacs(n: i32) -> Self {
        VarId::try_from_dimacs(n).unwrap_or_else(|| panic!("{n} names no DIMACS variable"))
    }

    /// The variable a **DIMACS** integer names, or `None` where it names none:
    /// `0`, which terminates a clause rather than naming anything, and
    /// `i32::MIN`, whose magnitude is one no DIMACS integer can write.
    ///
    /// THE entry for an integer read from a file, a record or a stored map —
    /// anything the crate has not already checked. A caller that has to answer
    /// for a malformed value keeps its own answer (drop the entry, reject the
    /// file) instead of inheriting the panic in [`VarId::from_dimacs`].
    ///
    /// ```
    /// use vitri::cnf::VarId;
    /// assert_eq!(VarId::try_from_dimacs(-42), VarId::new(42));
    /// assert_eq!(VarId::try_from_dimacs(0), None);
    /// ```
    #[inline(always)]
    pub fn try_from_dimacs(n: i32) -> Option<Self> {
        let named = n.checked_abs()?;
        VarId::new(named as u32)
    }
}

/// A literal: a variable with a polarity.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct Literal {
    /// The variable this literal refers to.
    pub var: VarId,
    /// `true` for a positive literal, `false` for a negated one.
    pub positive: bool,
}

impl Literal {
    /// Construct a literal over `var` with the given polarity.
    pub fn new(var: VarId, positive: bool) -> Self {
        Literal { var, positive }
    }

    /// The positive literal over `var`.
    pub fn pos(var: VarId) -> Self {
        Literal::new(var, true)
    }

    /// The negated literal over `var`.
    pub fn neg(var: VarId) -> Self {
        Literal::new(var, false)
    }

    /// This literal with its polarity flipped.
    #[must_use]
    pub fn negated(self) -> Self {
        Literal {
            var: self.var,
            positive: !self.positive,
        }
    }

    /// This literal as a signed **DIMACS** integer: the variable's number,
    /// negative for a negated literal. The inverse of this type's `From<i32>`
    /// conversion.
    ///
    /// ```
    /// use vitri::cnf::{Literal, VarId};
    /// assert_eq!(Literal::pos(VarId::from_dimacs(1)).to_dimacs(), 1);
    /// assert_eq!(Literal::neg(VarId::from_dimacs(2)).to_dimacs(), -2);
    /// ```
    pub fn to_dimacs(self) -> i32 {
        let var = self.var.to_dimacs();
        if self.positive { var } else { -var }
    }
}

/// Build a `Literal` from a signed **DIMACS** integer: the magnitude is the
/// variable, and a negative value denotes a negated literal.
///
/// # Panics
/// Panics on an integer that names no variable: see
/// [`VarId::try_from_dimacs`], which is the entry to use for an integer that
/// has not been validated yet.
///
/// ```
/// use vitri::cnf::{Literal, VarId};
/// assert_eq!(Literal::from(1), Literal::pos(VarId::from_dimacs(1)));
/// assert_eq!(Literal::from(-2), Literal::neg(VarId::from_dimacs(2)));
/// ```
impl From<i32> for Literal {
    fn from(n: i32) -> Self {
        let var = VarId::from_dimacs(n);
        if n > 0 {
            Literal::pos(var)
        } else {
            Literal::neg(var)
        }
    }
}

/// One variable merged away as equivalent to another: `eliminated ≡ survivor`,
/// where `survivor` is a literal, so a negative one means `eliminated` is
/// equivalent to the survivor's *negation*.
///
/// What a consumer owes the fold depends on what it is counting. An unweighted
/// one owes nothing beyond dropping `eliminated` from the set it counts over —
/// the survivor is counted in its place. A weighted one must additionally
/// multiply `eliminated`'s literal weights into the survivor's variable, since
/// that variable now stands for both; a negative survivor literal swaps the two
/// sides. [`Weights::fold_eliminated`](crate::cnf::Weights::fold_eliminated)
/// applies exactly that.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EquivFold {
    /// The variable that stops occurring.
    pub eliminated: VarId,
    /// The literal it is equivalent to, over a variable that survives.
    pub survivor: Literal,
}
