use crate::bundle::PreprocessPhase;
use crate::cnf::CnfFormula;
use crate::cnf::Literal;
use crate::config::PreprocessClock;
use crate::preprocess::meter::PreprocessMeter;
use crate::preprocess::probe_engine::*;
use crate::preprocess::tests::wall_meter;
use crate::tests::common::{clause, clause_dimacs, pigeonhole};
use std::collections::HashSet;
use std::time::Duration;

const TEST_BUDGET: Duration = Duration::from_secs(10);

/// x1 forced true: (x1∨x2) ∧ (x1∨¬x2). x3 ≡ x4: (¬x3∨x4) ∧ (x3∨¬x4), anchored
/// by (x3∨x5) to stay SAT. Variable `v` is written `name(v)` in a space of
/// `num_vars` variables.
fn backbone_and_equiv_formula(num_vars: u32, name: impl Fn(u32) -> u32) -> CnfFormula {
    let c = |lits: &[(u32, bool)]| {
        let renamed: Vec<(u32, bool)> = lits.iter().map(|&(v, p)| (name(v), p)).collect();
        clause(&renamed)
    };
    CnfFormula::from_parts(
        num_vars,
        vec![
            c(&[(1, true), (2, true)]),
            c(&[(1, true), (2, false)]),
            c(&[(3, false), (4, true)]),
            c(&[(3, true), (4, false)]),
            c(&[(3, true), (5, true)]),
        ],
    )
}

/// `(variable, polarity)` of each literal, its variable renamed by `name`.
fn renamed(lits: impl IntoIterator<Item = Literal>, name: impl Fn(u32) -> u32) -> Vec<(u32, bool)> {
    lits.into_iter()
        .map(|l| (name(l.var.get()), l.positive))
        .collect()
}

/// observe_model must split each class by the model's bit and keep the
/// ⊤-class's true-half as the anchor.
#[test]
fn observe_model_splits_and_tracks_top() {
    // Refinement is solver-free, so this runs on the partition alone.
    let mut p = Partition::new(4);

    // Single ⊤-class of true-literals [1,2,3,4] (dimacs).
    p.classes = vec![vec![1, 2, 3, 4]];
    p.top = 0;
    // Model: var1 true, var2 false, var3 true, var4 false → [1,-2,3,-4].
    p.observe_model(&[1, -2, 3, -4]);

    // ⊤-class true-half = literals true in the model: 1 and 3.
    assert_eq!(p.classes[p.top], vec![1, 3]);
    // The false-half [2,4] (size ≥ 2) becomes its own class.
    assert!(p.classes.iter().any(|c| c == &vec![2, 4]));
    assert_eq!(p.classes.len(), 2);

    // A second model splits the ⊤-class again; a singleton false-half is
    // dropped (can no longer yield an equivalence) but the ⊤ anchor survives.
    p.observe_model(&[1, -2, -3, 4]); // among {1,3}: 1 true, 3 false
    assert_eq!(p.classes[p.top], vec![1]);
    // No class of size ≥ 2 contains 3 alone → 3's singleton was dropped.
    assert!(p.classes.iter().all(|c| c != &vec![3]));
}

/// Golden expectations on a tiny hand-verified formula: the engine's backbone
/// pass confirms exactly its unique backbone, and its equivalence pass finds
/// its unique equivalence.
#[test]
fn engine_finds_backbone_and_equiv() {
    let f = backbone_and_equiv_formula(5, |v| v);

    let mut e = ProbeEngine::new(&f).expect("the solver allocates");
    let bb_eng = e.run_backbone_with_meter(TEST_BUDGET, &mut wall_meter());

    // Golden: x1=true is the UNIQUE backbone. x2 is free; x3≡x4 both take T
    // (models with x3=x4=T) and F (x3=x4=F, forcing x5=T via (x3∨x5)), and x5
    // takes both — so none of x2/x3/x4/x5 is backbone. Soundness guarantees
    // the engine confirms no spurious literal, so the set is exactly {x1=T}.
    let set_eng: HashSet<(u32, bool)> = bb_eng
        .forced
        .iter()
        .map(|l| (l.var.get(), l.positive))
        .collect();
    assert_eq!(
        set_eng,
        HashSet::from([(1u32, true)]),
        "engine backbone must be exactly {{x1=true}}, got {:?}",
        bb_eng.forced,
    );
    // The field is the single source of the returned `forced`.
    assert_eq!(e.partition.confirmed_backbone.len(), bb_eng.forced.len());

    // No post-backbone Tarjan mapping in this direct test → identity mapping.
    let eq_eng = e.run_equiv_with_meter(TEST_BUDGET, &None, &mut wall_meter());
    let has_34 = |v: &Vec<(Literal, Literal)>| {
        v.iter().any(|(a, b)| {
            let vars = [a.var.get(), b.var.get()];
            vars.contains(&3) && vars.contains(&4)
        })
    };
    assert!(
        has_34(&eq_eng.equivalences),
        "engine must find x3 ≡ x4, got {:?}",
        eq_eng.equivalences
    );
}

/// A confirmed backbone literal leaves the partition: it is a constant, not a
/// member of an equivalence class.
#[test]
fn a_confirmed_backbone_literal_leaves_every_class() {
    let f = backbone_and_equiv_formula(5, |v| v);
    let mut e = ProbeEngine::new(&f).expect("the solver allocates");
    e.run_backbone_with_meter(TEST_BUDGET, &mut wall_meter());

    let confirmed: Vec<i32> = e
        .partition
        .confirmed_backbone
        .iter()
        .map(|l| l.to_dimacs())
        .collect();
    assert_eq!(confirmed, vec![1]);
    assert!(
        e.partition.classes.iter().flatten().all(|&l| l.abs() != 1),
        "x1 is still in a class: {:?}",
        e.partition.classes
    );
}

/// Variables no clause mentions change only the names of what the engine
/// finds: padded with them below, between and above its own variables, the
/// formula yields the same backbone and equivalences after the same probes,
/// and every padding variable is counted as flippable.
#[test]
fn unmentioned_variables_change_only_the_names_of_what_the_engine_finds() {
    // Variable v becomes 10v - 5 in a space of 60: 55 variables go unmentioned,
    // ten of them above the last one a clause names.
    let spread = |v: u32| 10 * v - 5;
    let mut plain = ProbeEngine::new(&backbone_and_equiv_formula(5, |v| v)).expect("allocates");
    let mut padded = ProbeEngine::new(&backbone_and_equiv_formula(60, spread)).expect("allocates");

    let bb_plain = plain.run_backbone_with_meter(TEST_BUDGET, &mut wall_meter());
    let bb_padded = padded.run_backbone_with_meter(TEST_BUDGET, &mut wall_meter());
    assert_eq!(
        renamed(bb_padded.forced.clone(), |v| v),
        vec![(spread(1), true)]
    );
    assert_eq!(
        renamed(bb_padded.forced, |v| v),
        renamed(bb_plain.forced, spread)
    );
    assert_eq!(bb_padded.probes_completed, bb_plain.probes_completed);
    assert_eq!(bb_padded.fixed_found, bb_plain.fixed_found);
    assert_eq!(bb_padded.model_eliminated, bb_plain.model_eliminated);
    assert_eq!(
        bb_padded.flippable_eliminated,
        bb_plain.flippable_eliminated + 55
    );

    let eq_plain = plain.run_equiv_with_meter(TEST_BUDGET, &None, &mut wall_meter());
    let eq_padded = padded.run_equiv_with_meter(TEST_BUDGET, &None, &mut wall_meter());
    let flat = |eqs: Vec<(Literal, Literal)>| eqs.into_iter().flat_map(|(a, b)| [a, b]);
    assert_eq!(
        renamed(flat(eq_padded.equivalences), |v| v),
        renamed(flat(eq_plain.equivalences), spread)
    );
    assert_eq!(eq_padded.probes_completed, eq_plain.probes_completed);
}

/// One backbone literal that only a pigeonhole refutation proves: every
/// pigeonhole clause is written with `x1` in front, so `¬x1` leaves the
/// pigeonhole formula to refute, and `x1` holds in every model. The refutation
/// needs more conflicts than a single probe's cap and far fewer than a
/// run-to-answer probe's, so the capped probe answers unknown once and the probe
/// after it, on the same literal, confirms it.
#[test]
fn a_backbone_literal_past_the_single_probe_cap_is_confirmed() {
    let f = CnfFormula::from_parts(1 + 10 * 9, pigeonhole(1, 10, 9, &[1]));
    let mut meter = PreprocessMeter::new(PreprocessClock::Deterministic {
        configured_wall_ms: None,
    });
    let mut e = ProbeEngine::new(&f).expect("the solver allocates");
    let bb = e.run_backbone_with_meter(Duration::from_secs(3600), &mut meter);

    let trace = meter.into_trace().expect("deterministic mode traces");
    let backbone = trace
        .phases
        .iter()
        .find(|p| p.phase == PreprocessPhase::Backbone)
        .expect("the backbone phase ran");
    assert_eq!(
        (renamed(bb.forced, |v| v), backbone.probes.unknown),
        (vec![(1, true)], 1),
        "{:?}",
        backbone.probes,
    );
}

/// The probe after a capped single that stopped undecided runs to an answer,
/// whatever its size, and only that probe: the single after it is capped
/// again. A literal escalates once; stopping undecided a second time sets it
/// aside.
#[test]
fn escalation_lasts_one_probe_and_each_literal_escalates_once() {
    let mut e = Escalation::default();
    assert!(e.capped_single(1));
    assert!(e.escalate(7));
    // The escalated probe: literal 7 and the seven candidates after it.
    assert!(!e.capped_single(8));
    // Its counter-model kept 7 a candidate: the next single on it is capped,
    assert!(e.capped_single(1));
    // and stopping undecided again sets 7 aside.
    assert!(!e.escalate(7));
    // The next hard literal gets its own escalation, here with one candidate left.
    assert!(e.escalate(9));
    assert!(!e.capped_single(1));
    assert!(e.capped_single(1));
}

/// `pairs` true equivalences `a_k ≡ b_k` that only a pigeonhole refutation
/// proves: `a_k ∨ ¬b_k` is a clause of its own, and `a_k → b_k` holds only
/// because every clause of one shared pigeonhole formula is also written with
/// `¬a_k ∨ b_k` in front. `a_k` is variable `2k + 1` and `b_k` is `2k + 2`; the
/// pigeonhole variables follow. Every pair holds in every model, so the seed
/// leaves all of them in one class.
fn pairs_behind_a_pigeonhole(pairs: u32, pigeons: u32, holes: u32) -> CnfFormula {
    let mut clauses = Vec::new();
    for k in 0..pairs {
        let (a, b) = (2 * k as i32 + 1, 2 * k as i32 + 2);
        clauses.push(clause_dimacs(&[a, -b]));
        clauses.extend(pigeonhole(2 * pairs, pigeons, holes, &[-a, b]));
    }
    CnfFormula::from_parts(2 * pairs + pigeons * holes, clauses)
}

/// Each probe stops at the conflict cap, and the second one that does ends the
/// phase: three pairs need a pigeonhole refutation, and the phase answers
/// unknown twice and probes nothing after that. The deterministic clock counts
/// the outcomes; the budget is far beyond what the probes can spend.
#[test]
fn equivalence_probing_ends_at_its_second_unknown_probe() {
    let f = pairs_behind_a_pigeonhole(3, 10, 9);
    let mut meter = PreprocessMeter::new(PreprocessClock::Deterministic {
        configured_wall_ms: None,
    });
    let mut e = ProbeEngine::new(&f).expect("the solver allocates");
    // A zero backbone budget seeds the partition and probes nothing.
    e.run_backbone_with_meter(Duration::ZERO, &mut meter);
    e.run_equiv_with_meter(Duration::from_secs(3600), &None, &mut meter);

    let trace = meter.into_trace().expect("deterministic mode traces");
    let equivalence = trace
        .phases
        .iter()
        .find(|p| p.phase == PreprocessPhase::Equivalence)
        .expect("the equivalence phase ran");
    assert_eq!(equivalence.probes.unknown, 2, "{:?}", equivalence.probes);
}
