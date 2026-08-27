//! Formula context around goatd's PACE parser.

use super::*;

#[test]
fn a_decomposition_uses_its_declared_vertex_universe() {
    let text = "s td 1 3 3\nb 1 1 2 3\n";
    for kind in [GraphKind::Primal, GraphKind::Incidence] {
        let td =
            parse_pace_td(text, kind, 2).expect("the PACE solution line bounds the decomposition");
        assert_eq!(td.num_vertices(), 3);
    }
}

#[test]
fn an_incidence_decomposition_may_omit_isolated_formula_variables() {
    let text = "s td 1 1 1\nb 1 1\n";
    let td = parse_pace_td(text, GraphKind::Incidence, 2)
        .expect("conversion appends formula variables absent from every bag");

    assert_eq!(td.num_vertices(), 1);
}
