use crate::cnf::CnfFormula;
use crate::preprocess::preprocess_backbone_eq_iter;
use crate::tests::common::clause;
use std::time::{Duration, Instant};

/// A forcing chain (x0) ∧ (¬x0∨x1) ∧ … ∧ (¬x_{k-1}∨x_k) pins x0..=x_k true,
/// plus `free` unoccurring vars on top.
fn forcing_chain(k: u32, free: u32) -> CnfFormula {
    let mut clauses = vec![clause(&[(0, true)])];
    for i in 0..k {
        clauses.push(clause(&[(i, false), (i + 1, true)]));
    }
    CnfFormula {
        num_vars: k + 1 + free,
        clauses,
    }
}

fn is_unsat(f: &CnfFormula) -> bool {
    f.clauses.iter().any(|c| c.literals.is_empty())
}

// An imminent deadline must clamp every phase budget so the whole pipeline
// returns well within the budget — even though the backbone/equiv budgets are the
// full 300 s — and the result stays sound (a clamped/terminated solve is
// UNKNOWN, never mistaken for UNSAT on this satisfiable input).
#[test]
fn imminent_deadline_bounds_pipeline_and_stays_sound() {
    let formula = forcing_chain(20, 20);
    let t = Instant::now();
    let out = preprocess_backbone_eq_iter(
        &formula,
        Duration::from_secs(300),
        Some(Duration::from_secs(300)),
        Some(Instant::now() + Duration::from_millis(50)),
    );
    assert!(
        t.elapsed() < Duration::from_secs(2),
        "imminent deadline should bound preprocess, took {:?}",
        t.elapsed(),
    );
    assert!(
        !is_unsat(&out.formula),
        "satisfiable input must not be reported UNSAT"
    );
}

// With no deadline the clamp is inert, and the pipeline still detects the
// chain's backbone.
#[test]
fn no_deadline_finds_backbone_unchanged() {
    let formula = forcing_chain(20, 0);
    let out = preprocess_backbone_eq_iter(
        &formula,
        Duration::from_secs(300),
        Some(Duration::from_secs(300)),
        None,
    );
    assert!(!is_unsat(&out.formula));
    // The whole chain is backbone; the forced vars must be detected.
    assert!(
        out.stats.forced_vars >= 1,
        "expected forced vars with no deadline, got {}",
        out.stats.forced_vars,
    );
}
