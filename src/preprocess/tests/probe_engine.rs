use crate::cnf::CnfFormula;
use crate::cnf::Literal;
use crate::preprocess::probe_engine::*;
use crate::tests::common::clause;
use std::collections::HashSet;
use std::time::Duration;

const TEST_BUDGET: Duration = Duration::from_secs(10);

/// observe_model must split each class by the model's bit and keep the
/// ⊤-class's true-half as the anchor.
#[test]
fn observe_model_splits_and_tracks_top() {
    // Refinement is solver-free, so this runs on the partition alone.
    let mut p = Partition::new();

    // Single ⊤-class of true-literals [1,2,3,4] (dimacs).
    p.classes = vec![vec![1, 2, 3, 4]];
    p.top = 0;
    // Model: var0 true, var1 false, var2 true, var3 false → [1,-2,3,-4].
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
    // x0 forced true: (x0∨x1) ∧ (x0∨¬x1).
    // x2 ≡ x3: (¬x2∨x3) ∧ (x2∨¬x3), anchored by (x2∨x4) to stay SAT.
    let f = CnfFormula {
        num_vars: 5,
        clauses: vec![
            clause(&[(0, true), (1, true)]),
            clause(&[(0, true), (1, false)]),
            clause(&[(2, false), (3, true)]),
            clause(&[(2, true), (3, false)]),
            clause(&[(2, true), (4, true)]),
        ],
    };

    let mut e = ProbeEngine::new(&f).expect("the solver allocates");
    let bb_eng = e.run_backbone(TEST_BUDGET);

    // Golden: x0=true is the UNIQUE backbone. x1 is free; x2≡x3 both take T
    // (models with x2=x3=T) and F (x2=x3=F, forcing x4=T via (x2∨x4)), and x4
    // takes both — so none of x1/x2/x3/x4 is backbone. Soundness guarantees
    // the engine confirms no spurious literal, so the set is exactly {x0=T}.
    let set_eng: HashSet<(u32, bool)> = bb_eng
        .forced
        .iter()
        .map(|l| (l.var.0, l.positive))
        .collect();
    assert_eq!(
        set_eng,
        HashSet::from([(0u32, true)]),
        "engine backbone must be exactly {{x0=true}}, got {:?}",
        bb_eng.forced,
    );
    // The field is the single source of the returned `forced`.
    assert_eq!(e.partition.confirmed_backbone.len(), bb_eng.forced.len());

    // No phase-4 mapping in this direct test → identity mapping.
    let eq_eng = e.run_equiv(TEST_BUDGET, &None);
    let has_23 = |v: &Vec<(Literal, Literal)>| {
        v.iter().any(|(a, b)| {
            let vars = [a.var.0, b.var.0];
            vars.contains(&2) && vars.contains(&3)
        })
    };
    assert!(
        has_23(&eq_eng.equivalences),
        "engine must find x2 ≡ x3, got {:?}",
        eq_eng.equivalences
    );
}
