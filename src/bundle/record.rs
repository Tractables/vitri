//! `preprocess.json`: the lift record itself, its serde spellings and its
//! format tag.

use super::*;

use crate::cnf::DimacsHeader;

/// `#[serde(with = ...)]` for [`PreprocessRecord::mode`]: the mode is written as
/// the token [`Mode::token`] names it by, the same token `reduced.cnf` carries
/// on its `c t` line, so the two files cannot spell one mode two ways.
mod mode_token {
    use crate::cnf::Mode;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    /// Write the mode as its token.
    pub(super) fn serialize<S: Serializer>(mode: &Mode, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(mode.token())
    }

    /// Read it back through [`Mode::parse_mode`].
    ///
    /// # Errors
    ///
    /// The format's own error, naming the token, when it is not one this
    /// version of the crate has a mode for.
    pub(super) fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Mode, D::Error> {
        let token = String::deserialize(de)?;
        Mode::parse_mode(&token).ok_or_else(|| D::Error::custom(format!("unknown mode {token:?}")))
    }
}

/// `#[serde(with = ...)]` for [`PreprocessRecord::format`]: the tag is written
/// as it stands and read back only if it is [`RECORD_FORMAT_TAG`], which is what
/// the field's rustdoc promises a reader does with a tag it does not know.
mod record_format_tag {
    use super::RECORD_FORMAT_TAG;
    pub(super) use crate::bundle::format_tag::serialize;

    /// Read it back, refusing any other tag.
    ///
    /// # Errors
    ///
    /// The format's own error, naming both tags, for a record this version of
    /// the crate does not write.
    pub(super) fn deserialize<'de, D: serde::Deserializer<'de>>(de: D) -> Result<String, D::Error> {
        crate::bundle::format_tag::read(de, RECORD_FORMAT_TAG, "preprocess record")
    }
}

/// The count-lift record: everything needed to turn a count over `reduced.cnf`
/// back into a count over the original CNF, plus the variable correspondence
/// between the two files.
///
/// **All variable ids and literals in this record are 1-based DIMACS**, matching
/// the `.cnf` files it accompanies and the `vtree.vtree` sibling's own
/// numbering. A field with nothing to report is omitted from the file rather
/// than written empty.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreprocessRecord {
    /// Format tag; bump when the on-disk contract changes. Reading refuses any
    /// tag but [`RECORD_FORMAT_TAG`].
    #[serde(with = "record_format_tag")]
    pub format: String,
    /// What preprocessing preserved — the resolved mode ([`RunConfig::mode`],
    /// defaulting to detection from the input's headers). `reduced.cnf` carries
    /// the same token on its own `c t` line, except under `compile`, which no
    /// `c t` line can name.
    #[serde(with = "mode_token")]
    pub mode: Mode,

    /// The cardinality half of the lift:
    /// `count(original) == count(reduced) × 2^count_lift_pow2 × weight_lift`.
    ///
    /// Assembled by the same `SimplifiedFormula::count_lift_pow2` composition a
    /// consumer lifting a count applies, plus Arjun's own exponent. Variables
    /// that are *determined* rather than free (backbone, equivalences,
    /// DVE-defined, Arjun-eliminated-because-defined) contribute a factor of 1
    /// and appear in neither term.
    ///
    /// **Always 0 under a weighted mode**, where a cardinality factor is
    /// meaningless: an eliminated variable's contribution is `w⁻ + w⁺`, not 2, so
    /// the whole lift lives in `weight_lift` instead.
    pub count_lift_pow2: u32,

    /// The weighted half of the lift, exact, as `"numerator/denominator"` in
    /// lowest terms — `"1/1"` under an unweighted mode, where the whole lift is
    /// the power of two above. Apply it in exact rational arithmetic: a float
    /// rounds a `1/3` that no later step recovers.
    ///
    /// A single scalar covering every weighted factor the chain owes: the forced
    /// literals' own weights, `(w⁻ + w⁺)` per free variable, `w⁺` per
    /// equal-weight defined variable, and Arjun's rational multiplier. Factors
    /// that are not scalars — an equivalence-eliminated variable's weights — fold
    /// into the surviving representative and show up in
    /// [`Self::reduced_weights`] instead.
    pub weight_lift: String,

    /// `p cnf` variable count of the ORIGINAL input.
    pub original_num_vars: u32,

    /// The reduced→original variable correspondence, one entry per reduced
    /// variable (`reduced_to_original_dimacs[r - 1]` describes reduced variable
    /// `r`; one entry per variable of `reduced.cnf`).
    ///
    /// Source space is `reduced.cnf`, target space is the original CNF, and
    /// [`VarMap`] states the encoding: a **signed** 1-based DIMACS literal per
    /// reduced variable, `null` where preprocessing introduced one. Serializes as
    /// that bare array.
    ///
    /// Its length is `reduced.cnf`'s own `p cnf` variable count, so the reduced
    /// formula's size is read off the CNF or off this array rather than restated
    /// here.
    pub reduced_to_original_dimacs: VarMap<Reduced, Original>,

    /// The original→reduced correspondence, one entry per ORIGINAL variable
    /// (`original_to_reduced_dimacs[o - 1]` describes original variable `o`);
    /// present under `compile`, absent otherwise.
    ///
    /// The direction that survives elimination. The map above is indexed by the
    /// reduced formula's variables, so it can only name what survived; this one
    /// is indexed by the original's, so it also names each variable the
    /// preprocessing dropped — a backbone literal as the constant it was fixed to, a
    /// free variable as `null`, and an equivalence partner as the signed literal
    /// of the reduced variable its representative became. [`OriginalMap`] states
    /// the per-entry encoding; entries are fully resolved, so nothing here is a
    /// link to follow.
    ///
    /// This is what makes `compile` reconstructible: assign the reduced model,
    /// then read every original variable off it in one lookup. The counting
    /// modes do not write it — each of them eliminates variables whose value a
    /// reduced model does not determine, so no total map over the original
    /// variables exists to write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_to_reduced_dimacs: Option<OriginalMap>,

    /// Forced (backbone) literals as signed 1-based DIMACS literals over the
    /// ORIGINAL variable space: each is fixed to that polarity in every model, so
    /// each contributes its own factor to the lift and none is left for the
    /// consumer to account for. A consumer that wants full models rather than a
    /// count re-attaches these.
    ///
    /// Populated by the count-preserving and function-preserving chains (this
    /// crate's stripping stage plus the backbone Arjun proved). The
    /// projection-preserving chain records none: its stages report eliminations,
    /// not forced polarities.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forced_literals_original_dimacs: Vec<i32>,

    /// Variables this crate's stages removed as free — zero occurrences left
    /// after the simplify chain, or free at DVE — as original 1-based DIMACS
    /// ids. Listed separately from the exponent so a consumer enumerating models
    /// knows which variables are free rather than merely absent.
    ///
    /// Under an unweighted mode each contributes a factor of 2 and is already
    /// summed into `count_lift_pow2`, which also carries Arjun's own aggregate
    /// exponent — Arjun reports a number, not a list of variables, so
    /// `count_lift_pow2` can exceed this list's length. Under a weighted mode
    /// each contributes `(w⁻ + w⁺)` inside `weight_lift` instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub free_vars_original_dimacs: Vec<u32>,

    /// True when preprocessing already proved the instance unsatisfiable. The
    /// count is 0 and no compilation is needed.
    ///
    /// `reduced.cnf` then holds an explicit contradiction (`x` and `¬x` over the
    /// original variable count) with the identity variable map, not the empty
    /// clause: DIMACS has no portable spelling of the empty clause — a lone `0`
    /// line is read back as a stray terminator by most parsers, which would
    /// silently turn UNSAT into a nonzero count. The contradiction is
    /// count-equivalent (0 either way) and unambiguous.
    ///
    /// It is the written form of what a run in process reports as
    /// [`RunVtree::Refuted`]: the same outcome, for a consumer reading
    /// `preprocess.json` back rather than holding the run.
    pub unsat: bool,

    /// The show set, when the mode is projected; absent otherwise. Under
    /// `compile` it is the input's own declared show set, renumbered but not
    /// otherwise touched.
    ///
    /// **This is the set the reduced count must be taken over, and it is not the
    /// input's set**: Arjun's projection-set minimization REWRITES the show set
    /// (dropping variables that are free or determined given the others) and
    /// renumbers it, and the strengthening stage drops any show variable it
    /// proved equivalent to another counted one. Counting `reduced.cnf` over the
    /// input's own show ids would be a silent miscount, which is why the same set
    /// is also written as a `c p show` line INSIDE `reduced.cnf` — read it from
    /// either, never re-derive it from the input.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::cnf::show_set::dimacs"
    )]
    pub show_vars_reduced_dimacs: Option<ShowSet<Reduced>>,

    /// The literal weights the reduced count must be taken under, in REDUCED
    /// numbering, when the mode is weighted; absent otherwise. Also written as
    /// `c p weight` lines inside `reduced.cnf`.
    ///
    /// Not the input's weights: an equivalence-eliminated variable's weights are
    /// folded into its surviving representative (with the polarities swapped when
    /// the two are anti-equivalent), so a survivor's weight here is the product
    /// over its whole class. Every literal of the reduced formula is listed —
    /// including the weight-1 ones — so a consumer never has to know the default.
    /// Under `compile` these are the input's own weights, renumbered but not
    /// folded into the lift.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduced_weights: Option<Vec<LiteralWeight>>,
}

/// Which half of the lift carries it, before the record splits it into
/// [`PreprocessRecord::count_lift_pow2`] and [`PreprocessRecord::weight_lift`].
///
/// The two fields are disjoint by construction, and this is where that is
/// decided: naming one half names the other as neutral, so a chain building a
/// record says which mode it is in once instead of writing the neutral value
/// out by hand.
pub(super) enum RecordLift {
    /// An unweighted mode: the whole lift is `2^k`.
    Pow2(u32),
    /// A weighted mode: the whole lift is this exact rational.
    Weight(BigRational),
}

impl RecordLift {
    /// The lift that changes nothing — `2^0`, which is also the rational `1`,
    /// so it serialises the same under either mode.
    pub(super) fn neutral() -> Self {
        RecordLift::Pow2(0)
    }

    /// `(count_lift_pow2, weight_lift)`, with the half this lift is not
    /// filled in neutral.
    pub(super) fn into_fields(self) -> (u32, String) {
        match self {
            RecordLift::Pow2(k) => (k, rational_string(&BigRational::one())),
            RecordLift::Weight(w) => (0, rational_string(&w)),
        }
    }
}

/// Format tag written into every [`PreprocessRecord`]. Bumped whenever a field is
/// added, removed, or changes meaning; a consumer should refuse a tag it does not
/// know.
pub const RECORD_FORMAT_TAG: &str = "vitri-preprocess-v1";

impl PreprocessRecord {
    /// The record a finished chain starts from: the four things every chain
    /// settles for itself, with every other field at the value that says "this
    /// chain has nothing to report there".
    ///
    /// A chain fills in what it does have with struct-update syntax, so its own
    /// record names exactly what it decided, and the neutral values — no total
    /// map over the original variables, no forced literals, no free variables,
    /// no show set, no weight table — are spelled once here instead of at each
    /// chain's tail. `unsat` is among them: a refuted instance never reaches a
    /// chain's own record, because [`refuted`] answers first.
    pub(super) fn new(
        mode: Mode,
        lift: RecordLift,
        original_num_vars: u32,
        reduced_to_original_dimacs: VarMap<Reduced, Original>,
    ) -> Self {
        let (count_lift_pow2, weight_lift) = lift.into_fields();
        PreprocessRecord {
            format: RECORD_FORMAT_TAG.to_string(),
            mode,
            count_lift_pow2,
            weight_lift,
            original_num_vars,
            reduced_to_original_dimacs,
            original_to_reduced_dimacs: None,
            forced_literals_original_dimacs: Vec::new(),
            free_vars_original_dimacs: Vec::new(),
            unsat: false,
            show_vars_reduced_dimacs: None,
            reduced_weights: None,
        }
    }

    /// The lift as one factor: `count(original) = count(reduced) * lift`.
    ///
    /// Both halves of the split are always present in the record, and exactly
    /// one of them is ever live — a weighted mode records no cardinality lift
    /// at all, since each free variable contributes a rational instead, and
    /// every unweighted mode, `compile` included, records the identity
    /// rational. Which half that is is the mode's, which the record carries.
    pub fn lift(&self) -> String {
        if self.mode.is_weighted() {
            self.weight_lift.clone()
        } else {
            format!("2^{}", self.count_lift_pow2)
        }
    }

    /// Serialize to a pretty JSON string.
    pub fn to_json_string(&self) -> String {
        to_json_pretty(self)
    }

    /// The header lines `reduced.cnf` must carry to describe itself: the track,
    /// the reduced-space show set and the reduced-space weights.
    ///
    /// No `c t` line under `compile`: a `c t` line names a competition track, and
    /// writing `c t compile` would produce a file this crate's own parser rejects.
    /// The mode is in `preprocess.json`'s `mode` either way.
    pub(crate) fn dimacs_header(&self) -> DimacsHeader<'_, Reduced> {
        DimacsHeader {
            track: (self.mode != Mode::Compile).then_some(self.mode.token()),
            show: self.show_vars_reduced_dimacs.as_ref(),
            weights: self.reduced_weights.as_deref(),
        }
    }
}
