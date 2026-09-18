use crate::cnf::Clause;
use crate::cnf::CnfFormula;
use crate::cnf::Literal;
use crate::decompose::hybrid::*;
use crate::decompose::{ConversionRequest, Reading};
use crate::tests::td_fixture::make_test_td;
use crate::vtree::VarId;
use crate::vtree::VtreeIdx;

fn make_test_formula() -> CnfFormula {
    let clauses = vec![
        Clause::new(vec![
            Literal {
                var: VarId::from_dimacs(1),
                positive: true,
            },
            Literal {
                var: VarId::from_dimacs(2),
                positive: false,
            },
        ]),
        Clause::new(vec![
            Literal {
                var: VarId::from_dimacs(2),
                positive: true,
            },
            Literal {
                var: VarId::from_dimacs(3),
                positive: true,
            },
        ]),
        Clause::new(vec![
            Literal {
                var: VarId::from_dimacs(3),
                positive: false,
            },
            Literal {
                var: VarId::from_dimacs(4),
                positive: true,
            },
        ]),
        Clause::new(vec![
            Literal {
                var: VarId::from_dimacs(4),
                positive: false,
            },
            Literal {
                var: VarId::from_dimacs(5),
                positive: true,
            },
        ]),
        Clause::new(vec![
            Literal {
                var: VarId::from_dimacs(5),
                positive: false,
            },
            Literal {
                var: VarId::from_dimacs(6),
                positive: true,
            },
        ]),
    ];
    CnfFormula::from_parts(6, clauses)
}

#[test]
fn guided_bisect_with_precomputed_td() {
    let formula = make_test_formula();
    let td = make_test_td();
    let dials = crate::decompose::BisectDials {
        imbalance: 0.30,
        base_seed: 0,
        deadline: None,
    };
    let conversion = ConversionRequest::open(Reading::default(), None);
    let result = vtree_from_guided_bisect(&formula, &td, dials, conversion);
    assert!(result.is_ok());
    let vtree = result.unwrap();
    let leaf_count = (0..vtree.num_nodes())
        .filter(|&i| vtree.node(VtreeIdx(i as u32)).is_leaf())
        .count();
    assert_eq!(leaf_count, 6);
}
