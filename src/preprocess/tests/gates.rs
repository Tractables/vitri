use crate::cnf::VarId;
use crate::preprocess::gates::*;
use crate::tests::common::make_formula;

#[test]
fn detect_and_gate() {
    // y(3) = x1(1) ∧ x2(2)
    let f = make_formula(3, vec![vec![-3, 1], vec![-3, 2], vec![3, -1, -2]]);
    let gm = detect_gates(&f);
    assert_eq!(gm.gates.len(), 1);
    assert_eq!(gm.gates[0].gate_type, GateType::And);
    assert!(gm.eliminated.contains(&VarId(2))); // var 3 is VarId(2)
}

#[test]
fn detect_or_gate() {
    // y(3) = x1(1) ∨ x2(2)
    let f = make_formula(3, vec![vec![3, -1], vec![3, -2], vec![-3, 1, 2]]);
    let gm = detect_gates(&f);
    assert_eq!(gm.gates.len(), 1);
    assert_eq!(gm.gates[0].gate_type, GateType::Or);
}

#[test]
fn detect_xor_gate() {
    // y(3) = x1(1) ⊕ x2(2)
    // Invalid assignments for XOR: (y=0,x1=1,x2=0), (y=0,x1=0,x2=1),
    //                              (y=1,x1=0,x2=0), (y=1,x1=1,x2=1)
    let f = make_formula(
        3,
        vec![
            vec![3, -1, 2],
            vec![3, 1, -2],
            vec![-3, 1, 2],
            vec![-3, -1, -2],
        ],
    );
    let gm = detect_gates(&f);
    assert_eq!(gm.gates.len(), 1);
    assert_eq!(gm.gates[0].gate_type, GateType::Xor);
}

#[test]
fn detect_chain() {
    // y(3) = x1(1) ∧ x2(2): (-3,1), (-3,2), (3,-1,-2)
    // z(4) = y(3) ∧ x3(5): (-4,3), (-4,5), (4,-3,-5)
    let f = make_formula(
        5,
        vec![
            vec![-3, 1],
            vec![-3, 2],
            vec![3, -1, -2],
            vec![-4, 3],
            vec![-4, 5],
            vec![4, -3, -5],
        ],
    );
    let gm = detect_gates(&f);
    assert_eq!(gm.gates.len(), 2);
}

#[test]
fn detect_ite_gate() {
    // y(4) = ITE(s(1), a(2), b(3))
    // When s=1: y=a. When s=0: y=b.
    let f = make_formula(
        4,
        vec![
            vec![-1, -2, 4],
            vec![-1, 2, -4],
            vec![1, -3, 4],
            vec![1, 3, -4],
        ],
    );
    let gm = detect_gates(&f);
    assert_eq!(gm.gates.len(), 1, "should detect one ITE gate");
    assert_eq!(gm.gates[0].gate_type, GateType::Ite);
    assert!(gm.eliminated.contains(&VarId(3))); // var 4 is VarId(3)
}

/// The same four ternary clauses over the same three variables encode either
/// `y = a ⊕ b` or its negation, and the parity of positive literals per clause is
/// the only thing that separates them. Collapsing the odd-parity encoding into
/// the even one names the complement of the function the formula wrote.
#[test]
fn an_odd_parity_encoding_is_not_reported_as_an_even_one() {
    // One positive literal per clause in the first three, three in the last.
    let f = make_formula(
        3,
        vec![
            vec![3, -1, -2],
            vec![-3, 1, -2],
            vec![-3, -1, 2],
            vec![3, 1, 2],
        ],
    );
    let gm = detect_gates(&f);
    assert_eq!(gm.gates.len(), 1);
    assert_eq!(
        gm.gates[0].gate_type,
        GateType::Xnor,
        "an odd parity of positive literals is the negated output",
    );
}

#[test]
fn detect_no_gate_for_random_clauses() {
    let f = make_formula(
        4,
        vec![vec![1, 2, 3], vec![-1, -2], vec![2, 4], vec![-3, -4]],
    );
    let gm = detect_gates(&f);
    assert!(gm.gates.is_empty(), "should detect no gates");
}
