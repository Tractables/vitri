use super::super::BisectDials;
use super::multilevel_hg_bisect;

#[test]
fn an_invalid_hypergraph_imbalance_returns_the_backend_error() {
    let error = multilevel_hg_bisect(
        3,
        &[vec![0, 1], vec![1, 2]],
        None,
        BisectDials {
            imbalance: 0.51,
            base_seed: 0,
            deadline: None,
        },
        1.0,
    )
    .expect_err("the imbalance exceeds one half");

    assert!(error.contains("imbalance") && error.contains("0.0..=0.5"));
}

/// A bisection memo answers each hypergraph with the partition a direct call
/// gives it, and charges the construction meter what that call charges,
/// whether it bisects the hypergraph or recalls it.
#[test]
fn a_bisection_memo_returns_and_charges_what_a_direct_bisection_does() {
    use super::BisectionMemo;
    use crate::decompose::meter;
    use crate::tests::common::Lcg;
    let dials = BisectDials {
        imbalance: super::IMBALANCE_BALANCED,
        base_seed: 0,
        deadline: None,
    };
    let charged = |bisect: &dyn Fn() -> Vec<u8>| {
        let before = meter::units_spent();
        let part = bisect();
        (part, meter::units_spent() - before)
    };
    let mut rng = Lcg::new(5);
    let hypergraphs: Vec<(usize, Vec<Vec<u32>>)> = (0..12)
        .map(|_| {
            let n = 4 + rng.below(36) as usize;
            let edges = (0..1 + rng.below(60))
                .map(|_| {
                    let mut pins: Vec<u32> = (0..2 + rng.below(4))
                        .map(|_| rng.below(n as u64) as u32)
                        .collect();
                    pins.sort_unstable();
                    pins.dedup();
                    pins
                })
                .filter(|pins| pins.len() >= 2)
                .collect();
            (n, edges)
        })
        .collect();
    let memo = BisectionMemo::default();
    for round in 0..2 {
        for (index, (n, edges)) in hypergraphs.iter().enumerate() {
            let direct = charged(&|| multilevel_hg_bisect(*n, edges, None, dials, 1.0).unwrap());
            let kept = charged(&|| memo.bisect(*n, edges.clone(), dials, 1.0).unwrap());
            assert_eq!(kept, direct, "hypergraph {index}, round {round}");
        }
    }
}
