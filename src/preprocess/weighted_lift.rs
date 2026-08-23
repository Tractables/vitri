//! The WEIGHTED analogue of the integer `2^k` count lift: given a
//! [`SimplifiedFormula`] and the instance's per-literal weights, the exact
//! rational scalar that turns a weighted count over the reduced formula back into
//! one over the original, plus the reduced-space weight vector the count must be
//! taken under.
//!
//! # Why a weighted lift is not a `2^k`
//!
//! The integer lift is a CARDINALITY correction: an eliminated variable either
//! doubles the count (unconstrained) or leaves it alone (determined). Under
//! weights neither factor is 2 or 1 —
//!
//! | eliminated because      | integer factor | weighted factor |
//! |-------------------------|----------------|-----------------|
//! | unconstrained / free    | ×2             | ×(w⁻ + w⁺)      |
//! | forced (backbone)       | ×1             | ×w[forced pol]  |
//! | functionally determined | ×1             | ×w⁺ **only if `w⁻ == w⁺`** |
//! | equivalent to a survivor| ×1             | fold into the survivor's weights |
//!
//! — so a `2^k` applied to a weighted sum is simply the wrong number, and the
//! equivalence row is not a scalar at all: the eliminated partner's weights
//! MULTIPLY INTO its representative, changing the weight the reduced count is
//! taken under rather than the scalar applied afterwards.
//!
//! The "only if `w⁻ == w⁺`" row is the one that cannot always be paid. A defined
//! variable's value depends on the model, so an unequal-weight defined variable
//! contributes a model-dependent factor, which no scalar can express.
//! [`dve_eligibility`] is that check, and the caller's correct response is to
//! DISCARD the DVE stage (falling back to the pre-DVE formula, which is exact),
//! never to approximate the factor.
//!
//! # One implementation, two consumers
//!
//! An embedding caller running weighted preprocessing and the standalone export
//! bundle both need exactly this arithmetic — the caller to multiply its own
//! computed value, the bundle to write the multiplier into `preprocess.json`.
//! Everything here is pure (no `Config`, no I/O, no env), so each consumer
//! adapts its own weight source into `orig_w` and shares the rest.
//!
//! # Conventions
//!
//! Every table here is a [`Weights<Original>`] — `(w⁻, w⁺)` per variable of the
//! INPUT formula. The reduced-space table a count is finally taken under is
//! [`VarMap::carry_weights`](crate::preprocess::VarMap::carry_weights)'s job,
//! not this module's.

use num_rational::BigRational;
use num_traits::One;

use crate::cnf::{Literal, Original, VarId, Weights};
use crate::preprocess::dve::types::DveFate;
use crate::preprocess::simplify::SimplifiedFormula;

/// Equivalence-FOLDED per-variable weights.
///
/// Starts from `orig_w` and folds each equivalence-eliminated variable's literal
/// weights into its representative, swapping the polarities when the two are
/// opposite (`e ≡ ¬rep`). Covers BOTH equivalence layers — the preprocessing one
/// (`simplified.equiv_reduced`) and the one DVE discovers — because a
/// representative's true per-model weight is the product over its whole class.
///
/// Backbone and dead variables are NOT folded here: they leave the formula
/// entirely and are a scalar ([`stripped_correction`]), not a weight on a
/// survivor.
pub(crate) fn folded_weights(
    simplified: &SimplifiedFormula,
    orig_w: &Weights<Original>,
) -> Weights<Original> {
    let mut w = orig_w.clone();
    if let Some(eq) = simplified.equiv_reduced.as_ref() {
        for (rep_s, equivs) in &eq.mapping.rep_to_equivs {
            let rep_o = simplified.stripped_var_to_original(*rep_s);
            for &eq_s in equivs {
                let pair = orig_w[var_of(simplified.stripped_var_to_original(eq_s.var))].clone();
                w.fold_into(pair, Literal::new(VarId(rep_o as u32), eq_s.positive));
            }
        }
    }
    fold_dve_equivs(&mut w, simplified);
    w
}

/// A 0-based variable index as the id it is. Preprocessing reports its
/// correspondences as `usize` indices; a weight table is keyed by [`VarId`].
fn var_of(index: usize) -> VarId {
    VarId(index as u32)
}

/// Chase a DVE-discovered equivalence variable `v` (DVE-input space) along its
/// chain of representatives. `Some(survivor)` — the composed literal, over a
/// variable still present in the residual — when the chain ends at a survivor
/// that carries `v`'s value, so `v`'s per-literal weights can be FOLDED into it.
/// `None` when the chain ends at an ELIMINATED variable: folding there would
/// entangle with the scalar correction, so [`dve_eligibility`] reports the whole
/// reduction unsupported instead.
///
/// Every hop is bounds-checked and reads as `None` rather than panicking, so a
/// caller holding a chain it did not build itself (the projected pipeline walks
/// one straight out of the DVE stage) can treat a malformed one as "no survivor"
/// and discard the reduction.
pub(crate) fn dve_equiv_survivor(fates: &[DveFate], v: usize) -> Option<Literal> {
    let mut cur = v;
    let mut same = true;
    loop {
        let rep = fates.get(cur).copied()?.as_equiv()?;
        same = same == rep.positive;
        cur = rep.var.idx();
        if fates.get(cur).copied()?.as_equiv().is_none() {
            break;
        }
    }
    if fates.get(cur).copied().is_none_or(DveFate::eliminated) {
        None
    } else {
        Some(Literal::new(VarId(cur as u32), same))
    }
}

/// Fold every DVE-discovered equivalence's per-literal weights into its surviving
/// representative. `v`'s own weight (already including its preprocessing-equiv
/// partners) is the value folded in. Non-foldable equivalences are skipped —
/// [`dve_eligibility`] refuses the reduction for those, so this `w` is then
/// unused.
fn fold_dve_equivs(w: &mut Weights<Original>, simplified: &SimplifiedFormula) {
    let Some(dve) = simplified.dve_reduced.as_ref() else {
        return;
    };
    for v in 0..dve.fates.len() {
        if dve.fates[v].as_equiv().is_none() {
            continue;
        }
        let Some(surv) = dve_equiv_survivor(&dve.fates, v) else {
            continue;
        };
        let v_o = simplified.pre_dve_var_to_original(v);
        let surv_o = simplified.pre_dve_var_to_original(surv.var.idx());
        let pair = w[var_of(v_o)].clone();
        w.fold_into(pair, Literal::new(VarId(surv_o as u32), surv.positive));
    }
}

/// The scalar owed to the variables STRIPPING removed: `×w[forced polarity]` per
/// backbone literal, `×(w⁻ + w⁺)` per dead (unconstrained) variable.
///
/// Reads RAW weights, not folded ones, and is disjoint from the equivalence
/// layer by construction: stripping runs BEFORE equivalence reduction, so
/// equivalences only ever touch survivors.
pub(crate) fn stripped_correction(
    simplified: &SimplifiedFormula,
    orig_w: &Weights<Original>,
) -> BigRational {
    let mut correction = BigRational::one();
    let Some(stripped) = simplified.stripped.as_ref() else {
        return correction;
    };
    for &(var, pos) in &stripped.removed.backbone {
        let (wn, wp) = &orig_w[var];
        correction *= if pos { wp.clone() } else { wn.clone() };
    }
    for &var in &stripped.removed.dead {
        let (wn, wp) = &orig_w[var];
        correction *= wn.clone() + wp.clone();
    }
    correction
}

/// The scalar owed to the variables DVE removed: `×(w⁻ + w⁺)` for a FREE one,
/// `×w⁺` for a DEFINED one (`w⁺ == w⁻`, guaranteed by [`dve_eligibility`] having
/// passed). DVE-eliminated EQUIVALENCES are skipped — their weight is folded into
/// the surviving representative by [`folded_weights`], so charging them here too
/// would double-count.
///
/// `folded_w` must be [`folded_weights`]' output: DVE runs on the equiv-reduced
/// formula, so every variable it eliminates is a representative whose per-model
/// weight includes its folded partners.
pub(crate) fn dve_correction(
    simplified: &SimplifiedFormula,
    folded_w: &Weights<Original>,
) -> BigRational {
    classify_dve(simplified, folded_w).0
}

/// Whether every elimination DVE made can be paid for exactly — see
/// [`dve_eligibility`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DveEligibility {
    /// Every elimination is scalar-correctable or foldable. The counts are the
    /// eliminations of each kind, for a caller that wants to report them.
    Supported {
        /// Number of DEFINED eliminations with equal-weight polarities (`×w`).
        defined: usize,
        /// Number of FREE eliminations (`×(w⁻+w⁺)`).
        free: usize,
    },
    /// At least one elimination has no exact weighted correction. The caller must
    /// DISCARD the DVE reduction (`simplified.dve_reduced = None`), which returns
    /// the pre-DVE formula — exact, and byte-identical to never having run DVE.
    Unsupported,
}

/// Classify the DVE stage's eliminations against `folded_w`.
///
/// [`DveEligibility::Unsupported`] on either of the two failures a weighted lift
/// cannot express:
/// - a DVE-discovered equivalence whose chain ends at an ELIMINATED variable, so
///   the weight fold has nowhere sound to land;
/// - a DEFINED eliminated variable with `w⁻ != w⁺`, whose contribution is
///   gate-value-dependent and therefore not a scalar.
///
/// Supported classes: FREE (`×(w⁻+w⁺)`), equal-weight DEFINED (`×w`), and
/// equivalences that fold into a residual survivor (no scalar owed).
pub(crate) fn dve_eligibility(
    simplified: &SimplifiedFormula,
    folded_w: &Weights<Original>,
) -> DveEligibility {
    classify_dve(simplified, folded_w).1
}

/// The single pass over the DVE-eliminated variables both
/// [`dve_correction`] and [`dve_eligibility`] are a projection of: what each
/// elimination costs, and whether all of them can be paid for exactly. One pass
/// because the two answers come from the same classification of the same
/// variables by the same rules — split, they could disagree about which
/// eliminations they were describing.
///
/// The correction is meaningful only alongside
/// [`DveEligibility::Supported`]; on the unsupported verdict the caller reverts
/// the stage and the factor is never applied.
fn classify_dve(
    simplified: &SimplifiedFormula,
    folded_w: &Weights<Original>,
) -> (BigRational, DveEligibility) {
    let mut correction = BigRational::one();
    let Some(dve) = simplified.dve_reduced.as_ref() else {
        return (
            correction,
            DveEligibility::Supported {
                defined: 0,
                free: 0,
            },
        );
    };
    let (mut defined, mut free) = (0usize, 0usize);
    let mut supported = true;
    // The free/defined split is TRUE pipeline provenance, not a static
    // occurrence mask — a variable's fate is relative to the elimination order.
    for j in 0..dve.fates.len() {
        match dve.fates[j] {
            DveFate::Kept => continue,
            DveFate::Equiv { .. } => {
                // Folded into a survivor by `fold_dve_equivs`, so no scalar is
                // owed — unless the chain lands nowhere sound, which no fold can
                // express.
                if dve_equiv_survivor(&dve.fates, j).is_none() {
                    supported = false;
                }
            }
            DveFate::Free => {
                let (wn, wp) = &folded_w[var_of(simplified.pre_dve_var_to_original(j))];
                correction *= wn.clone() + wp.clone();
                free += 1;
            }
            DveFate::Defined => {
                let (wn, wp) = &folded_w[var_of(simplified.pre_dve_var_to_original(j))];
                if wn != wp {
                    supported = false;
                }
                correction *= wp.clone();
                defined += 1;
            }
        }
    }
    let eligibility = if supported {
        DveEligibility::Supported { defined, free }
    } else {
        DveEligibility::Unsupported
    };
    (correction, eligibility)
}

/// What a caller must DO with the DVE stage, once [`dve_eligibility`] has
/// classified it — see [`dve_verdict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DveVerdict {
    /// Keep the DVE reduction; the corrections in this module pay for it exactly.
    Keep {
        /// Number of DEFINED eliminations with equal-weight polarities (`×w`).
        defined: usize,
        /// Number of FREE eliminations (`×(w⁻+w⁺)`).
        free: usize,
        /// Number of variables DVE left undetermined (not resolved to a scalar correction).
        residual: u32,
    },
    /// Drop it (`simplified.dve_reduced = None`), falling back to the pre-DVE
    /// formula — exact, and byte-identical to never having run DVE. The string is
    /// the reason, short enough to land verbatim in a report or a log line.
    Revert(&'static str),
}

/// THE keep-or-revert decision for a weighted DVE stage, in one place so no two
/// callers can disagree about which eliminations they are willing to pay for.
///
/// Two independent reasons to revert:
/// 1. **Unsupported** — [`dve_eligibility`] found an elimination no scalar can
///    express. Always fatal to the stage.
/// 2. **Non-empty residual, without freezing** — every elimination is payable,
///    but DVE's resolution can restructure the residual LARGER than the pre-DVE
///    formula. `freeze` says the caller froze the unequal-weight variables out of
///    DVE ([`Weights::unequal_vars`]), which leaves a residual of only
///    scalar-correctable eliminations plus functionally-determined frozen
///    variables — safe to keep. Without it, only a FULL elimination (residual 0,
///    i.e. a pure scalar answer) is worth the risk.
///
/// The second reason is a cost decision, not a soundness one: reverting is always
/// sound, and keeping is sound whenever the verdict is not `Unsupported`.
pub(crate) fn dve_verdict(
    simplified: &SimplifiedFormula,
    folded_w: &Weights<Original>,
    freeze: bool,
) -> DveVerdict {
    let residual = match dve_residual_vars(simplified) {
        Some(n) => n,
        // No DVE stage ran at all — nothing to keep or revert.
        None => {
            return DveVerdict::Keep {
                defined: 0,
                free: 0,
                residual: 0,
            };
        }
    };
    match dve_eligibility(simplified, folded_w) {
        DveEligibility::Unsupported => DveVerdict::Revert("unsupported elimination"),
        DveEligibility::Supported { defined, free } => {
            if residual == 0 || freeze {
                DveVerdict::Keep {
                    defined,
                    free,
                    residual,
                }
            } else {
                DveVerdict::Revert("residual vars remain — only full elimination is supported")
            }
        }
    }
}

fn dve_residual_vars(simplified: &SimplifiedFormula) -> Option<u32> {
    simplified.dve_reduced.as_ref().map(|d| d.formula.num_vars)
}

/// THE whole scalar lift for a weighted count over `simplified.reduced_formula()`:
/// the stripping correction times the DVE correction. Composed here so the two
/// halves cannot be applied in one place and forgotten in another.
///
/// `wmc(original) == wmc(reduced_formula, carried weights) × weighted_lift`.
pub(crate) fn weighted_lift(
    simplified: &SimplifiedFormula,
    orig_w: &Weights<Original>,
    folded_w: &Weights<Original>,
) -> BigRational {
    stripped_correction(simplified, orig_w) * dve_correction(simplified, folded_w)
}
