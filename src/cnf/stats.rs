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
