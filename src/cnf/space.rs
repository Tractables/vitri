//! Which formula's variables a numbering is expressed over.
//!
//! Preprocessing renumbers, so the same `u32` names a different variable
//! depending on which file it came from. Every representation that crosses one
//! of those boundaries — the projection show set, the literal-weight table — is
//! keyed by one of the markers here, so a value expressed over one formula's
//! variables cannot be handed to a consumer expecting another's.

/// Implementation detail of [`Space`]: its supertrait, which only the three
/// markers below implement, so the set of variable spaces is closed.
pub(crate) mod sealed {
    /// Sealing supertrait — see [`super::Space`].
    pub trait Sealed {}
}

/// A variable space a numbering can be expressed in.
///
/// Sealed: the three markers below are all there are, and a fourth space would
/// be a fourth numbering for this crate to keep straight.
pub trait Space: sealed::Sealed + Copy + 'static {}

/// The input CNF's own variable ids, as the file numbers them.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct Original;

/// `reduced.cnf`'s variable ids — what preprocessing renumbered the survivors
/// into.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct Reduced;

/// One component's dense `0..num_vars` ids, the space `components/compNNN.cnf`
/// and its vtree are written in.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct Local;

impl sealed::Sealed for Original {}
impl Space for Original {}
impl sealed::Sealed for Reduced {}
impl Space for Reduced {}
impl sealed::Sealed for Local {}
impl Space for Local {}
