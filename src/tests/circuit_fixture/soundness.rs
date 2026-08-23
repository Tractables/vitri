use super::*;

/// The fixture is a pure function: two calls must produce the identical
/// formula, or every pin built on it is pinning noise.
#[test]
fn generation_is_deterministic() {
    let (a, b) = (multiplier(), multiplier());
    assert_eq!(a.num_vars, b.num_vars);
    assert_eq!(a.clauses, b.clauses);
}

/// Size floor. The pins rank a vtree portfolio on this formula, which is
/// only meaningful on an instance with real structure and real width.
#[test]
fn is_large_enough_to_rank_a_portfolio() {
    let f = multiplier();
    assert!(f.num_vars >= 150, "fixture too small: {} vars", f.num_vars);
    assert!(
        f.clauses.len() >= 500,
        "fixture too small: {} clauses",
        f.clauses.len()
    );
    // Every variable the header declares is actually used — a padded
    // `num_vars` would silently add free variables to every pin.
    let mut seen = vec![false; f.num_vars as usize];
    for c in &f.clauses {
        for l in &c.literals {
            seen[l.var.idx()] = true;
        }
    }
    assert!(seen.iter().all(|&s| s), "declared but unused variable");
}

/// Soundness of the encoding: driving the two operand words and unit-
/// propagating the gate definitions must produce the true product on the
/// wires `array_multiplier` reports. No SAT solver needed — every gate is
/// functionally defined by earlier variables, so propagation is complete.
///
/// Catches the failure mode that matters: a fixture that *looks* like a
/// multiplier but encodes something else still ranks a portfolio, so a
/// structural typo would never surface through the pins that consume it.
#[test]
fn encodes_multiplication() {
    // Width 4 keeps the operand sweep exhaustive (256 pairs) while
    // exercising every construct the shipped width uses: the AND array,
    // the per-row half adder, the full-adder chain, and the one row that
    // extends past the running sum's top bit.
    const N: usize = 4;
    let (f, product_wires) = array_multiplier(N);
    for x in 0u32..(1 << N) {
        for y in 0u32..(1 << N) {
            let mut vals = vec![false; f.num_vars as usize + 1];
            let mut assigned = vec![false; f.num_vars as usize + 1];
            for i in 0..N {
                vals[1 + i] = x >> i & 1 != 0;
                vals[1 + N + i] = y >> i & 1 != 0;
                assigned[1 + i] = true;
                assigned[1 + N + i] = true;
            }
            propagate(&f, &mut vals, &mut assigned);
            let product: u64 = product_wires
                .iter()
                .enumerate()
                .map(|(k, &v)| u64::from(vals[v as usize]) << k)
                .sum();
            assert_eq!(product, u64::from(x) * u64::from(y), "{x} * {y}");
        }
    }
}

/// Repeatedly satisfy every clause that is unit under the current partial
/// assignment, to fixpoint.
fn propagate(f: &CnfFormula, vals: &mut [bool], assigned: &mut [bool]) {
    let mut progress = true;
    while progress {
        progress = false;
        for c in &f.clauses {
            let mut unassigned = None;
            let mut satisfied = false;
            let mut open = 0;
            for l in &c.literals {
                let v = l.var.to_dimacs() as usize;
                if !assigned[v] {
                    open += 1;
                    unassigned = Some((v, l.positive));
                } else if vals[v] == l.positive {
                    satisfied = true;
                    break;
                }
            }
            if satisfied || open != 1 {
                continue;
            }
            let (v, pol) = unassigned.expect("open == 1");
            vals[v] = pol;
            assigned[v] = true;
            progress = true;
        }
    }
    assert!(
        assigned.iter().skip(1).all(|&a| a),
        "propagation left a gate undetermined — the encoding is not a circuit"
    );
}
