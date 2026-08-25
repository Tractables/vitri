//! Arjun (Ganak's preprocessor) front end.
//!
//! This module is the POLICY site: it owns the `ArjunResult` /
//! `ArjunProjResult` / `ArjunWeightedProjResult` payload shapes and the three
//! `run_arjun_*_anytime` entry points, dispatching to the in-process shim in
//! [`super::arjun_lib`].
//!
//! Arjun is MIT-licensed — the header of the pinned v2.7.2 release carries the
//! MIT notice. It is linked in-process through a narrow C++ shim that every
//! build compiles; there is no subprocess and no `arjun` binary to find on
//! `PATH`.

use crate::cnf::{CnfFormula, Literal, Reduced, ShowSet, Space, Weights};
use crate::diagnostics::diag;
use crate::error::VitriError;
use crate::preprocess::VarMap;
use crate::score::StructureProfile;
use std::time::Duration;
use std::time::Instant;

/// Whether Arjun's bounded variable addition runs during preprocessing.
///
/// Bounded variable addition rewrites the clause set into a smaller one with new
/// variables. It is count-preserving, but on some inputs the rewritten formula
/// compiles far worse than the un-rewritten formula, so it is worth being able
/// to turn off. Selected by
/// [`ArjunOptions::sbva`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArjunSbva {
    /// Always run it. The default.
    On,
    /// Never run it.
    Off,
    /// Run it unless the input's structure looks like a graph-colouring
    /// encoding: preprocessing measures how evenly variable occurrences and
    /// clause widths are spread and turns the addition off when both are
    /// near-uniform. On those inputs it rewrites the clause set into something
    /// whose good decomposition the vtree portfolio can no longer find. The
    /// predicate and its thresholds are the ones the portfolio's own
    /// colouring-like candidate gate uses, so the two agree by construction.
    Auto,
}

/// What `VITRI_ARJUN_SBVA` accepts, quoted in both of its error messages.
const SBVA_FORMS: &str = "on (always run bounded variable addition), off (never), or \
     auto (skip it when the input is coloring-like)";

/// Reads `VITRI_ARJUN_SBVA` — THE one place that variable is read, beside the
/// parser that owns its spellings.
///
/// # Errors
///
/// [`VitriError::Env`] naming the variable and the accepted forms.
pub(crate) fn resolve_arjun_sbva() -> Result<ArjunSbva, VitriError> {
    arjun_sbva_policy(crate::env::env_raw("VITRI_ARJUN_SBVA", SBVA_FORMS)?.as_deref())
}

/// Parses the `VITRI_ARJUN_SBVA` value — the one place the knob's spellings
/// live. Absent ⇒ [`ArjunSbva::On`]. `on`/`off`/`auto` map to the three
/// states; anything else ⇒ [`VitriError::Env`] naming the variable and the
/// accepted forms.
pub(crate) fn arjun_sbva_policy(v: Option<&str>) -> Result<ArjunSbva, VitriError> {
    crate::env::from_forms(
        "VITRI_ARJUN_SBVA",
        v,
        ArjunSbva::On,
        &[
            ("on", ArjunSbva::On),
            ("off", ArjunSbva::Off),
            ("auto", ArjunSbva::Auto),
        ],
        SBVA_FORMS,
    )
}

/// The bounded-variable-addition decision, as the `no_sbva` flag:
/// [`ArjunSbva::On`] never skips, [`ArjunSbva::Off`] always does,
/// [`ArjunSbva::Auto`] skips exactly when `formula` looks coloring-like.
pub(crate) fn arjun_sbva_skip(formula: &CnfFormula, policy: ArjunSbva) -> bool {
    match policy {
        ArjunSbva::On => false,
        ArjunSbva::Off => true,
        ArjunSbva::Auto => {
            let profile = StructureProfile::measure(formula);
            if profile.coloring_like {
                diag!(
                    "[arjun] VITRI_ARJUN_SBVA=auto — input is coloring-like, skipping SBVA \
                     (occ_cv={:.4} width_cv={:.4})",
                    profile.var_occurrence_cv,
                    profile.clause_width_cv,
                );
            }
            profile.coloring_like
        }
    }
}

/// Result of an Arjun preprocessing pass.
///
/// This is the payload that crosses the process boundary when a reduction runs
/// under the hard-deadline fork harness — its codec is
/// `preprocess::fork_payload`'s `impl ForkPayload for ArjunResult`, which
/// destructures this struct exhaustively, so adding a field here is a compile
/// error there until the field is carried across too.
#[derive(Debug, PartialEq)]
pub(crate) struct ArjunResult {
    /// Preprocessed formula (typically smaller; var IDs may differ from input).
    pub formula: CnfFormula,
    /// Exponent N: original count = preprocessed_count * 2^N.
    pub multiplier_exp: u32,
    /// Backbone literals (forced in every model) discovered at Arjun's minimize
    /// stage, in the INPUT formula's var space (the formula passed to
    /// `run_arjun_anytime`, NOT the reduced output).
    pub backbone: Vec<Literal>,
    /// Equivalence pairs `(a, b)` meaning `a ≡ b`, in the INPUT var space (same
    /// caveats as `backbone`).
    pub equiv: Vec<(Literal, Literal)>,
    /// Redundant/learnt clauses harvested from Arjun's internal solver during
    /// simplify, as per-clause DIMACS `i32` lists in the **REDUCED output**
    /// (`formula`) var space — NOT the input space (unlike `backbone`/`equiv`).
    /// Every var appears in `formula`; clauses over eliminated vars are dropped
    /// at harvest. Populated only when the caller asked
    /// ([`run_arjun_anytime`]'s `export_learned_clauses`); empty otherwise.
    pub learnt_clauses: Vec<Vec<i32>>,
    /// The INPUT→REDUCED variable correspondence for this pass — the piece that
    /// makes `formula` nameable in the caller's own variable space.
    ///
    /// Source space is the INPUT formula (the one passed to
    /// `run_arjun_anytime`), target space is `formula`; [`VarMap`] states the
    /// encoding and owns its algebra. A variable mapped to `None` contributed
    /// its factor already: it is inside `multiplier_exp` if it was free, and a
    /// factor of 1 if the reduction determined it.
    ///
    /// Read straight off Arjun's own `SimplifiedCNF::get_orig_to_new_var()` (see
    /// `vendor/arjun/arjun_shim.h`), not reconstructed here by matching clauses.
    pub input_to_reduced_lit: VarMap<Reduced, Reduced>,
}

/// The "keep this Arjun reduction, or fall back to the raw formula?" decision.
/// The tracks do NOT share criteria: MC/WMC gate on clause blowup, PMC/PWMC on
/// whether the PROJECTION actually shrank. The differing criteria are passed in
/// as the variant, so this only centralizes the branch.
pub(crate) enum ArjunKeep {
    /// MC / WMC clause-blowup gate. Arjun eliminates variables by resolution,
    /// which can MULTIPLY clause count; a reduced formula with MORE clauses than
    /// the raw one is structurally harder to compile despite fewer variables.
    /// Compares clause counts only, so it is variable-space-agnostic: the two
    /// counts need not share a numbering.
    ClauseCount {
        /// Clause count of the raw (unreduced) formula.
        raw_clauses: usize,
        /// Clause count of the candidate Arjun-reduced formula.
        reduced_clauses: usize,
    },
    /// PMC (integer) projection gate: keep iff Arjun minimized the PROJECTION —
    /// the show set shrank, or it removed free/defined show vars
    /// (`multiplier_exp > 0`). A pure variable elimination (show unchanged AND
    /// ×2^0) has zero counting benefit, so it is discarded.
    Projection {
        /// Whether Arjun's minimized show set is smaller than the show set it was handed.
        show_shrank: bool,
        /// Whether `multiplier_exp > 0`.
        multiplier_nontrivial: bool,
    },
    /// PWMC (weighted) projection gate: like [`ArjunKeep::Projection`], but ALSO
    /// keep a large defined/free non-show var block elimination (K==1, show
    /// preserved) when the variable count dropped enough (reduced < 90% of the
    /// original) — for compile-bound PWMC that var reduction is the whole point.
    /// Only a true no-op (show unchanged AND K==1 AND vars barely moved) is
    /// discarded.
    WeightedProjection {
        /// Whether Arjun's minimized show set is smaller than the show set it was handed.
        show_shrank: bool,
        /// Whether the rational multiplier `K` is not 1.
        multiplier_nontrivial: bool,
        /// Whether the reduced variable count is under 90% of the original.
        vars_shrank_10pct: bool,
    },
    /// WMC (weighted, unprojected) usability gate — checked FIRST, before the
    /// clause-blowup gate, because it is about SOUNDNESS rather than compiling
    /// faster.
    ///
    /// The weighted reduction's rational multiplier `K` lifts a weighted count
    /// over the reduced formula. When Arjun resolves the instance outright it
    /// hands back an empty reduction whose `K` does NOT carry the weighted
    /// backbone mass it dropped, so `K` alone is not the answer — keeping that
    /// reduction would silently lose the mass. `inert` is the separate, harmless
    /// case: nothing shrank and `K == 1`.
    Weighted {
        /// Arjun resolved the instance outright (no variables or no clauses left).
        solved_outright: bool,
        /// Nothing shrank and `K == 1`.
        inert: bool,
    },
}

impl ArjunKeep {
    /// The PMC criteria, read off a completed reduction. `orig_show_len` is the
    /// show-set size Arjun was handed.
    pub(crate) fn projection_for(orig_show_len: usize, r: &ArjunProjResult) -> Self {
        ArjunKeep::Projection {
            show_shrank: r.show.len() != orig_show_len,
            multiplier_nontrivial: r.multiplier_exp != 0,
        }
    }

    /// The PWMC criteria, read off a completed weighted reduction. `orig_num_vars`
    /// is the variable count of the formula Arjun was handed — the `< 90%` var
    /// drop is measured against it.
    pub(crate) fn weighted_projection_for(
        orig_show_len: usize,
        orig_num_vars: u32,
        r: &ArjunWeightedProjResult,
    ) -> Self {
        use num_traits::One;
        ArjunKeep::WeightedProjection {
            show_shrank: r.show.len() != orig_show_len,
            multiplier_nontrivial: !r.multiplier.is_one(),
            vars_shrank_10pct: (r.formula.num_vars as u64) * 10 < (orig_num_vars as u64) * 9,
        }
    }

    /// The WMC criteria, read off a completed weighted (unprojected) reduction.
    /// `input_num_vars` is the variable count of the formula Arjun was handed.
    pub(crate) fn weighted_for(input_num_vars: u32, r: &ArjunWeightedResult) -> Self {
        use num_traits::One;
        ArjunKeep::Weighted {
            solved_outright: r.formula.num_vars == 0 || r.formula.clauses.is_empty(),
            inert: r.formula.num_vars >= input_num_vars && r.multiplier.is_one(),
        }
    }
}

/// Decide whether to keep an Arjun reduction — see [`ArjunKeep`] for the
/// per-track criteria.
pub(crate) fn arjun_keep_reduction(criteria: ArjunKeep) -> bool {
    match criteria {
        ArjunKeep::ClauseCount {
            raw_clauses,
            reduced_clauses,
        } => reduced_clauses <= raw_clauses,
        ArjunKeep::Projection {
            show_shrank,
            multiplier_nontrivial,
        } => show_shrank || multiplier_nontrivial,
        ArjunKeep::WeightedProjection {
            show_shrank,
            multiplier_nontrivial,
            vars_shrank_10pct,
        } => show_shrank || multiplier_nontrivial || vars_shrank_10pct,
        ArjunKeep::Weighted {
            solved_outright,
            inert,
        } => !solved_outright && !inert,
    }
}

/// What the Arjun stage is configured with, on
/// [`RunConfig::arjun`](crate::config::RunConfig::arjun).
///
/// Every field is an axis with a measured record rather than a tuning constant,
/// and the defaults are the production settings, so a caller with no opinion
/// passes [`Default`]. Each field also has a `VITRI_*` variable that
/// [`RunConfig::from_env_defaults`](crate::config::RunConfig::from_env_defaults)
/// reads into it, listed in `docs/env.md`; a caller running two reductions in
/// one process sets the fields, since a variable is process-global and would
/// reach both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ArjunOptions {
    /// How hard the reduction works.
    pub effort: ArjunEffort,

    /// Whether Arjun's bounded variable addition runs — see [`ArjunSbva`].
    pub sbva: ArjunSbva,

    /// Variable counts above which a reduction skips Arjun's definability
    /// oracle.
    pub oracle_max_vars: OracleCaps,

    /// Keep a reduction that came back after its deadline, or discard it and
    /// reduce nothing.
    ///
    /// `false` — discard — by default, and a discarding run also runs the
    /// reduction in a forked child it kills once the deadline passes, since a
    /// result it would throw away is not worth the wall. A reduction that
    /// overran spent budget the stages after it now do not have, and discarding
    /// measured better than keeping; but it is the caller's wall.
    pub keep_overrun: bool,

    /// Seed for the reduction's internal randomization.
    ///
    /// Every seed gives a sound reduction and a different one, which re-rolls
    /// everything downstream. Fixed by default at Arjun's own seed, so two runs
    /// of one configuration over one formula reduce identically.
    pub seed: u32,

    /// Whether the reduction harvests the redundant clauses its internal solver
    /// derived, onto
    /// [`PreprocessBundle::learnt_clauses_reduced_dimacs`](crate::bundle::PreprocessBundle::learnt_clauses_reduced_dimacs).
    ///
    /// Off by default: the harvest buys Arjun's oracle passes extra work, and
    /// nothing in this crate consumes what they produce — it is there for a
    /// consumer that wants to seed its own solver with them.
    pub export_learned_clauses: bool,
}

impl Default for ArjunOptions {
    fn default() -> Self {
        ArjunOptions {
            effort: ArjunEffort::Full,
            sbva: ArjunSbva::On,
            oracle_max_vars: OracleCaps::default(),
            keep_overrun: false,
            // Arjun's own default seed. Passing it explicitly and passing
            // nothing are the same reduction, byte for byte.
            seed: 42,
            export_learned_clauses: false,
        }
    }
}

/// How hard an Arjun reduction works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ArjunEffort {
    /// The whole pipeline: extend-indep, autarky, bounded variable addition,
    /// renumbering, and a budget-gated variable elimination and oracle pass.
    #[default]
    Full,
    /// Propagation, backbone and probing, and equivalent-literal substitution,
    /// and nothing heavier. Same contract as [`Self::Full`] — a reduced formula
    /// and a power-of-two multiplier — reached faster and reducing less, for a
    /// caller whose budget the full pipeline does not repay.
    Lite,
}

/// Variable counts above which a reduction skips Arjun's definability oracle.
///
/// The oracle proves clauses redundant, and its cost grows with the formula: on
/// a large one it can spend the whole budget without reducing anything, and
/// skipping it lets the rest of the reduction run. Skipping is
/// count-preserving — the reduction comes back larger, never wrong.
///
/// `None` is no ceiling. The tracks are separate fields because they have been
/// measured separately: a projected instance's oracle works on the show set, an
/// unprojected one's on the whole formula.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct OracleCaps {
    /// Ceiling for the unprojected tracks, [`Mc`](crate::cnf::Mode::Mc) and
    /// [`Wmc`](crate::cnf::Mode::Wmc). `None` by default: a finite ceiling here
    /// loses more mid-size instances whose oracle repays itself than it gains
    /// on the class that overruns, and the two are not told apart by any count
    /// available before the oracle runs.
    pub plain: Option<u32>,
    /// Ceiling for [`Pmc`](crate::cnf::Mode::Pmc).
    pub projected: Option<u32>,
    /// Ceiling for [`Pwmc`](crate::cnf::Mode::Pwmc).
    pub weighted_projected: Option<u32>,
}

impl Default for OracleCaps {
    fn default() -> Self {
        OracleCaps {
            plain: None,
            projected: Some(super::arjun_lib::PROJECTED_ORACLE_MAX_VARS_DEFAULT),
            weighted_projected: Some(super::arjun_lib::PROJECTED_ORACLE_MAX_VARS_DEFAULT),
        }
    }
}

/// Track-1 (unweighted, full-count) Arjun reduction with the given budget.
///
/// Drives Arjun **in-process** and reads a sound checkpoint at the budget
/// boundary, so a budget-blow yields a usable partial reduction rather than
/// nothing. `None` means the caller compiles the raw formula.
///
/// # Errors
///
/// [`VitriError::Env`] for a `VITRI_*` variable this path reads. A reduction
/// that does not fit the budget is not an error: it comes back as `Ok(None)`.
pub(crate) fn run_arjun_anytime(
    formula: &CnfFormula,
    budget: Duration,
    arjun: ArjunOptions,
    force_no_sbva: bool,
) -> Result<Option<ArjunResult>, VitriError> {
    super::arjun_lib::reduce_anytime(formula, Instant::now() + budget, arjun, force_no_sbva)
}

/// Projected (Track-3 PMC) reduction: drive Arjun's projection-set minimization
/// through the in-process shim and keep the best partial reduction even when it
/// does not converge within `budget`, so an Arjun overrun costs a weaker
/// reduction rather than the whole window. Returns `None` only when not even the
/// cheap minimize stage fits the budget or the shim is unavailable — in which
/// case the caller takes the raw projected path. See
/// [`super::arjun_lib::reduce_anytime_projected`] for the soundness argument.
///
/// # Errors
///
/// [`VitriError::Env`] for a `VITRI_*` variable this path reads.
pub(crate) fn run_arjun_projected_anytime<S: Space>(
    formula: &CnfFormula,
    show: &ShowSet<S>,
    budget: Duration,
    arjun: ArjunOptions,
    force_no_sbva: bool,
) -> Result<Option<ArjunProjResult>, VitriError> {
    super::arjun_lib::reduce_anytime_projected(
        formula,
        show,
        Instant::now() + budget,
        arjun,
        force_no_sbva,
    )
}

/// Weighted projected (Track-4 PWMC) reduction: drive Arjun's weighted
/// projection-set minimization through the in-process shim and keep the best
/// partial reduction even when it does not converge within `budget`. Returns
/// `None` only when the cheap minimize stage does not fit the budget or the
/// shim is unavailable — in which case the caller takes the raw
/// weighted-projected path. See
/// [`super::arjun_lib::reduce_anytime_weighted_projected`] for the soundness
/// argument.
///
/// # Errors
///
/// [`VitriError::Env`] for a `VITRI_*` variable this path reads.
pub(crate) fn run_arjun_weighted_projected_anytime<S: Space>(
    formula: &CnfFormula,
    show: &ShowSet<S>,
    weights: &[(i32, num_rational::BigRational)],
    budget: Duration,
    arjun: ArjunOptions,
    force_no_sbva: bool,
) -> Result<Option<ArjunWeightedProjResult>, VitriError> {
    super::arjun_lib::reduce_anytime_weighted_projected(
        formula,
        show,
        weights,
        Instant::now() + budget,
        arjun,
        force_no_sbva,
    )
}

/// Weighted, UNPROJECTED (Track-2 WMC) reduction: Arjun's weighted mode over the
/// whole variable set, which folds the mass of every variable it eliminates into
/// the rational multiplier `K` rather than projecting it away. Returns the reduced
/// formula, the reduced per-literal weights, `K`, and the input→reduced variable
/// map; `wmc(input, weights) == K × wmc(reduced, reduced weights)`.
///
/// Not every result is usable: see [`ArjunKeep::weighted_for`].
///
/// `None` when the shim is unavailable or the cheap stage does not fit `budget`;
/// the caller then keeps its own formula.
///
/// `force_no_sbva` disables SBVA in the heavy simplify stage for this call, on
/// the same terms as [`run_arjun_anytime`].
///
/// # Errors
///
/// [`VitriError::Env`] for a `VITRI_*` variable this path reads.
pub(crate) fn run_arjun_weighted_anytime(
    formula: &CnfFormula,
    weights: &[(i32, num_rational::BigRational)],
    budget: Duration,
    arjun: ArjunOptions,
    force_no_sbva: bool,
) -> Result<Option<ArjunWeightedResult>, VitriError> {
    super::arjun_lib::reduce_anytime_weighted(
        formula,
        weights,
        Instant::now() + budget,
        arjun,
        force_no_sbva,
    )
}

/// Result of a *projected* Arjun pass (projection-set minimization).
pub(crate) struct ArjunProjResult {
    /// Reduced formula (var IDs renumbered from the input).
    pub formula: CnfFormula,
    /// Minimized show/independent-support set, over `formula`.
    pub show: ShowSet<Reduced>,
    /// Exponent N: original projected_count = reduced_projected_count * 2^N
    /// (N = number of removed *free* show vars; Arjun's MUST-MULTIPLY constant
    /// is always a power of two for projection minimization).
    pub multiplier_exp: u32,
    /// The INPUT->REDUCED variable correspondence, same spaces as
    /// [`ArjunResult::input_to_reduced_lit`] and read off the same `s->cur`
    /// checkpoint, so it is consistent with `formula`/`show`/`multiplier_exp`.
    /// Without it a projected reduction is un-nameable in the caller's variable
    /// space.
    pub input_to_reduced_lit: VarMap<Reduced, Reduced>,
}

/// Result of a *weighted projected* Arjun pass (PWMC projection-set
/// minimization).
pub(crate) struct ArjunWeightedProjResult {
    /// Reduced formula (var IDs renumbered from the input).
    pub formula: CnfFormula,
    /// Minimized show set, over `formula`.
    pub show: ShowSet<Reduced>,
    /// The weights a count over `formula` must be taken under, read back off
    /// the reduction itself.
    pub weights: Weights<Reduced>,
    /// Rational multiplier K: original PWMC = reduced PWMC × K. Unlike the
    /// unweighted path's power-of-two exponent, the weighted multiplier is a
    /// general rational.
    pub multiplier: num_rational::BigRational,
    /// The INPUT->REDUCED variable correspondence, same spaces as
    /// [`ArjunResult::input_to_reduced_lit`], read off the same `s->cur`
    /// checkpoint as every other field. See [`ArjunProjResult`] for why a
    /// projected reduction is un-nameable without it.
    pub input_to_reduced_lit: VarMap<Reduced, Reduced>,
}

/// Result of a *weighted, unprojected* Arjun pass (the full-WMC reduce). The
/// sibling of [`ArjunWeightedProjResult`] with no show set: this pass declares
/// no projection, so there is no minimized set to report and the mass of every
/// eliminated variable is in `multiplier` instead.
///
/// Same fork-harness codec contract as [`ArjunResult`]: `fork_payload`'s
/// `ForkPayload` impl destructures this struct exhaustively.
#[derive(Debug, PartialEq)]
pub(crate) struct ArjunWeightedResult {
    /// Reduced formula (var IDs renumbered from the input).
    pub formula: CnfFormula,
    /// The weights a count over `formula` must be taken under, read back off
    /// the reduction itself.
    pub weights: Weights<Reduced>,
    /// Rational multiplier K: original WMC = reduced WMC × K.
    pub multiplier: num_rational::BigRational,
    /// The INPUT->REDUCED variable correspondence, same spaces as
    /// [`ArjunResult::input_to_reduced_lit`], read off the same `s->cur`
    /// checkpoint as every other field.
    pub input_to_reduced_lit: VarMap<Reduced, Reduced>,
}
