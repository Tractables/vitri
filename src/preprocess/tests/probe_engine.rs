use crate::cnf::CnfFormula;
use crate::cnf::Literal;
use crate::preprocess::probe_engine::*;
use crate::preprocess::tests::wall_meter;
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
    // x1 forced true: (x1∨x2) ∧ (x1∨¬x2).
    // x3 ≡ x4: (¬x3∨x4) ∧ (x3∨¬x4), anchored by (x3∨x5) to stay SAT.
    let f = CnfFormula::from_parts(
        5,
        vec![
            clause(&[(1, true), (2, true)]),
            clause(&[(1, true), (2, false)]),
            clause(&[(3, false), (4, true)]),
            clause(&[(3, true), (4, false)]),
            clause(&[(3, true), (5, true)]),
        ],
    );

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
