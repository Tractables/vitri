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
            effort_scale: 1.0,
        },
    )
    .expect_err("the imbalance exceeds one half");

    assert!(error.contains("imbalance") && error.contains("0.0..=0.5"));
}
