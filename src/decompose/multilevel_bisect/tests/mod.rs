use super::multilevel_bisect;

#[test]
fn an_invalid_graph_imbalance_returns_the_backend_error() {
    let graph = goatd::Graph::new(3, [(0, 1), (1, 2)]);
    let error = multilevel_bisect(&graph, 0.51, 0).expect_err("the imbalance exceeds one half");

    assert!(error.contains("imbalance") && error.contains("0.0..=0.5"));
}
