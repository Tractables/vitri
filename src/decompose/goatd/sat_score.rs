//! CNF-derived weights for goatd's sampled elimination orders.
//!
//! Goatd's sampled cores draw the next vertex out of the tie set around the
//! minimum priority, and give a smaller weight more probability. Ties there
//! are common, so this fills the slot with structure from the formula rather
//! than an arbitrary choice. The deterministic cores ignore these weights,
//! and a hedge stage replaces them with a ranking goatd derives itself, so
//! what this module sets is the draw inside the plain sampled orders and the
//! restarts.
//!
//! The weight is the degree-normalized Jeroslow-Wang score,
//! `J(v) / (1 + clause_count(v))` where `J(v) = Σ_{c ∋ v or ¬v} 2^(-|c|)`,
//! quantized and inverted, so a vertex whose clauses are short relative to
//! how often it occurs is drawn earlier.

use crate::cnf::CnfFormula;

/// The tie-break weight per vertex: degree-normalized Jeroslow-Wang, quantized
/// and inverted into goatd's smaller-is-earlier convention.
pub(super) fn compute_weight(formula: &CnfFormula, total_vertices: u32) -> Vec<u32> {
    let mut weights = quantize(&jw_degree_normalized(formula, total_vertices));
    for w in &mut weights {
        *w = u32::MAX - *w;
    }
    weights
}

fn jw_degree_normalized(formula: &CnfFormula, total_vertices: u32) -> Vec<f64> {
    let n = total_vertices as usize;
    let mut score = vec![0.0f64; n];
    let mut count = vec![0u32; n];
    for clause in formula.clauses() {
        let w = 2f64.powi(-(clause.literals.len() as i32));
        for lit in &clause.literals {
            let idx = lit.var.idx();
            score[idx] += w;
            count[idx] += 1;
        }
    }
    for (s, &c) in score.iter_mut().zip(&count) {
        *s /= 1.0 + f64::from(c);
    }
    score
}

/// Quantize the scores into goatd's weight range. Scores are scaled against the
/// formula's own maximum, so the spread does not depend on the formula's size.
///
/// The scale is 1e6, which is a small part of the `u32` range, and the
/// conversion truncates. A vertex scoring below a millionth of the maximum
/// therefore lands at zero, and `compute_weight` inverts that into `u32::MAX` —
/// the weight goatd draws least often of all. One outlying score can put a real
/// share of a formula's variables there.
fn quantize(scores: &[f64]) -> Vec<u32> {
    let max = scores.iter().cloned().fold(0.0f64, f64::max);
    if max <= 0.0 {
        return vec![0u32; scores.len()];
    }
    let scale = 1_000_000.0 / max;
    scores
        .iter()
        .map(|&w| {
            let v = (w * scale) as i64;
            v.clamp(0, 1_000_000) as u32
        })
        .collect()
}
