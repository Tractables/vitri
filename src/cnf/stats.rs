//! Whole-formula shape statistics, and the `coloring_like` predicate over
//! them.
//!
//! A formula whose variable occurrences and clause widths are both
//! near-uniform is shaped like a graph-colouring encoding, which two
//! independent decisions read: Arjun's bounded-variable-addition policy under
//! `auto`, and the vtree portfolio's candidate gates. Both must agree on what
//! "coloring-like" means, so the two statistics and the thresholds over them
//! live here — a function of the clause set alone, beside the per-variable
//! views in [`occ`](super::occ).

use super::CnfFormula;

/// The clause-width half of the `coloring_like` predicate: the coefficient of
/// variation (stddev / mean) of clause widths. Degenerate inputs (fewer than 2
/// clauses, zero mean width) score `0.0`.
pub(crate) fn clause_width_cv(formula: &CnfFormula) -> f64 {
    let widths: Vec<f64> = formula
        .clauses
        .iter()
        .map(|c| c.literals.len() as f64)
        .collect();
    if widths.len() > 1 {
        let wm = widths.iter().sum::<f64>() / widths.len() as f64;
        let wv = widths.iter().map(|x| (x - wm).powi(2)).sum::<f64>() / widths.len() as f64;
        if wm > 0.0 { wv.sqrt() / wm } else { 0.0 }
    } else {
        0.0
    }
}

/// The occurrence half of the `coloring_like` predicate: the coefficient of
/// variation (stddev / mean) of per-variable occurrence counts, over the
/// variables that actually occur. Degenerate inputs (fewer than 2 occurring
/// variables) score `0.0`.
pub(crate) fn var_occurrence_cv(formula: &CnfFormula) -> f64 {
    let occ = super::occ::frequency(&formula.clauses, formula.num_vars as usize);
    let active: Vec<f64> = occ.iter().filter(|&&c| c > 0).map(|&c| c as f64).collect();
    if active.len() > 1 {
        let mean = active.iter().sum::<f64>() / active.len() as f64;
        let var = active.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / active.len() as f64;
        var.sqrt() / mean
    } else {
        0.0
    }
}

/// Occurrence-dispersion ceiling of the `coloring_like` predicate.
const COLORING_OCC_CV_MAX: f64 = 0.5;
/// Clause-width-dispersion ceiling of the `coloring_like` predicate.
pub(crate) const COLORING_WIDTH_CV_MAX: f64 = 0.30;

/// The `coloring_like` predicate over its two statistics — the one place its
/// thresholds live.
pub(crate) fn coloring_like_predicate(occ_cv: f64, width_cv: f64) -> bool {
    occ_cv < COLORING_OCC_CV_MAX && width_cv < COLORING_WIDTH_CV_MAX
}

/// How evenly a formula's clause widths and variable occurrences are spread,
/// and the near-uniform verdict two of this crate's decisions read off them.
///
/// A formula whose clause widths and variable occurrences are both near-uniform
/// is shaped like a graph-colouring encoding. Two independent decisions here
/// consult that: Arjun's bounded-variable-addition policy under
/// [`ArjunSbva::Auto`](crate::preprocess::ArjunSbva::Auto), which skips the pass
/// on such an input, and the vtree portfolio's candidate gate. Both call this,
/// so a caller reporting these numbers is reporting what those decisions saw —
/// not a second measurement that agrees with them today.
///
/// Both coefficients are dispersion relative to the mean, so `0.0` is perfectly
/// uniform and there is no upper bound. A formula too small to have a spread —
/// fewer than two clauses, fewer than two occurring variables — scores `0.0`,
/// which reads as uniform.
///
/// `#[non_exhaustive]`: the verdict may come to read a third statistic, and a
/// caller that only prints these two should not have to be recompiled for that.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct StructureProfile {
    /// Coefficient of variation of clause width — the standard deviation of the
    /// clause lengths over their mean.
    pub clause_width_cv: f64,
    /// Coefficient of variation of per-variable occurrence count, over the
    /// variables that occur at all. A variable in no clause is not a variable
    /// with an occurrence count of zero for this purpose; it is absent.
    pub var_occurrence_cv: f64,
    /// Whether both coefficients are inside the thresholds that make an input
    /// look like a graph-colouring encoding.
    pub coloring_like: bool,
}

impl StructureProfile {
    /// Construct a profile from already-measured coefficients.
    ///
    /// This is the counterpart to [`StructureProfile::measure`] for an
    /// embedding that already owns the source formula's statistics and should
    /// not scan or reconstruct that formula merely to pass its profile into a
    /// selection context.
    pub fn from_coefficients(clause_width_cv: f64, var_occurrence_cv: f64) -> Self {
        StructureProfile {
            clause_width_cv,
            var_occurrence_cv,
            coloring_like: coloring_like_predicate(var_occurrence_cv, clause_width_cv),
        }
    }

    /// Measure `formula`. One scan of the clause set for each coefficient.
    ///
    /// This is a measurement of the formula it is handed, so a preprocessed
    /// formula and the raw one it came from can profile differently — which of
    /// the two a decision should read is that decision's to settle.
    pub fn measure(formula: &CnfFormula) -> Self {
        let clause_width_cv = clause_width_cv(formula);
        let var_occurrence_cv = var_occurrence_cv(formula);
        StructureProfile::from_coefficients(clause_width_cv, var_occurrence_cv)
    }
}
