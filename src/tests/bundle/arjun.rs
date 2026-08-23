use super::*;
use crate::error::VitriError;
use crate::tests::learnt_clauses::assert_learnts_are_implied;

/// A Tseitin-defined instance: `v4 ⇔ (v1 ∧ v2)` and `v5 ⇔ (v3 ∨ v4)`, with a
/// constraint over the inputs. `v4`/`v5` are *determined* by `v1..v3`, which is
/// exactly the structure Arjun's variable elimination removes — and removing a
/// determined variable must contribute nothing to the `2^k` lift. If the
/// composition mistakenly gave it a factor of 2, `assert_lift_exact` reports a
/// doubled count.
#[test]
fn round_trip_arjun_definitions() {
    let rt = round_trip(
        "arjun-defs",
        "p cnf 5 8\n\
         -4 1 0\n\
         -4 2 0\n\
         4 -1 -2 0\n\
         5 -3 0\n\
         5 -4 0\n\
         -5 3 4 0\n\
         1 3 0\n\
         -1 -3 5 0\n",
    );
    rt.assert_sound();
}

/// The same definitional structure under WEIGHTS. A defined variable contributes
/// a model-dependent factor unless its two literal weights agree, so this is the
/// case where the weighted chain must either pay `×w` exactly or refuse the DVE
/// stage and say so — never split the difference.
#[test]
fn round_trip_weighted_definitions() {
    let rt = round_trip(
        "wmc-defs",
        "c t wmc\n\
         p cnf 5 8\n\
         c p weight 1 1/3 0\n\
         c p weight -1 2/3 0\n\
         c p weight 4 5/7 0\n\
         c p weight -4 5/7 0\n\
         c p weight 5 3/4 0\n\
         c p weight -5 1/4 0\n\
         -4 1 0\n\
         -4 2 0\n\
         4 -1 -2 0\n\
         5 -3 0\n\
         5 -4 0\n\
         -5 3 4 0\n\
         1 3 0\n\
         -1 -3 5 0\n",
    );
    rt.assert_sound();
}

/// A backbone + definitions + free-variable instance, so stage 1 and the Arjun
/// stage both have work to do and their two lift contributions have to compose
/// into one exponent (checked by `assert_lift_exact` inside `assert_sound`).
/// `v7` occurs nowhere.
#[test]
fn round_trip_arjun_composes_with_stage1() {
    let rt = round_trip(
        "arjun-compose",
        "p cnf 7 9\n\
         1 0\n\
         -5 2 0\n\
         -5 3 0\n\
         5 -2 -3 0\n\
         -6 4 0\n\
         6 -4 0\n\
         2 3 4 0\n\
         -5 -6 0\n\
         -1 5 6 0\n",
    );
    rt.assert_sound();
}

/// **The case that actually exercises the variable map.** Arjun solves a small,
/// sparse instance outright — the reduced formula comes back with no variables
/// and an all-`null` map, which is correct but tests nothing about the
/// correspondence. This instance is dense enough (16 vars, 62 three-literal
/// clauses, 27 models) that variable elimination stalls and a real residual
/// survives, so `reduced_to_original_dimacs` names actual variables and
/// `assert_models_lift_back` has something to lift.
#[test]
fn round_trip_arjun_leaves_a_mapped_residual() {
    let rt = round_trip(
        "arjun-residual",
        "p cnf 16 62\n\
         3 7 11 0\n1 10 12 0\n-2 7 14 0\n2 10 14 0\n-4 1 13 0\n-3 9 14 0\n\
         -10 4 15 0\n-10 -1 3 0\n-11 -10 -8 0\n-3 8 13 0\n-16 -15 6 0\n-9 4 7 0\n\
         -7 -1 16 0\n-12 8 10 0\n-12 8 9 0\n-15 -5 12 0\n-6 3 15 0\n-5 7 13 0\n\
         -13 14 15 0\n-15 -9 7 0\n-12 -9 7 0\n2 3 5 0\n-14 -8 1 0\n-3 -1 7 0\n\
         -14 -8 -2 0\n-13 2 8 0\n-8 3 4 0\n-1 2 16 0\n-2 1 14 0\n-9 -6 10 0\n\
         -14 -8 -4 0\n-3 -2 10 0\n12 14 16 0\n-3 12 16 0\n-16 6 9 0\n-4 11 16 0\n\
         -8 -4 9 0\n-13 -1 5 0\n-12 -8 13 0\n-8 -4 2 0\n-10 -8 7 0\n-14 3 11 0\n\
         -16 -15 3 0\n7 8 13 0\n-5 -3 1 0\n3 9 12 0\n-9 4 12 0\n-1 5 7 0\n\
         -11 5 9 0\n-12 8 15 0\n1 8 14 0\n-8 3 6 0\n13 15 16 0\n-5 1 7 0\n\
         -8 -6 3 0\n-16 -4 9 0\n-7 -5 -2 0\n-3 4 11 0\n-13 2 10 0\n-9 3 15 0\n\
         8 13 15 0\n-14 -9 7 0\n",
    );
    eprintln!(
        "[test] arjun residual: {} -> {} vars, lift 2^{} ({} named free), map {:?}",
        rt.record.original_num_vars,
        rt.reparsed.num_vars,
        rt.record.count_lift_pow2,
        rt.record.free_vars_original_dimacs.len(),
        rt.record.reduced_to_original_dimacs,
    );
    rt.assert_sound();
    assert!(
        rt.record
            .reduced_to_original_dimacs
            .iter()
            .any(|e| e.is_some()),
        "this instance must leave a residual whose variables the map names — \
         otherwise the map assertions above are vacuous",
    );
}

/// An instance Arjun can resolve outright is the case where the reduced formula
/// has no variables left and the whole answer is the lift. Whatever the stage
/// does with it, the identity must still hold — this is the shape the CLI
/// short-circuits on, so it must not be the one shape that is untested.
#[test]
fn round_trip_arjun_fully_determined() {
    let rt = round_trip(
        "arjun-determined",
        "p cnf 4 6\n\
         1 0\n\
         -1 2 0\n\
         -2 3 0\n\
         -3 4 0\n\
         -4 1 0\n\
         2 3 0\n",
    );
    rt.assert_sound();
}

/// The whole learnt-clause contract, in the one place a consumer meets it: the
/// clauses come back on the bundle, in the numbering of the formula that is
/// exported beside them, and they say nothing the exported formula did not
/// already say.
///
/// The count is what proves BOTH halves at once. A clause preprocessing implies
/// removes no model, so conjoining the harvest cannot change the reduced count —
/// while a clause left in Arjun's own internal numbering would constrain the
/// wrong variables and, on an instance this dense, drop models. The range
/// assertion under it says which of the two failed when one does.
#[test]
fn arjun_learnt_harvest_is_implied_by_the_exported_formula() {
    let (formula, meta) = parse(LEARNT_FIXTURE_12);
    let config = RunConfig {
        export_learned_clauses: true,
        ..RunConfig::default()
    };
    let bundle = crate::bundle::preprocess(&formula, &meta, &config).expect("preprocess");

    // Guard against a vacuous pass: this fixture leaves a residual Arjun's
    // oracle records clauses on, so an empty harvest is a broken one.
    assert!(
        !bundle.learnt_clauses_reduced_dimacs.is_empty(),
        "expected a non-empty harvest — without one the assertions below prove nothing",
    );
    for cl in &bundle.learnt_clauses_reduced_dimacs {
        assert!(!cl.is_empty(), "the empty clause is not a learnt clause");
        for &l in cl {
            assert!(
                l != 0 && l.unsigned_abs() <= bundle.reduced.num_vars,
                "literal {l} names no variable of the exported formula ({} vars) — the \
                 harvest is in the wrong variable space",
                bundle.reduced.num_vars,
            );
        }
    }

    assert_learnts_are_implied(&bundle.reduced, &bundle.learnt_clauses_reduced_dimacs);
}

/// The default is no harvest and no cost: the same instance under
/// `RunConfig::default()` preprocesses to the same formula and carries no clauses.
#[test]
fn arjun_learnt_harvest_is_off_by_default() {
    let (formula, meta) = parse(LEARNT_FIXTURE_12);
    let bundle =
        crate::bundle::preprocess(&formula, &meta, &RunConfig::default()).expect("preprocess");
    assert!(bundle.learnt_clauses_reduced_dimacs.is_empty());
}

/// Asking for the harvest where nothing can produce it is refused rather than
/// answered with an empty list, which would be indistinguishable from a run
/// where Arjun derived nothing. Each message names the request and the stage it
/// needs.
#[test]
fn arjun_learnt_harvest_is_refused_where_nothing_could_produce_it() {
    let (formula, meta) = parse(LEARNT_FIXTURE_12);
    let asked = RunConfig {
        export_learned_clauses: true,
        ..RunConfig::default()
    };

    let no_arjun = RunConfig {
        stages: crate::config::PreprocessStages {
            arjun: false,
            ..asked.stages
        },
        ..asked.clone()
    };
    let e = crate::bundle::preprocess(&formula, &meta, &no_arjun)
        .expect_err("the Arjun stage is off — there is nothing to harvest from");
    let VitriError::Config { reason } = &e else {
        panic!("an inert request is something the caller configures, not {e:?}");
    };
    assert!(
        reason.contains("VITRI_ARJUN_EXPORT_LEARNED_CLAUSES") && reason.contains("--no-arjun"),
        "the message must name the request and the stage it needs: {reason}",
    );

    // Arjun runs under `wmc` and does not run at all under `compile`; neither
    // preprocessing has the harvesting stage, so both refuse.
    for mode in [Mode::Wmc, Mode::Compile] {
        let other = RunConfig {
            mode: Some(mode),
            ..asked.clone()
        };
        let e = crate::bundle::preprocess(&formula, &meta, &other)
            .err()
            .unwrap_or_else(|| panic!("mode {} must refuse the request", mode.token()));
        let VitriError::Config { reason } = &e else {
            panic!("an inert request is something the caller configures, not {e:?}");
        };
        assert!(
            reason.contains("VITRI_ARJUN_EXPORT_LEARNED_CLAUSES")
                && reason.contains(mode.token())
                && reason.contains(Mode::Mc.token()),
            "the message must name the request, the mode asked for, and the one that \
             harvests: {reason}",
        );
    }
}

const LEARNT_FIXTURE_12: &str = "p cnf 12 30\n\
     1 2 3 0\n-1 -2 4 0\n2 -3 5 0\n-4 5 6 0\n1 -5 -6 0\n3 4 -6 0\n\
     7 8 -1 0\n-7 9 2 0\n8 -9 10 0\n-8 -10 11 0\n9 10 -12 0\n-11 12 1 0\n\
     4 7 -10 0\n-3 -8 11 0\n5 -9 12 0\n6 -7 -11 0\n-2 8 12 0\n1 -4 9 0\n\
     2 5 -7 0\n-6 10 -12 0\n3 -5 8 0\n-1 7 11 0\n4 -8 -9 0\n-3 6 10 0\n\
     2 -4 -11 0\n5 9 -12 0\n-1 -6 8 0\n3 7 -10 0\n-2 -5 11 0\n1 6 -9 0\n";
