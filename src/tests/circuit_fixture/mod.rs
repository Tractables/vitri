//! A deterministic, self-generated structured CNF fixture.
//!
//! This module exists so that **no third-party test data ships with the crate**.
//! The vtree selection and stats pins need a formula with genuine circuit
//! structure — a synthetic random or chain instance ranks candidates
//! meaninglessly — so this generates one instead of vendoring a benchmark CNF
//! of unclear provenance.
//!
//! [`multiplier`] is a Tseitin encoding of a binary array multiplier: `WIDTH²`
//! partial products feeding `WIDTH - 1` ripple rows of adders. It is a pure
//! function — no RNG, no clock, no I/O — and allocates variable ids in one
//! fixed order, so the formula is byte-identical on every run and every
//! platform. The product bits are left unconstrained: the instance is the
//! circuit itself, not a factoring query.

use crate::cnf::{Clause, CnfFormula, Literal};

/// Operand width of the generated multiplier.
///
/// An `n`-bit array multiplier costs `2n` inputs, `n²` partial products and
/// `2n(n-1)` adder wires, so the fixture's size is set entirely by this number:
/// 8 gives 192 variables and 920 clauses. 6 would be the more obvious width but
/// yields only 108 variables — too small to rank a vtree portfolio
/// meaningfully. Change it and every selection pin below must be re-observed,
/// not guessed.
const WIDTH: usize = 8;

/// A Tseitin-encoded [`WIDTH`]×[`WIDTH`] binary array multiplier.
///
/// Structure, in the order variables are allocated: the two operands `a` and
/// `b`; the `WIDTH²` partial products `p[i][j] ⇔ a[i] ∧ b[j]`; then one ripple
/// row per operand bit `i ≥ 1`, each accumulating `p[i][..]` into the running
/// sum at offset `i` with a half adder at its lowest position and full adders
/// above it.
pub(crate) fn multiplier() -> CnfFormula {
    array_multiplier(WIDTH).0
}

/// The formula, plus the `2n` product wires in bit order (least significant
/// first) as 1-based DIMACS ids. Only the soundness test below needs the second
/// half, but it comes from THIS construction rather than a replay of it — a
/// second copy of the index bookkeeping would be free to drift.
fn array_multiplier(n: usize) -> (CnfFormula, Vec<i32>) {
    let mut b = Builder::default();

    let a: Vec<i32> = (0..n).map(|_| b.fresh()).collect();
    let y: Vec<i32> = (0..n).map(|_| b.fresh()).collect();

    // p[i][j] ⇔ a[i] ∧ y[j] — the AND array.
    let p: Vec<Vec<i32>> = (0..n)
        .map(|i| (0..n).map(|j| b.and2(a[i], y[j])).collect())
        .collect();

    // Row 0 needs no adder: it IS the running sum, at positions 0..n.
    let mut acc: Vec<i32> = p[0].clone();

    for i in 1..n {
        // Position i has no carry-in, so it is a half adder.
        let (sum, mut carry) = (b.xor2(acc[i], p[i][0]), b.and2(acc[i], p[i][0]));
        acc[i] = sum;

        for (j, &pij) in p[i].iter().enumerate().skip(1) {
            let k = i + j;
            if k < acc.len() {
                let (sum, next) = (b.xor3(acc[k], pij, carry), b.maj3(acc[k], pij, carry));
                acc[k] = sum;
                carry = next;
            } else {
                // First row to reach past the running sum's top bit: nothing to
                // add to, so a half adder of the operand bit and the carry.
                let (sum, next) = (b.xor2(pij, carry), b.and2(pij, carry));
                acc.push(sum);
                carry = next;
            }
        }
        // The row's carry-out becomes the sum's new top bit.
        acc.push(carry);
    }

    let formula = CnfFormula {
        num_vars: b.next as u32 - 1,
        clauses: b.clauses,
    };
    (formula, acc)
}

/// Allocates 1-based DIMACS variable ids in call order and collects clauses.
struct Builder {
    next: i32,
    clauses: Vec<Clause>,
}

impl Default for Builder {
    fn default() -> Self {
        Self {
            next: 1,
            clauses: Vec::new(),
        }
    }
}

impl Builder {
    fn fresh(&mut self) -> i32 {
        let v = self.next;
        self.next += 1;
        v
    }

    fn clause(&mut self, lits: &[i32]) {
        self.clauses.push(Clause::new(
            lits.iter().map(|&l| Literal::from(l)).collect(),
        ));
    }

    /// `o ⇔ x ∧ y` — 3 clauses. Also the half-adder carry.
    fn and2(&mut self, x: i32, y: i32) -> i32 {
        let o = self.fresh();
        self.clause(&[-o, x]);
        self.clause(&[-o, y]);
        self.clause(&[o, -x, -y]);
        o
    }

    /// `s ⇔ x ⊕ y` — 4 clauses of width 3. The half-adder sum.
    fn xor2(&mut self, x: i32, y: i32) -> i32 {
        let s = self.fresh();
        self.clause(&[-x, -y, -s]);
        self.clause(&[x, y, -s]);
        self.clause(&[x, -y, s]);
        self.clause(&[-x, y, s]);
        s
    }

    /// `s ⇔ x ⊕ y ⊕ z` — 8 clauses of width 4, one blocking each input
    /// assignment paired with the wrong parity. The full-adder sum.
    fn xor3(&mut self, x: i32, y: i32, z: i32) -> i32 {
        let s = self.fresh();
        for bits in 0u8..8 {
            let (bx, by, bz) = (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0);
            let parity = bx ^ by ^ bz;
            self.clause(&[
                if bx { -x } else { x },
                if by { -y } else { y },
                if bz { -z } else { z },
                if parity { s } else { -s },
            ]);
        }
        s
    }

    /// `c ⇔ majority(x, y, z)` — 6 clauses of width 3: any two inputs true
    /// force `c`, any two false force `¬c`. The full-adder carry.
    fn maj3(&mut self, x: i32, y: i32, z: i32) -> i32 {
        let c = self.fresh();
        for (u, v) in [(x, y), (y, z), (x, z)] {
            self.clause(&[-u, -v, c]);
            self.clause(&[u, v, -c]);
        }
        c
    }
}

mod soundness;
