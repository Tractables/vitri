use crate::decompose::goatd::nested_diss::*;

/// The parameters every test here runs under, varying only the base-case
/// threshold: no deadline, seed 0, and the balance tolerance the production
/// caller passes.
fn params<'a>(salt: &'a [u32], base_thresh: usize) -> NdParams<'a> {
    NdParams {
        salt,
        base_thresh,
        max_imbalance: 0.2,
        hard_deadline: None,
        base_seed: 0,
    }
}

#[test]
fn empty_graph_returns_empty_order() {
    let order = nd_order(&[], &[], &params(&[], 32), 0);
    assert!(order.is_empty());
}

#[test]
fn small_graph_falls_through_to_base_case() {
    let active = vec![0, 1, 2];
    let edges = vec![(0, 1), (0, 2), (1, 2)];
    let salt = vec![7, 3, 11];
    let order = nd_order(&active, &edges, &params(&salt, 32), 0);
    assert_eq!(order.len(), 3);
    let mut s: Vec<u32> = order.clone();
    s.sort();
    assert_eq!(s, vec![0, 1, 2]);
}

#[test]
fn grid_10x10_produces_full_order() {
    let mut edges = Vec::new();
    for r in 0..10u32 {
        for c in 0..10u32 {
            let v = r * 10 + c;
            if c + 1 < 10 {
                edges.push((v, v + 1));
            }
            if r + 1 < 10 {
                edges.push((v, v + 10));
            }
        }
    }
    let active: Vec<u32> = (0..100).collect();
    let salt: Vec<u32> = (0..100)
        .map(|i| (i as u32).wrapping_mul(2_654_435_761))
        .collect();
    let order = nd_order(&active, &edges, &params(&salt, 8), 0);
    assert_eq!(order.len(), 100);
    let mut s = order.clone();
    s.sort();
    assert_eq!(s, (0..100).collect::<Vec<u32>>());
}
