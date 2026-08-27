//! Formula context around goatd's PACE parser.

use super::*;

#[test]
fn a_decomposition_uses_the_exported_graph_vertex_universe() {
    let formula = make_formula(3, vec![vec![1, 2]]);
    let graph = GraphKind::Primal.build(&formula);
    let text = "s td 1 3 3\nb 1 1 2 3\n";
    let td = parse_pace_td(text, &graph).expect("the bag covers the exported graph");

    assert_eq!(td.num_vertices(), graph.num_vertices());
}

#[test]
fn an_incidence_decomposition_must_declare_isolated_formula_variables() {
    let formula = make_formula(2, vec![vec![1]]);
    let graph = GraphKind::Incidence.build(&formula);
    let text = "s td 1 1 1\nb 1 1\n";

    assert!(parse_pace_td(text, &graph).is_err());
}

#[test]
fn goatd_contract_rejects_a_pace_decomposition_that_misses_a_graph_edge() {
    let formula = make_formula(2, vec![vec![1, 2]]);
    let graph = GraphKind::Primal.build(&formula);
    let text = "s td 2 1 2\nb 1 1\nb 2 2\n1 2\n";

    assert!(parse_pace_td(text, &graph).is_err());
}
