//! `--mode compile`: the function-preserving chain.
//!
//! The strictest of the three: not the count but the FUNCTION has to
//! survive, so the record must be enough to reconstruct the original
//! formula's truth value on every original assignment.

use super::*;

use crate::cnf::{Original, Reduced, ShowSet, Weights};

/// `compile`: preprocess only as far as the record can undo.
///
/// The contract is stronger than every counting mode's. `reduced.cnf` plus
/// `preprocess.json` must reconstruct the original Boolean function over the
/// ORIGINAL variables exactly, so a stage may run only if its effect is written
/// down: forced-literal propagation, equivalent-literal substitution, the
/// equivalence REDUCTION that then drops each class's partners, and
/// free-variable removal. `original_to_reduced_dimacs` is what writes all four
/// down — one entry per original variable, naming the reduced literal it equals,
/// the constant it was fixed to, or nothing at all. Reconstruction is one lookup
/// per original variable in that map.
///
/// The lift is the PLAIN model count's, over all original variables. Declared
/// weights are carried through to `reduced.cnf` rather than folded into
/// `weight_lift`, so a weighted count over the reduced file needs the
/// reconstruction first, not the lift.
pub(super) fn compile_preserving_bundle(
    formula: &CnfFormula,
    meta: &CnfMeta,
    config: &RunConfig,
) -> PreprocessBundle {
    let orig_nv = formula.num_vars as usize;
    let mode = Mode::Compile;
    // `SimplifyPurpose::Function` IS this chain's contract, and its stage list
    // is what keeps gate detection and DVE out of the chain: each removes a
    // variable determined by a FUNCTION of the survivors, and an
    // `original_to_reduced_dimacs` entry names a literal, not a function, so the
    // removed variable's value could not be recovered from a model of
    // `reduced.cnf`. Arjun is excluded for the same reason and additionally
    // eliminates on the strength of an independent support, as are BVE and SBVA,
    // which change the models.
    //
    // The weights argument is unread under this contract — `frozen_vars` is a
    // `WeightedCount` concern — so there is no weight table to build here.
    let mut simplified = simplify(
        formula,
        &preprocess_config(config, SimplifyPurpose::Function, &Weights::empty()),
    );

    // No Arjun entry: this chain has no Arjun stage, so the field stays absent
    // rather than reporting a stage that was never in the chain.
    let stages = StageReport {
        simplify: Some(super::stage::simplify_outcome(config)),
        ..StageReport::default()
    };
    if let Some(bundle) = refuted(
        &simplified.reduced_formula().clauses,
        formula.num_vars,
        mode,
        None,
        stages.clone(),
    ) {
        return bundle;
    }
    if simplified.reduced_formula().num_vars == 0 {
        simplified.promote_all_backbone_to_live();
    }

    let reduced = simplified.reduced_formula().clone();
    let reduced_to_original_dimacs: VarMap<Reduced, Original> = simplified.composed_var_map();
    // The total map, and the reason this chain may drop an equivalence partner:
    // the partner is one signed literal of the reduced variable its
    // representative became, which is an entry here and cannot be one above.
    let fates = simplified.original_fates();

    let (forced_literals_original_dimacs, free_vars_original_dimacs) =
        simplified.stripped_forced_and_free();

    // Pass-through, keyed on what the FILE declares rather than on the mode:
    // `compile` is neither projected nor weighted, so `is_projected()` /
    // `is_weighted()` would drop both declarations. Renumbered into the reduced
    // space through the shared carry and otherwise untouched.
    let show_vars_reduced_dimacs = meta.declared_show_vars().map(|show| {
        // A reduced variable is a show variable iff ANY original that resolves
        // to it was declared one. Restricting to the original a reduced variable
        // *stands for* would silently drop a declared equivalence partner, whose
        // value is read off its representative and so must be projected there.
        let mut class_declared = vec![false; orig_nv];
        for (original, fate) in fates.iter().enumerate() {
            if let OriginalFate::Variable { index, .. } = *fate
                && show.contains(VarId(original as u32))
            {
                class_declared[simplified.reduced_var_to_original(index)] = true;
            }
        }
        let widened = ShowSet::<Original>::from_zero_based(
            (0..orig_nv as u32).filter(|v| class_declared[*v as usize]),
        );
        reduced_to_original_dimacs.carry_show(&widened)
    });
    // Renumbered through the same shared carry the show set uses, which is also
    // what swaps `(w⁻, w⁺)` for a reduced variable standing for an original's
    // NEGATION.
    let reduced_weights = meta.declared_weights().map(|t| {
        reduced_to_original_dimacs
            .carry_weights(&t.resolve(orig_nv))
            .to_record_rows()
    });

    // `compile` folds no weight into the lift: the input's own weights are
    // renumbered onto the reduced formula and travel with the record.
    let record = PreprocessRecord {
        original_to_reduced_dimacs: Some(OriginalMap::from_fates(&fates)),
        forced_literals_original_dimacs,
        free_vars_original_dimacs,
        show_vars_reduced_dimacs,
        reduced_weights,
        ..PreprocessRecord::new(
            mode,
            RecordLift::Pow2(simplified.count_lift_pow2(0)),
            formula.num_vars,
            reduced_to_original_dimacs,
        )
    };
    PreprocessBundle {
        reduced,
        record,
        learnt_clauses_reduced_dimacs: Vec::new(),
        stages,
        count_lift: CountLift {
            simplify_pow2: simplified.count_lift_pow2(0),
            arjun_pow2: 0,
        },
        arjun_input: None,
        independent_support_reduced: None,
    }
}
