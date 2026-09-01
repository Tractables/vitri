//! Compile mode: the reduced CNF plus the record must reconstruct the
//! ORIGINAL function over the ORIGINAL variables, exactly.
//!
//! Checked by exhaustive enumeration rather than by counting — a count
//! identity would pass on a reconstruction that got the function wrong in
//! two compensating places.

use super::*;

use crate::decompose::SelectionCtx;

fn compile_run(dimacs: &str) -> VitriRun {
    let (formula, meta) = parse(dimacs);
    run(
        &formula,
        &meta,
        &RunConfig {
            mode: Some(Mode::Compile),
            vtree_spec: "minfill-primal".to_string(),
            ..RunConfig::default()
        },
        &SelectionCtx::plain(),
    )
    .expect("the public compile frontend must run")
}

/// Compile mode can represent a function settled entirely by constants in its
/// total original map. Keeping one artificial unit variable would contradict
/// both that map and the whole frontend's fully-resolved outcome.
#[test]
fn compile_all_backbones_is_fully_resolved_with_a_total_constant_map() {
    let produced = compile_run("p cnf 3 3\n1 0\n-2 0\n3 0\n");

    assert_eq!(produced.preprocessed.reduced.num_vars, 0);
    assert!(produced.preprocessed.reduced.clauses.is_empty());
    let total = produced
        .preprocessed
        .record
        .original_to_reduced_dimacs
        .as_ref()
        .expect("compile mode must return its total original map");
    assert_eq!(
        total.iter().collect::<Vec<_>>(),
        vec![
            OriginalTarget::Constant(true),
            OriginalTarget::Constant(false),
            OriginalTarget::Constant(true),
        ],
    );
    assert!(matches!(produced.vtree, RunVtree::FullyResolved));
}

/// A refutation is a function too — the constant false — and `compile` states
/// it the same way every other mode does: the record carries the verdict and
/// the total map, and no vtree is built, because a compiler handed
/// `count(original) = 0` has nothing left to compile.
#[test]
fn compile_a_refutation_is_reported_without_a_vtree() {
    let produced = compile_run("p cnf 2 2\n1 0\n-1 0\n");

    assert!(produced.preprocessed.record.unsat);
    assert!(
        produced
            .preprocessed
            .record
            .original_to_reduced_dimacs
            .is_some(),
        "compile mode owes its total original map whatever the outcome",
    );
    assert!(matches!(produced.vtree, RunVtree::Refuted));
    assert!(produced.built().is_none());
}

/// An all-free input takes no simplification path at all: its original
/// variables stay live and the total map remains the identity. Removing the
/// all-backbone promotion must not turn that existing reconstruction into a
/// different zero-variable representation.
#[test]
fn compile_all_free_keeps_its_identity_reconstruction() {
    let produced = compile_run("p cnf 3 0\n");

    assert_eq!(produced.preprocessed.reduced.num_vars, 3);
    let total = produced
        .preprocessed
        .record
        .original_to_reduced_dimacs
        .as_ref()
        .expect("compile mode must return its total original map");
    assert_eq!(
        total.iter().collect::<Vec<_>>(),
        vec![
            OriginalTarget::Literal(1),
            OriginalTarget::Literal(2),
            OriginalTarget::Literal(3),
        ],
    );
    assert_eq!(produced.preprocessed.record.count_lift_pow2, 0);
    assert!(matches!(produced.vtree, RunVtree::Built(_)));
}

/// Forced literals, an equivalence (with a polarity flip) and a free variable in
/// one instance — every stage `compile` permits, all of which the reconstruction
/// has to undo.
#[test]
fn compile_reconstructs_the_function() {
    let rt = round_trip_with(
        "compile",
        "p cnf 6 6\n\
         1 0\n\
         -2 -3 0\n\
         2 3 0\n\
         -1 2 4 0\n\
         -4 3 0\n\
         4 5 0\n",
        &RunConfig {
            mode: Some(Mode::Compile),
            ..Default::default()
        },
    );
    rt.assert_sound();
    assert_eq!(rt.record.mode, Mode::Compile);
    assert_function_reconstructs(&rt);
    assert!(
        equivalence_fired(&rt),
        "`2 ≡ ¬3` must be reduced away, or the reconstruction proves nothing; record = {}",
        rt.record.to_json_string(),
    );
    // The class is an ANTI-equivalence, so the partner's entry must carry the
    // sign. Dropping it would leave the model count right and every lifted model
    // wrong, which is why the map is signed at all.
    let map = rt
        .record
        .original_to_reduced_dimacs
        .as_ref()
        .expect("total map");
    assert!(
        map.iter()
            .any(|t| matches!(t, OriginalTarget::Literal(l) if l < 0)),
        "an anti-equivalent partner must be named by a negative literal; record = {}",
        rt.record.to_json_string(),
    );
}

#[test]
fn compile_still_reduces() {
    let rt = round_trip_with(
        "compile-reduces",
        "p cnf 5 5\n\
         1 0\n\
         -2 3 0\n\
         2 -3 0\n\
         -1 2 0\n\
         3 4 0\n",
        &RunConfig {
            mode: Some(Mode::Compile),
            ..Default::default()
        },
    );
    rt.assert_sound();
    assert_function_reconstructs(&rt);
    rt.assert_reduced_below(5);
}

/// The equivalence reduction on its own, with no backbone to absorb it: `1 ≡ 2`
/// is the instance's ONLY reduction opportunity, so the partner is dropped
/// because that stage dropped it. Both members must come back naming the one
/// reduced variable the class left behind.
#[test]
fn compile_drops_an_equivalence_partner() {
    let rt = round_trip_with(
        "compile-equivalence",
        "p cnf 4 4\n\
         1 -2 0\n\
         -1 2 0\n\
         1 3 4 0\n\
         -1 -3 -4 0\n",
        &RunConfig {
            mode: Some(Mode::Compile),
            ..Default::default()
        },
    );
    rt.assert_sound();
    assert_function_reconstructs(&rt);
    assert!(
        equivalence_fired(&rt),
        "`1 ≡ 2` must be reduced away; record = {}",
        rt.record.to_json_string(),
    );
    let map = rt
        .record
        .original_to_reduced_dimacs
        .as_ref()
        .expect("total map");
    assert_eq!(
        map.get(VarId(0)),
        map.get(VarId(1)),
        "both members of the class must name the same reduced literal; record = {}",
        rt.record.to_json_string(),
    );
    assert!(
        rt.record.forced_literals_original_dimacs.is_empty(),
        "no backbone here — the drop is the equivalence reduction's; record = {}",
        rt.record.to_json_string(),
    );
    rt.assert_reduced_below(4);
}

/// `compile` renumbers the declared weights onto the reduced variables and
/// counts under none of them, so the table it writes must be the declared one
/// read through the map beside it — variable for variable, exactly. The
/// equivalence reduction drops a partner here, so the two numberings genuinely
/// differ; a carry that lost the renumbering would still write a
/// well-formed-looking table of the right length.
#[test]
fn compile_renumbers_the_declared_weights() {
    // `compile` carries declared weights without counting under them, so the
    // harness's own table is the neutral one — read the declarations back.
    const CNF: &str = "c t wmc\n\
                       p cnf 4 4\n\
                       c p weight 1 1/3 0\n\
                       c p weight -1 2/5 0\n\
                       c p weight 2 4/7 0\n\
                       c p weight -2 6/11 0\n\
                       -1 -2 0\n\
                       1 2 0\n\
                       1 3 4 0\n\
                       -1 -3 -4 0\n";
    let rt = round_trip_with(
        "compile-weight-renumber",
        CNF,
        &RunConfig {
            mode: Some(Mode::Compile),
            ..Default::default()
        },
    );
    rt.assert_sound();
    assert_function_reconstructs(&rt);
    assert!(
        equivalence_fired(&rt),
        "`2 ≡ ¬1` must be reduced away, or the two numberings coincide and the \
         carry proves nothing; record = {}",
        rt.record.to_json_string(),
    );
    let declared: Weights<Original> = parse(CNF)
        .1
        .declared_weights()
        .expect("the fixture declares weights")
        .resolve(rt.original.num_vars as usize);
    let reduced_w = rt.reduced_weights();
    for (r, entry) in rt.record.reduced_to_original_dimacs.iter().enumerate() {
        let o = entry.expect("compile names an original for every reduced variable");
        let (wn, wp) = &declared[VarId(o.unsigned_abs() - 1)];
        let expected = if o > 0 {
            (wn.clone(), wp.clone())
        } else {
            (wp.clone(), wn.clone())
        };
        assert_eq!(
            reduced_w[VarId(r as u32)],
            expected,
            "reduced variable {} stands for original literal {o}; record = {}",
            r + 1,
            rt.record.to_json_string(),
        );
    }
}

#[test]
fn compile_passes_declarations_through() {
    let rt = round_trip_with(
        "compile-passthrough",
        "c t pwmc\n\
         p cnf 4 4\n\
         c p show 1 3 0\n\
         c p weight 1 1/3 0\n\
         c p weight -1 2/3 0\n\
         1 2 0\n\
         -1 3 0\n\
         -2 -3 4 0\n\
         2 3 -4 0\n",
        &RunConfig {
            mode: Some(Mode::Compile),
            ..Default::default()
        },
    );
    rt.assert_sound();
    assert_eq!(rt.record.mode, Mode::Compile);
    assert_function_reconstructs(&rt);
    assert!(
        rt.record.show_vars_reduced_dimacs.is_some(),
        "the declared show set must survive"
    );
    assert!(
        rt.record.reduced_weights.is_some(),
        "the declared weights must survive"
    );
    assert!(rt.reduced_cnf_text.contains("c p show"));
    assert!(rt.reduced_cnf_text.contains("c p weight"));
    assert_eq!(
        rt.record.weight_lift, "1/1",
        "compile carries weights rather than folding them into the lift",
    );
}

/// **The compile lift is exactly the unused variables.** Every other chain can
/// put more into the exponent — a counting chain pays for gate detection, DVE,
/// Arjun, BVE and SBVA there — but `compile` runs none of those, so the only
/// variables it strips without a map entry are the ones no clause mentions, and
/// `count_lift_pow2` must equal `free_vars_original_dimacs.len()` on the nose.
/// The general gate elsewhere is `>=`; here it is `==`, and that is the whole
/// point.
///
/// The instance is chosen so neither side is trivially zero: variables 5 and 6
/// are declared and never used, and `1 ≡ 2` gives the equivalence reduction a
/// variable to remove that must NOT reach the exponent.
#[test]
fn compile_lift_counts_exactly_the_free_variables() {
    let rt = round_trip_with(
        "compile-free-lift",
        "p cnf 6 4\n\
         1 -2 0\n\
         -1 2 0\n\
         1 3 4 0\n\
         -1 -3 -4 0\n",
        &RunConfig {
            mode: Some(Mode::Compile),
            ..Default::default()
        },
    );
    rt.assert_sound();
    assert_function_reconstructs(&rt);
    assert_eq!(
        rt.record.free_vars_original_dimacs,
        vec![5, 6],
        "variables 5 and 6 occur in no clause; record = {}",
        rt.record.to_json_string(),
    );
    assert!(
        equivalence_fired(&rt),
        "`1 ≡ 2` must be reduced away, so a removed non-free variable is in play; record = {}",
        rt.record.to_json_string(),
    );
    assert_eq!(
        rt.record.count_lift_pow2 as usize,
        rt.record.free_vars_original_dimacs.len(),
        "under compile the exponent counts the unused variables and nothing else; record = {}",
        rt.record.to_json_string(),
    );
}

/// A declared show variable can be the member of an equivalence class that
/// `compile`'s simplify chain DROPS. Its value is read back off the survivor, so
/// the declaration has to land on the survivor's reduced id — a carry that
/// walked only the reduced-indexed map would find no entry naming the dropped
/// partner and emit a `c p show` line one variable short, which is a silently
/// wrong projection rather than a failure.
///
/// The fixture declares ONLY the partner, so the class's other member cannot
/// mask the bug by putting the right id there for the wrong reason.
#[test]
fn compile_carries_a_declared_equivalence_partner_into_the_representative() {
    let rt = round_trip_with(
        "compile-equiv-partner",
        "p cnf 4 4\n\
         c p show 2 0\n\
         1 -2 0\n\
         -1 2 0\n\
         1 3 4 0\n\
         -1 -3 -4 0\n",
        &RunConfig {
            mode: Some(Mode::Compile),
            ..Default::default()
        },
    );
    rt.assert_sound();
    assert_function_reconstructs(&rt);
    assert!(
        equivalence_fired(&rt),
        "`1 ≡ 2` must be reduced away, or nothing is being carried across a class; record = {}",
        rt.record.to_json_string(),
    );

    // Where the class landed, read off the map rather than assumed: both
    // originals must resolve to the same reduced variable, and that variable is
    // what the show set has to name.
    let total = rt
        .record
        .original_to_reduced_dimacs
        .as_ref()
        .expect("compile records the total map");
    let reduced_var = |original: usize| match total.get(VarId(original as u32)) {
        Some(OriginalTarget::Literal(l)) => l.unsigned_abs(),
        other => panic!(
            "original {} resolves to {other:?}, not a reduced literal",
            original + 1
        ),
    };
    let representative = reduced_var(0);
    assert_eq!(
        representative,
        reduced_var(1),
        "the fixture must merge the pair, or this is not the partner case; record = {}",
        rt.record.to_json_string(),
    );

    assert_eq!(
        rt.record
            .show_vars_reduced_dimacs
            .as_ref()
            .map(|s| s.to_dimacs()),
        Some(vec![representative]),
        "the declared partner must be projected onto its class representative; record = {}",
        rt.record.to_json_string(),
    );
}
