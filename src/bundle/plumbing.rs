//! Shared plumbing: the refutation bundle, the stage configuration and the
//! small helpers all three chains reach for.

use super::*;

use crate::cnf::WeightTable;

// ── Shared plumbing ───────────────────────────────────────────────────────────

/// The bundle for an instance `clauses` refutes, or `None` when it carries no
/// refutation.
///
/// Every stage that can derive the empty clause asks here, and the check is
/// repeated after each of them because the empty clause is the one thing DIMACS
/// cannot spell: writing it emits a lone `0` line, which parsers (including this
/// crate's, as a SATLIB end marker) read as anything but a contradiction,
/// silently turning UNSAT into a nonzero count. The answer is the synthetic
/// contradiction [`unsat_bundle`] builds, whichever chain asked.
///
/// `show_vars_reduced_dimacs` is the projected chains' declared set: a
/// refutation bundle is a contradiction over the ORIGINAL variable space, so its
/// "reduced" formula is renumbered from the input by nothing at all and the
/// declared set already reads over it.
///
/// `decision_trace` is the trace the stage that refuted the instance had
/// recorded so far. It is an argument rather than something a caller patches
/// on afterwards, so a refutation bundle carries the same trace as the bundle
/// the same stage would have produced had it not refuted.
pub(super) fn refuted(
    clauses: &[Clause],
    num_vars: u32,
    mode: Mode,
    show_vars_reduced_dimacs: Option<ShowSet<Reduced>>,
    stages: StageReport,
    telemetry: PreprocessTelemetry,
    decision_trace: Option<PreprocessDecisionTrace>,
) -> Option<PreprocessBundle> {
    crate::cnf::contains_empty_clause(clauses).then(|| {
        unsat_bundle(
            num_vars,
            mode,
            show_vars_reduced_dimacs,
            stages,
            telemetry,
            decision_trace,
        )
    })
}

/// The bundle for an instance preprocessing proved UNSAT: a two-unit-clause
/// contradiction over the original variable space, the identity map, and no
/// lift. Count 0 both before and after, so the lift identity holds trivially —
/// and, unlike the empty clause, it survives a DIMACS write/read round trip.
pub(super) fn unsat_bundle(
    num_vars: u32,
    mode: Mode,
    show_vars_reduced_dimacs: Option<ShowSet<Reduced>>,
    stages: StageReport,
    telemetry: PreprocessTelemetry,
    decision_trace: Option<PreprocessDecisionTrace>,
) -> PreprocessBundle {
    debug_assert!(num_vars >= 1, "an UNSAT instance has at least one variable");
    let x = VarId::from_dimacs(1);
    PreprocessBundle {
        reduced: CnfFormula::from_parts(
            num_vars,
            vec![
                Clause::new(vec![Literal::new(x, true)]),
                Clause::new(vec![Literal::new(x, false)]),
            ],
        ),
        record: PreprocessRecord {
            // Present iff the mode is `compile`, exactly as on the satisfiable
            // path. Nothing was eliminated here — the reduced formula is a
            // contradiction over the original variable space — so the identity
            // is the truthful map, and it keeps the field's presence rule a
            // property of the mode rather than of the outcome.
            original_to_reduced_dimacs: matches!(mode, Mode::Compile)
                .then(|| OriginalMap::identity(num_vars)),
            unsat: true,
            show_vars_reduced_dimacs,
            // Nothing was eliminated, so there is nothing to lift back —
            // whatever the mode, and whatever the weights. `reduced_weights`
            // stays absent with it: a refuted instance has count 0 whatever the
            // weights are, so a table would be decoration on an answer that is
            // already known.
            ..PreprocessRecord::new(
                mode,
                RecordLift::neutral(),
                num_vars,
                VarMap::identity(num_vars),
            )
        },
        learnt_clauses_reduced_dimacs: Vec::new(),
        stages,
        // A refutation is counted 0 before and after, so there is nothing to
        // lift and no stage that earned any of it.
        count_lift: CountLift::default(),
        telemetry,
        decision_trace,
        arjun_input: None,
        independent_support_reduced: None,
    }
}

/// The stage configuration for an export preprocessing run: the shared
/// [`SimplifyConfig::for_purpose`] base (which owns the sound stage ceiling)
/// plus [`RunConfig::simplify`](crate::config::RunConfig::simplify), which may
/// reduce work inside that ceiling but cannot enable a stage the contract bans.
///
/// `purpose` is the caller's CONTRACT, and it is the only thing that decides
/// which stages run — a chain names its contract and takes the stage list that
/// comes with it, so there is no per-mode stage arithmetic here to get wrong.
///
/// `stages.simplify == false` is expressed on that SAME base rather than by
/// bypassing the call: [`SimplifyPrefix::Disabled`](crate::preprocess::simplify::SimplifyPrefix::Disabled)
/// suppresses preprocessing and `keep_all_vars = true` suppresses every
/// variable-eliminating tail stage. `simplify()` then returns an identity
/// `SimplifiedFormula` — one code path, one set of defaults.
///
/// The only per-call difference is that `WeightedCount` FREEZES every
/// unequal-weight variable out of DVE, so that every elimination DVE does make
/// is one a scalar can pay for.
pub(super) fn preprocess_config(
    config: &RunConfig,
    purpose: SimplifyPurpose,
    orig_w: &Weights<Original>,
) -> SimplifyConfig {
    if !config.stages.simplify {
        return SimplifyConfig {
            prefix: crate::preprocess::simplify::SimplifyPrefix::Disabled,
            deadline: config.deadline,
            clock: config.preprocess_clock,
            ..SimplifyConfig::for_purpose(purpose, /*keep_all_vars=*/ true)
        };
    }
    let mut resolved = SimplifyConfig {
        prefix: match config.simplify.backbone_budget_ms {
            Some(budget_ms) => crate::preprocess::simplify::SimplifyPrefix::Backbone {
                budget_ms,
                equivalence_budget_ms: config.simplify.equivalence_budget_ms,
            },
            None => crate::preprocess::simplify::SimplifyPrefix::EqIter,
        },
        deadline: config.deadline,
        clock: config.preprocess_clock,
        frozen_vars: if purpose == SimplifyPurpose::WeightedCount {
            orig_w.unequal_vars()
        } else {
            rustc_hash::FxHashSet::default()
        },
        ..SimplifyConfig::for_purpose(purpose, /*keep_all_vars=*/ false)
    };
    // The purpose's stage set is the soundness ceiling. Public policy can turn
    // count-only work down or off, never turn it on for `Function`.
    if resolved.stages.gates {
        resolved.stages.gates = config.simplify.detect_gates;
    }
    if resolved.stages.dve.is_some() {
        resolved.stages.dve = config.simplify.dve.map(|dve| DveBudget {
            rounds: dve.rounds,
            budget_ms: dve.budget_ms,
        });
    }
    resolved
}

/// The weight table the file declares, when the mode counts under one:
/// `None` for an unweighted mode, whose count ignores any table the file
/// happens to carry, and for a weighted mode over a file that declares none.
///
/// The one place that pairing is decided. What a chain then does with the
/// table differs on purpose: a count needs a weight for every literal
/// ([`original_weights`]), while Arjun's projected entry point is told only
/// about the ones the file wrote down.
pub(super) fn weight_table(meta: &CnfMeta, mode: Mode) -> Option<&WeightTable> {
    mode.is_weighted()
        .then(|| meta.declared_weights())
        .flatten()
}

/// The instance's literal weights over `num_vars`, unspecified literals
/// defaulting to 1 (the MCC convention); all-ones for an unweighted mode, where
/// every weighted formula in this crate degenerates to its integer counterpart.
pub(super) fn original_weights(meta: &CnfMeta, num_vars: usize, mode: Mode) -> Weights<Original> {
    weight_table(meta, mode).map_or_else(|| Weights::uniform(num_vars), |t| t.resolve(num_vars))
}

/// `value` as the pretty JSON a bundle's own `.json` files are written in.
///
/// Serialization of these types cannot fail: every field is a plain owned value
/// with a derived or hand-written impl that only ever writes, and the writer is
/// a `String`. So there is nothing here for a caller to handle, and no
/// half-written file to explain — the panic would be a bug in this crate's own
/// types.
pub(super) fn to_json_pretty<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).expect("bundle serialization is infallible")
}
