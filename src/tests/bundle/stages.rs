//! What the stage toggles and the mode decide about a bundle, asked of the
//! public entry point rather than of the chain that answers.

use super::*;

use crate::preprocess::VarMap;
use crate::tests::common::IRREDUCIBLE_5;

/// Refuted by unit propagation, and carrying a show set so a projected mode can
/// be asked of the same file every other mode is.
const REFUTED_WITH_A_SHOW_SET: &str = "p cnf 2 2\nc p show 1 2 0\n1 0\n-1 0\n";

/// The total map over the original variables is what makes `compile`
/// reconstructible, and its presence is a property of the MODE, not of how the
/// run turned out — so a refutation carries it under `compile` and under no
/// other mode, exactly as a satisfiable reduction does.
#[test]
fn only_the_compile_mode_gets_a_total_map_in_a_refutation_bundle() {
    let (formula, meta) = parse(REFUTED_WITH_A_SHOW_SET);
    for mode in [Mode::Mc, Mode::Wmc, Mode::Pmc, Mode::Pwmc, Mode::Compile] {
        let config = RunConfig {
            mode: Some(mode),
            ..RunConfig::default()
        };
        let bundle = preprocess(&formula, &meta, &config)
            .unwrap_or_else(|e| panic!("mode {} must preprocess: {e}", mode.token()));
        assert!(
            bundle.record.unsat,
            "mode {} must record the refutation",
            mode.token(),
        );
        assert_eq!(
            bundle.record.original_to_reduced_dimacs.is_some(),
            mode == Mode::Compile,
            "the total map belongs to mode {} alone, asked under mode {}",
            Mode::Compile.token(),
            mode.token(),
        );
    }
}

/// Turning the simplify chain off does not bypass the call — it configures the
/// one path so that no stage may drop a variable. The observable form of that
/// is a bundle whose formula is the input, under the identity map, with nothing
/// to lift.
#[test]
fn a_disabled_simplify_chain_is_the_config_where_no_stage_may_drop_a_variable() {
    let (formula, meta) = parse(IRREDUCIBLE_5);
    let config = RunConfig {
        mode: Some(Mode::Mc),
        stages: crate::config::PreprocessStages {
            simplify: false,
            arjun: false,
        },
        ..RunConfig::default()
    };
    let bundle = preprocess(&formula, &meta, &config).expect("a keep-everything run must succeed");
    assert_eq!(
        bundle.reduced, formula,
        "no stage may drop a variable, so the formula is the one handed in",
    );
    assert_eq!(
        bundle.record.reduced_to_original_dimacs,
        VarMap::identity(formula.num_vars),
        "every reduced variable is still its own original one",
    );
    assert_eq!(bundle.record.lift(), "2^0", "there is nothing to lift back");
    assert!(bundle.record.forced_literals_original_dimacs.is_empty());
    assert!(bundle.record.free_vars_original_dimacs.is_empty());
}
