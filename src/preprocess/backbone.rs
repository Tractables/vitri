//! Shared literal-partition types and helpers for backbone detection and
//! literal-equivalence detection.
//!
//! See: "The Power of Literal Equivalence in Model Counting" (AAAI-21).

use super::cadical_ffi::CaDiCal;

use crate::cnf::{Literal, VarId};

// ── Result types ───────────────────────────────────────────────────────────

pub(super) struct BackboneResult {
    pub forced: Vec<Literal>,
    pub probes_completed: usize,
    /// Time spent on the initial SAT solve.
    pub solve_ms: u64,
    pub unsat: bool,
    /// Variables proven non-backbone via CaDiCaL's `fixed()`.
    pub fixed_found: usize,
    /// Variables eliminated via CaDiCaL's `flippable()`.
    pub flippable_eliminated: usize,
    pub model_eliminated: usize,
}

pub(super) struct EquivResult {
    pub equivalences: Vec<(Literal, Literal)>,
    pub probes_completed: usize,
    pub unsat: bool,
}

// ── Counter-model refinement ─────────────────────────────────────────────────

/// Split `candidates` against a new counter-model. Returns `(stay, split)`:
/// `stay` holds candidates that still agree with the representative in
/// `new_model` (still equivalence candidates), `split` holds those that
/// disagree (proven non-equivalent). Splitting is relative to the
/// representative's truth value in this model (`rep_true_in_model`) rather
/// than a fixed polarity — the fixed-polarity version livelocks on a backbone
/// literal when refining from the "false" direction.
pub(super) fn refine_candidates(
    candidates: &[i32],
    new_model: &[i32],
    rep_true_in_model: bool,
) -> (Vec<i32>, Vec<i32>) {
    let mut stay = Vec::new();
    let mut split = Vec::new();

    for &lit in candidates {
        let model_val = new_model[VarId::from_dimacs(lit).idx()];
        let lit_true_in_model = (lit > 0 && model_val > 0) || (lit < 0 && model_val < 0);
        if lit_true_in_model == rep_true_in_model {
            stay.push(lit);
        } else {
            split.push(lit);
        }
    }

    (stay, split)
}

// ── Helpers ────────────────────────────────────────────────────────────────

pub(super) fn read_model(solver: &mut CaDiCal, num_vars: usize) -> Vec<i32> {
    let mut model = vec![0i32; num_vars];
    for (i, slot) in model.iter_mut().enumerate() {
        *slot = solver.val(VarId(i as u32).to_dimacs());
    }
    model
}
