//! CNF-derived score that biases the elimination cores' tie-breaking.
//!
//! The graph-level priority ties often (measured: 79.5% of min-fill pops on
//! ISCAS CNFs have ≥2 candidates tied on exact (fill, degree)). A SAT-aware
//! score fills that tie slot with structural signal instead of uniform random
//! salt.
//!
//! The shipped score is the degree-normalized Jeroslow-Wang weight,
//! `J(v) / (1 + clause_count(v))` where `J(v) = Σ_{c ∋ v or ¬v} 2^(-|c|)`,
//! quantized to `u32` and inverted into the HIGH direction: the elimination
//! order is the *reverse* of SAT branching order, so a variable a SAT heuristic
//! would branch on first must be eliminated last. `compute_weight` returns it
//! in the argmin convention (smaller = eliminated first);
//! `width_opt::to_sample_weight` flips that into the sampling convention the
//! tie-set samplers want.

use crate::cnf::CnfFormula;

/// The tie-break weight per vertex: degree-normalized Jeroslow-Wang,
/// quantized, then inverted so the highest-scoring variable is eliminated LAST.
pub(super) fn compute_weight(formula: &CnfFormula, total_vertices: u32) -> Vec<u32> {
    let q = quantize(&jw_degree_normalized(formula, total_vertices));
    // HIGH → late elimination (argmin convention eliminates smallest key first).
    q.into_iter().map(|w| u32::MAX - w).collect()
}

fn jeroslow_wang(formula: &CnfFormula, total_vertices: u32) -> Vec<f64> {
    let mut score = vec![0.0f64; total_vertices as usize];
    for clause in &formula.clauses {
        let w = 2f64.powi(-(clause.literals.len() as i32));
        for lit in &clause.literals {
            score[lit.var.0 as usize] += w;
        }
    }
    score
}

fn jw_degree_normalized(formula: &CnfFormula, total_vertices: u32) -> Vec<f64> {
    let jw = jeroslow_wang(formula, total_vertices);
    let cnt = clause_count(formula, total_vertices);
    jw.iter()
        .zip(cnt.iter())
        .map(|(&s, &c)| s / (1.0 + c))
        .collect()
}

fn clause_count(formula: &CnfFormula, total_vertices: u32) -> Vec<f64> {
    let mut score = vec![0.0f64; total_vertices as usize];
    for clause in &formula.clauses {
        for lit in &clause.literals {
            score[lit.var.0 as usize] += 1.0;
        }
    }
    score
}

/// Quantize f64 scores to u32 for the heap key. Scores are normalized to the
/// observed max so the 1e6 multiplier maps to full u32 range regardless of the
/// formula's size.
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
