//! The projection show set — which variables a count is taken over — and the
//! per-formula mask derived from it.
//!
//! # Two types, and why the split
//!
//! [`ShowSet`] is the durable thing: it is parsed from `c p show`, written back
//! to `c p show`, serialized into `preprocess.json` and `components.json`, and
//! carried across every variable renumbering preprocessing performs. It is
//! keyed by a [`Space`] marker so that a set expressed over one formula's
//! variables cannot be handed to a consumer expecting another's.
//!
//! [`ShowMask`] is the derived view: `mask[variable]`, over a formula named at
//! the point of derivation. It is not space-keyed, because the mistake a mask
//! can make is being indexed by a different formula's ids, and what prevents
//! that is deriving it from a set plus that formula's variable count
//! ([`ShowSet::mask`]) or restricting an existing one through a local→global
//! correspondence ([`ShowMask::restrict`]) — never a marker.
//!
//! # Numbering
//!
//! A [`ShowSet`] holds variable numbers as DIMACS writes them, ascending and
//! deduplicated; the constructors are what establish that, and every reader
//! may rely on it. [`ShowSet::from_dimacs_ids`] is where a written set is
//! checked (a `0` names no variable) and [`ShowSet::as_dimacs`] is the array
//! every artifact writes.

use std::marker::PhantomData;

use super::VarId;
use super::space::{Local, Original, Reduced, Space};
use crate::error::VitriError;

/// The variables a count is projected onto, in the space `S` names.
///
/// Ascending, deduplicated variable numbers. `S` is a compile-time marker only
/// — it costs nothing at runtime and appears in no serialized form.
pub struct ShowSet<S: Space>(Vec<u32>, PhantomData<S>);

impl<S: Space> ShowSet<S> {
    /// The set that shows nothing. Distinct from "no show set at all", which
    /// callers spell `None`: `c p show 0` projects onto nothing, and a count
    /// over it is 1 or 0.
    pub fn empty() -> Self {
        ShowSet(Vec::new(), PhantomData)
    }

    /// The set as a written artifact carries it — a `c p show` line, a record
    /// field, a manifest entry. THE place a written show set is checked.
    ///
    /// # Errors
    ///
    /// [`VitriError::Input`] naming the offending id when one is `0`, which
    /// terminates a `c p show` line rather than naming a variable.
    pub fn from_dimacs_ids(ids: &[u32]) -> Result<Self, VitriError> {
        if ids.contains(&0) {
            return Err(VitriError::input(
                "0 is not a show variable (it terminates a `c p show` line)",
            ));
        }
        Ok(Self::from_vars(ids.iter().map(|&id| VarId(id))))
    }

    /// The set over `vars`, in any order and with any repeats — canonicalized
    /// here.
    pub fn from_vars(vars: impl IntoIterator<Item = VarId>) -> Self {
        let mut vars: Vec<u32> = vars.into_iter().map(|v| v.0).collect();
        vars.sort_unstable();
        vars.dedup();
        ShowSet(vars, PhantomData)
    }

    /// Whether `var` is shown.
    pub fn contains(&self, var: VarId) -> bool {
        self.0.binary_search(&var.0).is_ok()
    }

    /// How many variables are shown.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether nothing is shown.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The shown variables, ascending.
    pub fn iter_vars(&self) -> impl ExactSizeIterator<Item = VarId> + '_ {
        self.0.iter().map(|&v| VarId(v))
    }

    /// The variable numbers, ascending — the array every emitted artifact
    /// carries, and what [`Self::from_dimacs_ids`] reads back.
    pub fn as_dimacs(&self) -> &[u32] {
        &self.0
    }

    /// The mask over a formula of `num_vars` variables: `mask[variable]`.
    ///
    /// Ids the formula does not have are dropped rather than rejected, so
    /// masking a set against a narrower space yields the intersection.
    pub fn mask(&self, num_vars: u32) -> ShowMask {
        let mut bits = vec![false; num_vars as usize];
        for &v in &self.0 {
            if let Some(slot) = bits.get_mut(VarId(v).idx()) {
                *slot = true;
            }
        }
        ShowMask(bits)
    }

    /// Add `var`, keeping the set canonical. Idempotent.
    pub fn insert(&mut self, var: VarId) {
        if let Err(at) = self.0.binary_search(&var.0) {
            self.0.insert(at, var.0);
        }
    }

    /// Drop `var` from the set. Idempotent, and the set stays canonical.
    pub fn remove(&mut self, var: VarId) {
        if let Ok(at) = self.0.binary_search(&var.0) {
            self.0.remove(at);
        }
    }
}

impl ShowSet<Original> {
    /// Read this set as one over the REDUCED space, for the paths where the two
    /// spaces coincide because nothing was renumbered.
    ///
    /// The named alternative to a silent re-typing: every call site owes a
    /// one-line reason why the reduced formula carries the input's own variable
    /// ids. A site that cannot give one has a [`VarMap`](crate::preprocess::VarMap)
    /// to carry the set through instead.
    pub(crate) fn assume_reduced_identity(self) -> ShowSet<Reduced> {
        ShowSet(self.0, PhantomData)
    }
}

impl<S: Space> Clone for ShowSet<S> {
    fn clone(&self) -> Self {
        ShowSet(self.0.clone(), PhantomData)
    }
}

impl<S: Space> std::fmt::Debug for ShowSet<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ShowSet").field(&self.0).finish()
    }
}

impl<S: Space> PartialEq for ShowSet<S> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<S: Space> Eq for ShowSet<S> {}

/// A [`ShowSet`] resolved against one formula: `is_show(v)` for every variable
/// of it.
///
/// Only derivable from a set plus that formula's variable count
/// ([`ShowSet::mask`]) or by restricting one to a component
/// ([`Self::restrict`]), which is what ties a mask to the formula it indexes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ShowMask(Vec<bool>);

impl ShowMask {
    /// Whether `var` is shown. A variable outside the masked formula is not.
    pub fn is_show(&self, var: VarId) -> bool {
        self.0.get(var.idx()).copied().unwrap_or(false)
    }

    /// The mask as a `mask[variable]` slice, for the scoring walks that index
    /// it once per variable.
    pub fn as_slice(&self) -> &[bool] {
        &self.0
    }

    /// How many variables are shown.
    pub fn count(&self) -> usize {
        self.0.iter().filter(|&&b| b).count()
    }

    /// This mask read over a sub-formula: `local_to_global[i]` is the variable
    /// of the masked formula that the sub-formula's variable `VarId::from_idx(i)`
    /// stands for.
    ///
    /// THE way a show set descends into a component, so the per-component CNF's
    /// `c p show` line and the mask its vtree selection scores cannot disagree
    /// about which local variables are shown.
    pub fn restrict(&self, local_to_global: &[VarId]) -> ShowSet<Local> {
        ShowSet(
            local_to_global
                .iter()
                .enumerate()
                .filter(|&(_, &global)| self.is_show(global))
                .map(|(local, _)| VarId::from_idx(local).0)
                .collect(),
            PhantomData,
        )
    }
}

/// `#[serde(with = ...)]` for an `Option<ShowSet<_>>` record field: the
/// ascending array every consumer of these files already parses, the same
/// [`ShowSet::as_dimacs`] the `c p show` line beside it is written from.
pub(crate) mod dimacs {
    use super::{ShowSet, Space};
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// Write the set as its ascending array, `null` for absent.
    pub(crate) fn serialize<S: Space, Ser: Serializer>(
        set: &Option<ShowSet<S>>,
        ser: Ser,
    ) -> Result<Ser::Ok, Ser::Error> {
        set.as_ref().map(ShowSet::as_dimacs).serialize(ser)
    }

    /// Read it back.
    ///
    /// # Errors
    ///
    /// The format's own error, carrying [`ShowSet::from_dimacs_ids`]'s sentence
    /// when the array names variable `0`.
    pub(crate) fn deserialize<'de, S: Space, D: Deserializer<'de>>(
        de: D,
    ) -> Result<Option<ShowSet<S>>, D::Error> {
        match Option::<Vec<u32>>::deserialize(de)? {
            Some(ids) => ShowSet::from_dimacs_ids(&ids)
                .map(Some)
                .map_err(D::Error::custom),
            None => Ok(None),
        }
    }
}
