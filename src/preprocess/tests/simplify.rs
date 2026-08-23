//! The record a simplification leaves: which layer answers for the reduced
//! formula, and the `2^k` the count owes back.

use std::collections::HashMap;

use crate::cnf::{Clause, CnfFormula, Literal, VarId};
use crate::preprocess::dve::types::DveFate;
use crate::preprocess::equivalence::EquivMapping;
use crate::preprocess::renumber::Renumber;
use crate::preprocess::simplify::{
    DveReduction, EquivReduction, SimplifiedFormula, Stripped, VariableStripping,
};

/// An equivalence reduction over a variable space that has none left.
fn empty_equiv_reduction() -> EquivReduction {
    EquivReduction {
        formula: CnfFormula {
            num_vars: 0,
            clauses: Vec::new(),
        },
        mapping: EquivMapping {
            var_to_rep: Vec::new(),
            rep_to_equivs: HashMap::new(),
            representatives: Vec::new(),
        },
        renumbering: Renumber::of_kept(0, []),
    }
}

/// Stripping every variable leaves a formula a vtree cannot be built over, so
/// one backbone variable is handed back as a live unit clause. The promotion has
/// to reach the layer that ANSWERS: an equivalence reduction left in place would
/// keep answering with its own zero-variable formula and the promoted variable
/// would never be compiled.
#[test]
fn promoting_the_last_backbone_variable_leaves_one_live_unit_clause() {
    let mut record = SimplifiedFormula {
        original: CnfFormula {
            num_vars: 2,
            clauses: vec![
                Clause::new(vec![Literal::pos(VarId(0))]),
                Clause::new(vec![Literal::neg(VarId(1))]),
            ],
        },
        equiv_reduced: Some(empty_equiv_reduction()),
        dve_reduced: None,
        preprocessed: None,
        stripped: Some(Stripped {
            formula: CnfFormula {
                num_vars: 0,
                clauses: Vec::new(),
            },
            removed: VariableStripping {
                backbone: vec![(VarId(0), true), (VarId(1), false)],
                dead: Vec::new(),
                renumbering: Renumber::of_kept(2, []),
            },
        }),
    };
    assert_eq!(
        record.reduced_formula().num_vars,
        0,
        "the fixture must start with nothing left to compile",
    );

    record.promote_all_backbone_to_live();

    let reduced = record.reduced_formula();
    assert_eq!(
        reduced.num_vars, 1,
        "the promoted variable must be the one live variable",
    );
    assert_eq!(
        reduced.clauses,
        vec![Clause::new(vec![Literal::pos(VarId(0))])],
        "the promoted variable keeps the polarity its backbone entry forced",
    );
    assert!(
        record.equiv_reduced.is_none(),
        "an equivalence reduction over the old space must stop answering",
    );

    let removed = &record
        .stripped
        .as_ref()
        .expect("the stripping stays")
        .removed;
    assert_eq!(
        removed.backbone,
        vec![(VarId(1), false)],
        "the promoted variable is no longer accounted for as forced",
    );
    assert_eq!(
        removed.renumbering.kept(),
        &[VarId(0)],
        "the renumbering must name the promoted original variable",
    );
}

/// The exponent counts the variables NOTHING constrains, once each, across both
/// stages that produce them: the ones stripping found dead and the ones DVE left
/// free. A determined variable — forced, defined, or merged onto another — is a
/// factor of one and must not appear. Whatever a caller's own earlier stage
/// freed is added on top rather than folded in here.
#[test]
fn the_free_variable_exponent_counts_each_dead_and_eliminated_free_variable_once() {
    let record = SimplifiedFormula {
        original: CnfFormula {
            num_vars: 8,
            clauses: Vec::new(),
        },
        equiv_reduced: None,
        dve_reduced: Some(DveReduction {
            formula: CnfFormula {
                num_vars: 1,
                clauses: vec![Clause::new(vec![Literal::pos(VarId(0))])],
            },
            renumbering: Renumber::of_kept(5, [VarId(0)]),
            fates: vec![
                DveFate::Kept,
                DveFate::Free,
                DveFate::Defined,
                DveFate::Free,
                DveFate::Equiv {
                    rep: Literal::pos(VarId(0)),
                },
            ],
        }),
        preprocessed: None,
        stripped: Some(Stripped {
            formula: CnfFormula {
                num_vars: 5,
                clauses: Vec::new(),
            },
            removed: VariableStripping {
                backbone: vec![(VarId(0), true)],
                dead: vec![VarId(1), VarId(2)],
                renumbering: Renumber::of_kept(
                    8,
                    [VarId(3), VarId(4), VarId(5), VarId(6), VarId(7)],
                ),
            },
        }),
    };

    assert_eq!(
        record.free_var_exp(),
        4,
        "two dead variables and two DVE-free ones, and nothing else",
    );
    assert_eq!(record.count_lift(0).pow2_exp, 4);
    assert_eq!(
        record.count_lift(3).pow2_exp,
        7,
        "a caller's own exponent is added to this one, not merged with it",
    );
}
