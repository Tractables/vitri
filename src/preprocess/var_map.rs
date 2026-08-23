//! [`VarMap`]: the variable correspondence preprocessing leaves behind.
//!
//! Every stage that renumbers variables owes its caller a way back, and this is
//! the one shape all of them use — including the one that reaches disk as
//! `preprocess.json`'s `reduced_to_original_dimacs`. The algebra its consumers
//! need (injectivity, inversion, composition with an earlier stage's map) lives
//! on the type, so no caller has to re-derive the sign convention or the
//! 1-basing to use it.
//!
//! [`OriginalMap`] is the same correspondence read the other way, and the only
//! one that can name a variable the reduction DROPPED — a reduced formula has no
//! id for it, so a reduced-indexed map has no slot to put it in.

use super::simplify::OriginalFate;
use crate::cnf::{Original, Reduced, ShowSet, Space, VarId, Weights};
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use std::marker::PhantomData;

/// A correspondence from one CNF's variables to another's — what makes a reduced
/// formula nameable in the space it was reduced FROM.
///
/// # Spaces
///
/// `Src` is the space this map is INDEXED by, `Tgt` the space its entries are
/// written in, so the direction of a map is in its type: a reduced formula's
/// map back to the input file's numbering is a `VarMap<Reduced, Original>`, and
/// [`carry_show`](Self::carry_show) reading a set from `Tgt` back onto `Src`
/// can only be handed a set of the right one. Both markers are compile-time
/// only and appear in no serialized form.
///
/// # Numbering
///
/// Indexed by the **0-based variable id of the SOURCE formula**, one entry per
/// source variable. Each entry is a **signed 1-based DIMACS literal in the
/// TARGET formula's space**:
///
/// - `Some(n)`, `n > 0` — source variable `i` *is* target variable `n`.
/// - `Some(n)`, `n < 0` — source variable `i` is the **negation** of target
///   variable `-n`. Equivalent-literal replacement flips polarity, so dropping
///   the sign lifts models back wrongly while leaving the model *count* identity
///   intact, which is the worst kind of bug.
/// - `None` — source variable `i` has no target counterpart. It was proved
///   constant (backbone), eliminated, or folded onto another source variable.
///
/// # Injective, not surjective
///
/// At most one source variable names any given target variable —
/// [`is_injective`](Self::is_injective) is the check, and the boundary that
/// produces a map runs it before the stage it describes is accepted. The map
/// is NOT onto: a target variable named by no entry was **introduced** by the
/// stage (SBVA adds definitional variables) and has no source counterpart,
/// which is normal rather than a map defect.
///
/// # Serialization
///
/// Serializes as the bare array of entries, `null` for `None` — the on-disk
/// shape of the export record's variable map.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent, bound = "")]
pub struct VarMap<Src: Space, Tgt: Space> {
    entries: Vec<Option<i32>>,
    #[serde(skip)]
    spaces: PhantomData<(Src, Tgt)>,
}

impl<Src: Space, Tgt: Space> VarMap<Src, Tgt> {
    /// The map whose source-indexed entries are `entries`, in the encoding
    /// described on the type.
    pub fn from_entries(entries: Vec<Option<i32>>) -> Self {
        VarMap {
            entries,
            spaces: PhantomData,
        }
    }

    /// The identity over `num_vars` variables: nothing renumbered, nothing
    /// flipped, nothing dropped. What a chain that ran no renumbering stage
    /// reports.
    pub fn identity(num_vars: u32) -> Self {
        VarMap::from_entries((1..=num_vars as i32).map(Some).collect())
    }

    /// One entry per source variable.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the source formula had no variables at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The target literal that source variable `source_var` maps to — `None`
    /// both when the variable has no counterpart and when it is not a variable
    /// of the source formula at all.
    pub fn get(&self, source_var: VarId) -> Option<i32> {
        self.entries.get(source_var.idx()).copied().flatten()
    }

    /// The entries in source-variable order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = Option<i32>> + '_ {
        self.entries.iter().copied()
    }

    /// Whether the map is injective into a target space of `target_num_vars`
    /// variables: every entry names a target variable that exists, and no two
    /// name the same one.
    ///
    /// A producer must verify this rather than assume it. A map that aliased two
    /// source variables onto one target variable would still satisfy the count
    /// identity while making every model lifted back through it wrong — so the
    /// safe response to a false here is to drop the stage's result, not to repair
    /// the map.
    pub fn is_injective(&self, target_num_vars: u32) -> bool {
        let nv = target_num_vars as usize;
        let mut claimed = vec![false; nv];
        for e in self.entries.iter().flatten() {
            let Some(t) = VarId::try_from_dimacs(*e).map(VarId::idx) else {
                return false;
            };
            if t >= nv || claimed[t] {
                return false;
            }
            claimed[t] = true;
        }
        true
    }

    /// The reverse correspondence: target variable → the source variable it
    /// stands for, as a signed 1-based literal, over a target space of
    /// `target_num_vars` variables.
    ///
    /// Target variables no entry names come back as `None` — they are the ones
    /// preprocessing introduced.
    pub fn invert(&self, target_num_vars: u32) -> VarMap<Tgt, Src> {
        self.invert_composed(target_num_vars, |source_var| {
            VarId(source_var as u32).to_dimacs()
        })
    }

    /// Read a show set expressed over this map's TARGET formula back onto its
    /// SOURCE one: source variable `i` is shown exactly when the target variable
    /// it names is shown.
    ///
    /// That is the direction a renumbering is carried in practice — a
    /// reduced→original map is what a chain holds, and the set it has to produce
    /// is the REDUCED one. The result is ascending because the walk is in source
    /// order, and a source variable this map leaves unnamed — one preprocessing
    /// INTRODUCED — is never shown, since it has no target variable to inherit
    /// the declaration from.
    ///
    /// THE conversion for a show set changing space, so no caller reimplements
    /// the renumbering and ends up disagreeing about which variables are shown.
    pub fn carry_show(&self, target_show: &ShowSet<Tgt>) -> ShowSet<Src> {
        ShowSet::from_zero_based(
            self.entries
                .iter()
                .enumerate()
                .filter_map(|(source, entry)| {
                    let target = VarId::try_from_dimacs(*entry.as_ref()?)?;
                    target_show.contains(target).then_some(source as u32)
                }),
        )
    }

    /// Read a weight table expressed over this map's TARGET formula back onto
    /// its SOURCE one: source variable `i` carries the weights of the target
    /// literal its entry names, `(w⁻, w⁺)` **SWAPPED** when the entry names the
    /// negation — a reduced variable standing for `¬o` has the original's
    /// negative weight on its positive literal. A source variable this map
    /// leaves unnamed — one preprocessing INTRODUCED — weighs 1 either way.
    ///
    /// THE conversion for a weight table changing space, so no caller
    /// reimplements the renumbering, and above all so no caller reimplements
    /// the swap.
    pub fn carry_weights(&self, target: &Weights<Tgt>) -> Weights<Src> {
        Weights::from_carried(self.entries.iter().map(|entry| {
            let lit = (*entry)?;
            let (wn, wp) = target.get(VarId::try_from_dimacs(lit)?)?;
            Some(if lit > 0 {
                (wn.clone(), wp.clone())
            } else {
                (wp.clone(), wn.clone())
            })
        }))
    }

    /// [`invert`](Self::invert), composing an earlier stage's map on the source
    /// side: `source_dimacs` names each 0-based source variable in whatever
    /// space that earlier stage came from, so a two-stage chain lands in the
    /// original space in one pass instead of composing two inverted maps.
    ///
    /// [`invert`](Self::invert) is this with the identity naming.
    pub(crate) fn invert_composed<Named: Space>(
        &self,
        target_num_vars: u32,
        source_dimacs: impl Fn(usize) -> i32,
    ) -> VarMap<Tgt, Named> {
        let nv = target_num_vars as usize;
        let mut out: Vec<Option<i32>> = vec![None; nv];
        for (source_var, entry) in self.entries.iter().enumerate() {
            let Some(lit) = entry else { continue };
            // Naming a variable, naming one the target formula has, and naming
            // it alone are all established by `is_injective` before a stage is
            // accepted, so a violation here is a broken producer rather than a
            // case to handle — but drop the entry rather than index out of
            // bounds.
            let Some(target) = VarId::try_from_dimacs(*lit).map(VarId::idx) else {
                continue;
            };
            debug_assert!(
                target < nv && out[target].is_none(),
                "the map must be injective into the target space before it is inverted",
            );
            if target >= nv {
                continue;
            }
            let named = source_dimacs(source_var);
            out[target] = Some(if *lit > 0 { named } else { -named });
        }
        VarMap::from_entries(out)
    }
}

impl<Src: Space> VarMap<Src, Reduced> {
    /// Read this map's TARGET space as the ORIGINAL one, for a chain whose
    /// reduction renumbered nothing before this map was produced — so what the
    /// map calls a reduced variable is an original variable under a different
    /// name.
    ///
    /// The named alternative to a silent re-typing, as on
    /// [`Weights::assume_reduced_identity`]: every call site owes a one-line
    /// reason why the two numberings coincide.
    pub(crate) fn assume_original_target(self) -> VarMap<Src, Original> {
        VarMap::from_entries(self.entries)
    }
}

impl<Src: Space, Tgt: Space> FromIterator<Option<i32>> for VarMap<Src, Tgt> {
    fn from_iter<T: IntoIterator<Item = Option<i32>>>(iter: T) -> Self {
        VarMap::from_entries(iter.into_iter().collect())
    }
}

/// What one ORIGINAL variable became, as an [`OriginalMap`] entry.
///
/// # Serialization
///
/// One JSON value per variant, so an entry's kind is its type:
///
/// - [`Literal`](Self::Literal) — a **nonzero signed 1-based DIMACS literal** of
///   the reduced formula. `3` means the original variable equals reduced
///   variable 3, `-3` means it equals its negation.
/// - [`Constant`](Self::Constant) — `true` or `false`: the value the original
///   variable takes in every model.
/// - [`Free`](Self::Free) — `null`: nothing constrains it, so both values extend
///   every reduced model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OriginalTarget {
    /// Equal to this signed 1-based DIMACS literal of the reduced formula.
    Literal(i32),
    /// Fixed to this constant by preprocessing.
    Constant(bool),
    /// Unconstrained: absent from the reduced formula and fixed by nothing.
    Free,
}

impl Serialize for OriginalTarget {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match *self {
            OriginalTarget::Literal(lit) => s.serialize_i32(lit),
            OriginalTarget::Constant(value) => s.serialize_bool(value),
            OriginalTarget::Free => s.serialize_none(),
        }
    }
}

/// The inverse of the untagged encoding the impl above writes: the value's own
/// type selects the variant, so there is no tag to read and a derived impl
/// (which would demand one) would not read what this crate writes.
///
/// Dispatches on the data model rather than on `serde_json::Value`, so a record
/// can be read from any self-describing format, not only from JSON.
impl<'de> Deserialize<'de> for OriginalTarget {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct TargetVisitor;

        impl<'de> Visitor<'de> for TargetVisitor {
            type Value = OriginalTarget;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a signed reduced literal, a boolean, or null")
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                match i32::try_from(v) {
                    Ok(lit) => Ok(OriginalTarget::Literal(lit)),
                    Err(_) => Err(E::invalid_value(de::Unexpected::Signed(v), &self)),
                }
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                match i32::try_from(v) {
                    Ok(lit) => Ok(OriginalTarget::Literal(lit)),
                    Err(_) => Err(E::invalid_value(de::Unexpected::Unsigned(v), &self)),
                }
            }

            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(OriginalTarget::Constant(v))
            }

            // `null` reaches a visitor as either, depending on whether the
            // format's own decoder took the option path.
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(OriginalTarget::Free)
            }

            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(OriginalTarget::Free)
            }
        }

        d.deserialize_any(TargetVisitor)
    }
}

/// The TOTAL original→reduced correspondence: one entry per variable of the
/// ORIGINAL formula, in original order, saying what became of it.
///
/// # Total, and the direction that survives elimination
///
/// [`VarMap`] is indexed by the REDUCED formula's variables, so it can only name
/// what survived; a variable preprocessing dropped — a backbone literal, a dead
/// variable, an equivalence partner folded onto its representative — has no
/// entry there to be named in. This map is indexed by the original variables
/// instead, so every one of them is named, and reconstructing an original
/// assignment from a reduced model is one lookup per original variable.
///
/// # No chains
///
/// Each entry is already resolved: an equivalence partner names the REDUCED
/// variable its representative became, not the partner it was found equivalent
/// to. A consumer never follows a link and never has to detect a cycle.
///
/// # Serialization
///
/// The bare array of [`OriginalTarget`] entries. Its length is
/// `original_num_vars`, so the record does not restate it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginalMap(Vec<OriginalTarget>);

impl OriginalMap {
    /// The map whose original-indexed entries are `entries`.
    pub fn from_entries(entries: Vec<OriginalTarget>) -> Self {
        OriginalMap(entries)
    }

    /// The map for a run that removed nothing: original variable `i` is
    /// reduced variable `i`, over `num_vars` variables.
    pub fn identity(num_vars: u32) -> Self {
        OriginalMap((1..=num_vars as i32).map(OriginalTarget::Literal).collect())
    }

    /// The DIMACS form of what the simplification reported, one entry per
    /// original variable: the 0-based reduced index becomes a signed 1-based
    /// literal, negated when the original is the negation of that variable.
    ///
    /// The single place the internal statement crosses into the on-disk
    /// numbering, so no producer re-derives the sign convention or the 1-basing.
    pub(crate) fn from_fates(fates: &[OriginalFate]) -> Self {
        OriginalMap(
            fates
                .iter()
                .map(|fate| match *fate {
                    OriginalFate::Variable {
                        index,
                        same_polarity,
                    } => {
                        let dimacs = VarId(index as u32).to_dimacs();
                        OriginalTarget::Literal(if same_polarity { dimacs } else { -dimacs })
                    }
                    OriginalFate::Forced(value) => OriginalTarget::Constant(value),
                    OriginalFate::Unconstrained => OriginalTarget::Free,
                })
                .collect(),
        )
    }

    /// One entry per original variable.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the original formula had no variables at all.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// What became of original variable `original_var` — `None` for an id the
    /// original formula did not have.
    pub fn get(&self, original_var: VarId) -> Option<OriginalTarget> {
        self.0.get(original_var.idx()).copied()
    }

    /// The entries in original-variable order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = OriginalTarget> + '_ {
        self.0.iter().copied()
    }
}
