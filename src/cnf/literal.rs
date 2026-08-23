//! The two smallest CNF types: a variable identifier and a literal over it.
//!
//! Everything else in this crate is built on them — clauses, formulas, the
//! preprocessing passes, and the vtree leaves that carry a [`VarId`] — so they
//! are defined here, at the bottom, and re-exported by the modules that use
//! them ([`crate::vtree`] among them).

/// A 0-indexed variable identifier.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd)]
pub struct VarId(pub u32);

impl VarId {
    /// The variable number as a `usize`.
    #[inline(always)]
    pub fn idx(self) -> usize {
        self.0 as usize
    }

    /// This variable's number in **DIMACS**: `var + 1`.
    ///
    /// The inverse of [`VarId::from_dimacs`] — together, the crate's one
    /// spelling of the 0-based↔1-based offset for a variable, as
    /// [`Literal::to_dimacs`] and `Literal`'s `From<i32>` are for a literal.
    ///
    /// ```
    /// use vitri::cnf::VarId;
    /// assert_eq!(VarId(0).to_dimacs(), 1);
    /// assert_eq!(VarId(41).to_dimacs(), 42);
    /// ```
    #[inline(always)]
    pub fn to_dimacs(self) -> i32 {
        self.0 as i32 + 1
    }

    /// The variable a **DIMACS** integer names, whatever its sign: `1` and `-1`
    /// both name `VarId(0)`.
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
    /// assert_eq!(VarId::from_dimacs(1), VarId(0));
    /// assert_eq!(VarId::from_dimacs(-42), VarId(41));
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
    /// assert_eq!(VarId::try_from_dimacs(-42), Some(VarId(41)));
    /// assert_eq!(VarId::try_from_dimacs(0), None);
    /// ```
    #[inline(always)]
    pub fn try_from_dimacs(n: i32) -> Option<Self> {
        let named = n.checked_abs()?;
        (named != 0).then(|| VarId(named as u32 - 1))
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

    /// This literal as a signed **DIMACS** integer: `±(var + 1)`, negative for a
    /// negated literal.
    ///
    /// The inverse of this type's `From<i32>` conversion; the offset itself is
    /// [`VarId::to_dimacs`].
    ///
    /// ```
    /// use vitri::cnf::{Literal, VarId};
    /// assert_eq!(Literal::pos(VarId(0)).to_dimacs(), 1);
    /// assert_eq!(Literal::neg(VarId(1)).to_dimacs(), -2);
    /// ```
    pub fn to_dimacs(self) -> i32 {
        let var = self.var.to_dimacs();
        if self.positive { var } else { -var }
    }
}

/// Build a `Literal` from a signed **DIMACS** integer.
///
/// DIMACS variables are 1-based: `1` is the first variable (`VarId(0)`), `2` the
/// second, and so on; a negative value denotes a negated literal. The magnitude
/// is decremented to the 0-based [`VarId`] used internally by
/// [`VarId::from_dimacs`].
///
/// # Panics
/// Panics on an integer that names no variable: see
/// [`VarId::try_from_dimacs`], which is the entry to use for an integer that
/// has not been validated yet.
///
/// ```
/// use vitri::cnf::{Literal, VarId};
/// assert_eq!(Literal::from(1), Literal::pos(VarId(0)));
/// assert_eq!(Literal::from(-2), Literal::neg(VarId(1)));
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
