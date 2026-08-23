//! Gate detection and variable elimination for model-count-preserving BVE.
//!
//! Detects AND, OR, XOR, and ITE gate patterns in a CNF formula and eliminates
//! gate output variables. The eliminated variables can be re-introduced into the
//! compiled TDD via vtree extension and clause conjunction.
//!
//! Gate patterns (output variable y, inputs x₁, x₂, ...):
//!
//! - AND `y = x₁ ∧ x₂`: (¬y∨x₁), (¬y∨x₂), (y∨¬x₁∨¬x₂)
//! - OR  `y = x₁ ∨ x₂`: (y∨¬x₁), (y∨¬x₂), (¬y∨x₁∨x₂)
//! - XOR `y = x₁ ⊕ x₂`: 4 ternary clauses encoding parity
//! - ITE `y = ITE(s,a,b)`: 4 ternary clauses encoding if-then-else

use rustc_hash::FxHashSet;

use crate::cnf::VarId;
use crate::cnf::occ;
use crate::cnf::{Clause, CnfFormula};

/// Type of gate detected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GateType {
    /// `y = x₁ ∧ x₂`.
    And,
    /// `y = x₁ ∨ x₂`.
    Or,
    /// `y = x₁ ⊕ x₂`.
    Xor,
    /// `y = ¬(x₁ ⊕ x₂)` — XOR with a complemented output. Detected from the same
    /// 4-clause encoding as [`GateType::Xor`] but with the opposite output
    /// polarity (odd-parity clause signs). Kept distinct because a consumer has
    /// to rebuild `¬(a ⊕ b)`; collapsing it into `Xor` names the wrong function,
    /// and a kept biconditional then contradicts the formula (count → 0).
    Xnor,
    /// `y = ITE(s, a, b)`.
    Ite,
}

/// A detected gate, in the terms preprocessing reads: which pattern matched, and
/// which clauses encode it. The output variable itself is recorded once, in
/// [`GateMapping::eliminated`].
#[derive(Clone, Debug)]
pub(super) struct Gate {
    pub gate_type: GateType,
    /// Indices of clauses in the original formula that define this gate.
    pub clause_indices: Vec<usize>,
}

/// Result of gate detection: the gates found, in elimination order.
#[derive(Clone, Debug)]
pub(super) struct GateMapping {
    /// Gates in elimination order (first eliminated = last re-introduced).
    pub gates: Vec<Gate>,
    /// Set of eliminated variable IDs, for quick lookup.
    pub eliminated: FxHashSet<VarId>,
}

impl GateMapping {
    /// True when no gates were detected — nothing to eliminate or re-introduce.
    pub(super) fn is_empty(&self) -> bool {
        self.gates.is_empty()
    }

    /// Number of gate output variables eliminated by this mapping.
    pub(super) fn num_eliminated(&self) -> usize {
        self.gates.len()
    }
}

/// Detect AND/OR/XOR/ITE gates and produce a mapping of eliminable variables.
///
/// Iterates until fixpoint: eliminating one gate may make another variable
/// eliminable (its "usage" clauses were gate clauses of the just-eliminated var).
pub(super) fn detect_gates(formula: &CnfFormula) -> GateMapping {
    let num_vars = formula.num_vars as usize;
    let (pos_occs, neg_occs) = occ::occurrence_lists(&formula.clauses, num_vars);

    let mut gates = Vec::new();
    let mut eliminated: FxHashSet<VarId> = FxHashSet::default();
    let mut consumed_clauses: FxHashSet<usize> = FxHashSet::default();

    loop {
        let mut changed = false;
        for v in 0..num_vars {
            let var = VarId(v as u32);
            if eliminated.contains(&var) {
                continue;
            }

            // Cheap guard: skip vars whose occurrences have all been consumed by prior gates.
            if pos_occs[v]
                .iter()
                .chain(&neg_occs[v])
                .all(|ci| consumed_clauses.contains(ci))
            {
                continue;
            }

            let active_pos = filter_active(&pos_occs[v], &consumed_clauses);
            let active_neg = filter_active(&neg_occs[v], &consumed_clauses);

            let ctx = GateCtx {
                var,
                pos: &active_pos,
                neg: &active_neg,
                clauses: &formula.clauses,
                eliminated: &eliminated,
            };
            if let Some(gate) = try_detect_gate(&ctx) {
                consumed_clauses.extend(gate.clause_indices.iter().copied());
                eliminated.insert(var);
                gates.push(gate);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    GateMapping { gates, eliminated }
}

/// Filter an occurrence list down to clauses not yet consumed by a detected gate.
fn filter_active(indices: &[usize], consumed: &FxHashSet<usize>) -> Vec<usize> {
    indices
        .iter()
        .copied()
        .filter(|ci| !consumed.contains(ci))
        .collect()
}

/// One candidate output variable, and everything a detector has to look at to
/// judge it: the still-active clauses it occurs in under each sign, the
/// formula's clauses those index into, and the outputs eliminated so far — an
/// input that is itself an eliminated output disqualifies the gate.
struct GateCtx<'a> {
    var: VarId,
    pos: &'a [usize],
    neg: &'a [usize],
    clauses: &'a [Clause],
    eliminated: &'a FxHashSet<VarId>,
}

impl GateCtx<'_> {
    /// The occurrence count the XOR and ITE encodings share: two clauses of
    /// each sign. Their four-clause tables have no other shape.
    fn is_ternary_shaped(&self) -> bool {
        self.pos.len() == 2 && self.neg.len() == 2
    }

    /// Every active clause the candidate occurs in, positives first. This is
    /// the clause set a matched gate records.
    fn all_indices(&self) -> Vec<usize> {
        self.pos.iter().chain(self.neg.iter()).copied().collect()
    }
}

/// Try to detect any supported gate pattern for the candidate.
fn try_detect_gate(ctx: &GateCtx<'_>) -> Option<Gate> {
    try_and_or_gate(ctx, GateType::And)
        .or_else(|| try_and_or_gate(ctx, GateType::Or))
        .or_else(|| try_xor_gate(ctx))
        .or_else(|| try_ite_gate(ctx))
}

/// Try to recognise an AND / OR gate. AND and OR are structural duals, so a single
/// function handles both (parameterised by `kind`).
///
/// AND `y = x₁ ∧ … ∧ xₙ`:  n binary clauses `(¬y ∨ xᵢ)`  +  1 long clause `(y ∨ ¬x₁ ∨ … ∨ ¬xₙ)`
/// OR  `y = x₁ ∨ … ∨ xₙ`:  n binary clauses `(y ∨ ¬xᵢ)`  +  1 long clause `(¬y ∨ x₁ ∨ … ∨ xₙ)`
fn try_and_or_gate(ctx: &GateCtx<'_>, kind: GateType) -> Option<Gate> {
    // For AND the output appears positive in the long clause and negative in the binaries;
    // for OR the polarities are flipped. `y_pos_in_long` captures that one-bit difference.
    let y_pos_in_long = match kind {
        GateType::And => true,
        GateType::Or => false,
        _ => return None,
    };
    let (binary_indices, long_indices) = if y_pos_in_long {
        (ctx.neg, ctx.pos)
    } else {
        (ctx.pos, ctx.neg)
    };

    if long_indices.len() != 1 || binary_indices.is_empty() {
        return None;
    }

    let mut inputs_from_binary = collect_and_or_binary_inputs(ctx, binary_indices, y_pos_in_long)?;

    // The long clause must restate the same input set with the opposite polarity.
    let long_clause = &ctx.clauses[long_indices[0]];
    if long_clause.literals.len() != 1 + inputs_from_binary.len() {
        return None;
    }
    let mut inputs_from_long: Vec<VarId> = Vec::with_capacity(inputs_from_binary.len());
    for lit in &long_clause.literals {
        if lit.var == ctx.var {
            if lit.positive != y_pos_in_long {
                return None;
            }
        } else {
            if lit.positive == y_pos_in_long {
                return None; // inputs in long clause must be opposite polarity to y
            }
            inputs_from_long.push(lit.var);
        }
    }

    inputs_from_binary.sort_unstable();
    inputs_from_long.sort_unstable();
    if inputs_from_binary != inputs_from_long {
        return None;
    }

    Some(Gate {
        gate_type: kind,
        clause_indices: ctx.all_indices(),
    })
}

/// Collect the `xᵢ` inputs from the binary clauses of an AND/OR gate, validating each.
/// For AND (`y_pos_in_long=true`) each clause must be `(¬y ∨ xᵢ)`; for OR it's `(y ∨ ¬xᵢ)`.
/// In both cases the polarity of `xᵢ` in the binary clause equals `y_pos_in_long`.
fn collect_and_or_binary_inputs(
    ctx: &GateCtx<'_>,
    binary_indices: &[usize],
    y_pos_in_long: bool,
) -> Option<Vec<VarId>> {
    let mut inputs = Vec::with_capacity(binary_indices.len());
    for &ci in binary_indices {
        let clause = &ctx.clauses[ci];
        if clause.literals.len() != 2 {
            return None;
        }
        let other = clause.literals.iter().find(|l| l.var != ctx.var)?;
        if other.positive != y_pos_in_long || ctx.eliminated.contains(&other.var) {
            return None;
        }
        inputs.push(other.var);
    }
    Some(inputs)
}

/// Check for XOR gate: `y = x₁ ⊕ x₂`.
///
/// A 3-variable XOR has exactly 4 CNF clauses — one per falsified assignment.
/// Pattern: 2 positive + 2 negative ternary clauses, all over the same triple
/// `{y, x₁, x₂}`, with four *distinct* sign patterns sharing the same parity
/// of positive literals. (Same-parity encodes either `y = x₁⊕x₂` or its
/// negation; both are valid XOR gates up to output polarity.)
fn try_xor_gate(ctx: &GateCtx<'_>) -> Option<Gate> {
    if !ctx.is_ternary_shaped() {
        return None;
    }
    let all_indices = ctx.all_indices();

    // All 4 clauses must be ternary over exactly the 2 non-output XOR inputs.
    let inputs = collect_ternary_other_vars(ctx, &all_indices, 2)?;

    // Sign pattern per clause: `(y_positive, x₁_positive, x₂_positive)`.
    let mut patterns: Vec<(bool, bool, bool)> = all_indices
        .iter()
        .map(|&ci| ternary_signs(&ctx.clauses[ci], ctx.var, inputs[0], inputs[1]))
        .collect();
    patterns.sort();
    patterns.dedup();
    if patterns.len() != 4 {
        return None; // duplicate sign pattern → not a full XOR encoding
    }

    // Shared parity fixes the output polarity: even → XOR, odd → XNOR (see
    // `GateType::Xnor`).
    let parity_of = |(a, b, c): (bool, bool, bool)| (a as u32 + b as u32 + c as u32) % 2;
    let parity = parity_of(patterns[0]);
    if !patterns.iter().all(|&p| parity_of(p) == parity) {
        return None;
    }
    let gate_type = if parity == 0 {
        GateType::Xor
    } else {
        GateType::Xnor
    };

    Some(Gate {
        gate_type,
        clause_indices: all_indices,
    })
}

/// Extract positive/negative signs of `y`, `a`, `b` within `clause`. If any of
/// the three variables is missing the corresponding slot is `false` — callers
/// that care about presence should verify it separately.
fn ternary_signs(clause: &Clause, y: VarId, a: VarId, b: VarId) -> (bool, bool, bool) {
    let mut sy = false;
    let mut sa = false;
    let mut sb = false;
    for lit in &clause.literals {
        if lit.var == y {
            sy = lit.positive;
        } else if lit.var == a {
            sa = lit.positive;
        } else if lit.var == b {
            sb = lit.positive;
        }
    }
    (sy, sa, sb)
}

/// Walk every literal of every clause in `clause_indices`, asserting each is a
/// ternary clause, and collect the set of variables other than the candidate.
/// Returns `None` if any clause isn't ternary, any "other" variable is already
/// eliminated, or the set size ≠ `expected_size`.
fn collect_ternary_other_vars(
    ctx: &GateCtx<'_>,
    clause_indices: &[usize],
    expected_size: usize,
) -> Option<Vec<VarId>> {
    let mut others: FxHashSet<VarId> = FxHashSet::default();
    for &ci in clause_indices {
        let clause = &ctx.clauses[ci];
        if clause.literals.len() != 3 {
            return None;
        }
        for lit in &clause.literals {
            if lit.var != ctx.var {
                others.insert(lit.var);
            }
        }
    }
    if others.len() != expected_size {
        return None;
    }
    if others.iter().any(|v| ctx.eliminated.contains(v)) {
        return None;
    }
    Some(others.into_iter().collect())
}

/// Check for ITE gate: `y = ITE(s, a, b) = (s ∧ a) ∨ (¬s ∧ b)`.
///
/// Pattern: 2 positive + 2 negative ternary clauses over exactly 4 variables
/// `{y, s, a, b}`. The 4 clauses encode: if `s` then `y ↔ a`, if `¬s` then `y ↔ b`.
///
/// We don't know which of the three "other" variables is the selector `s`, so
/// we try each of the 3 × 2 role assignments (pick `s`, then pick which of the
/// remaining two is `a`) and check against the canonical clause set.
fn try_ite_gate(ctx: &GateCtx<'_>) -> Option<Gate> {
    if !ctx.is_ternary_shaped() {
        return None;
    }

    let all_indices = ctx.all_indices();
    let inputs = collect_ternary_other_vars(ctx, &all_indices, 3)?;

    for (si, &selector) in inputs.iter().enumerate() {
        let non_selectors: [VarId; 2] = {
            let mut it = inputs
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != si)
                .map(|(_, &v)| v);
            [it.next().unwrap(), it.next().unwrap()]
        };
        // Try both (a, b) role assignments of the two non-selector variables.
        for &(a, b) in &[
            (non_selectors[0], non_selectors[1]),
            (non_selectors[1], non_selectors[0]),
        ] {
            if check_ite_pattern(ctx, selector, a, b, &all_indices) {
                return Some(Gate {
                    gate_type: GateType::Ite,
                    clause_indices: all_indices,
                });
            }
        }
    }

    None
}

/// Verify that the 4 clauses match the canonical ITE encoding `y = ITE(s, a, b)`:
///
/// ```text
///   (¬s ∨ ¬a ∨  y)   — (s, a, y) = (F, F, T)   [s=1,a=0 forces y=1]
///   (¬s ∨  a ∨ ¬y)   — (s, a, y) = (F, T, F)   [s=1,a=1 forces y=0 wrong]
///   ( s ∨ ¬b ∨  y)   — (s, b, y) = (T, F, T)
///   ( s ∨  b ∨ ¬y)   — (s, b, y) = (T, T, F)
/// ```
///
/// We return `true` iff all 4 expected clauses are present. The caller probes
/// alternative role assignments separately (selector/a/b) and the negated-output
/// form is caught by those probes swapping `a ↔ b`.
fn check_ite_pattern(
    ctx: &GateCtx<'_>,
    s: VarId,
    a: VarId,
    b: VarId,
    clause_indices: &[usize],
) -> bool {
    let sign_of = |clause: &Clause, v: VarId| -> Option<bool> {
        clause
            .literals
            .iter()
            .find(|l| l.var == v)
            .map(|l| l.positive)
    };

    // Indexed 0..=3 in the same order as the comment above.
    let mut found = [false; 4];

    for &ci in clause_indices {
        let clause = &ctx.clauses[ci];
        // Every clause must mention y and s; exactly one of a / b.
        let sy = match sign_of(clause, ctx.var) {
            Some(v) => v,
            None => return false,
        };
        let ss = match sign_of(clause, s) {
            Some(v) => v,
            None => return false,
        };
        let sa = sign_of(clause, a);
        let sb = sign_of(clause, b);

        let idx = match (ss, sa, sb) {
            // ¬s branch: uses `a`.
            (false, Some(false), None) if sy => 0,
            (false, Some(true), None) if !sy => 1,
            // s branch: uses `b`.
            (true, None, Some(false)) if sy => 2,
            (true, None, Some(true)) if !sy => 3,
            _ => return false,
        };
        if found[idx] {
            return false; // duplicate clause for the same slot
        }
        found[idx] = true;
    }

    found.iter().all(|&f| f)
}
