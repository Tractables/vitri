//! The two smallest CNF types: a variable identifier and a literal over it.
//!
//! Everything else in this crate is built on them — clauses, formulas, the
//! preprocessing passes, and the vtree leaves that carry a [`VarId`] — so they
//! are defined here, at the bottom, and re-exported by the modules that use
//! them ([`crate::vtree`] among them).

/// A variable identifier: `VarId(n)` is DIMACS variable `n`, and `VarId(0)`
/// is not a variable.
///
/// The number is the one a `.cnf`, a `.vtree` or a record file writes, so
/// the crate's file readers and writers carry no offset. A table sized by
/// variables is indexed through [`VarId::idx`], which is `n - 1`, and a
/// variable recovered from such an index is [`VarId::from_idx`].
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd)]
pub struct VarId(pub u32);

impl VarId {
    /// The position of this variable in a table with one slot per variable:
    /// `n - 1`, the one place the offset is spelled.
    ///
    /// ```
    /// use vitri::cnf::VarId;
    /// assert_eq!(VarId(1).idx(), 0);
    /// assert_eq!(VarId(42).idx(), 41);
    /// ```
    #[inline(always)]
    pub fn idx(self) -> usize {
        debug_assert!(self.0 >= 1, "VarId(0) is not a variable");
        self.0 as usize - 1
    }

    /// The variable at position `idx` of a table with one slot per variable:
    /// the inverse of [`VarId::idx`].
    ///
    /// ```
    /// use vitri::cnf::VarId;
    /// assert_eq!(VarId::from_idx(0), VarId(1));
    /// assert_eq!(VarId::from_idx(41).idx(), 41);
    /// ```
    #[inline(always)]
    pub fn from_idx(idx: usize) -> Self {
        VarId(idx as u32 + 1)
    }

    /// This variable's number as the signed integer DIMACS writes it, which
    /// is the number itself.
    ///
    /// ```
    /// use vitri::cnf::VarId;
    /// assert_eq!(VarId(1).to_dimacs(), 1);
    /// assert_eq!(VarId(42).to_dimacs(), 42);
    /// ```
    #[inline(always)]
    pub fn to_dimacs(self) -> i32 {
        self.0 as i32
    }

    /// The variable a **DIMACS** integer names, whatever its sign: `1` and `-1`
    /// both name `VarId(1)`.
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
    /// assert_eq!(VarId::from_dimacs(1), VarId(1));
    /// assert_eq!(VarId::from_dimacs(-42), VarId(42));
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
    /// assert_eq!(VarId::try_from_dimacs(-42), Some(VarId(42)));
    /// assert_eq!(VarId::try_from_dimacs(0), None);
    /// ```
    #[inline(always)]
    pub fn try_from_dimacs(n: i32) -> Option<Self> {
        let named = n.checked_abs()?;
        (named != 0).then_some(VarId(named as u32))
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
    /// assert_eq!(Literal::pos(VarId(1)).to_dimacs(), 1);
    /// assert_eq!(Literal::neg(VarId(2)).to_dimacs(), -2);
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
/// assert_eq!(Literal::from(1), Literal::pos(VarId(1)));
/// assert_eq!(Literal::from(-2), Literal::neg(VarId(2)));
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
