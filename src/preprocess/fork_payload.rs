//! The wire codec that carries a reduce's result out of the forked child of
//! [`super::fork_budget`] and back into the parent.

use crate::cnf::{Clause, CnfFormula};
use crate::cnf::{Literal, Space, VarId, Weights};

use super::arjun::{ArjunResult, ArjunWeightedResult};
use super::var_map::VarMap;

/// A result type that can travel from the forked child back to the parent.
///
/// The encoding is a length-prefixed little-endian byte stream, decoded in the
/// same order; it is private to one `fork`/`read` pair in one process, so there
/// is no versioning or endianness concern.
///
/// **Every `encode` below destructures its struct exhaustively** — adding a
/// field to one of these results is then a compile error here, instead of a
/// field that silently stops crossing the process boundary.
pub(crate) trait ForkPayload: Sized {
    /// Append `self` to `out`.
    fn encode(&self, out: &mut Vec<u8>);
    /// Read a value back. `None` on a truncated or malformed stream.
    fn decode(d: &mut Dec<'_>) -> Option<Self>;
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_len(out: &mut Vec<u8>, v: usize) {
    put_u64(out, v as u64);
}
fn put_str(out: &mut Vec<u8>, s: &str) {
    put_len(out, s.len());
    out.extend_from_slice(s.as_bytes());
}
/// A literal packs into one `u32` as `var * 2 + polarity`. Variable ids are CNF
/// indices (bounded by the formula's `num_vars`), so the top bit is never used
/// in practice; the assert documents and checks the assumption in debug builds.
fn put_literal(out: &mut Vec<u8>, l: Literal) {
    debug_assert!(l.var.0 < 1 << 31, "var id too large to pack: {}", l.var.0);
    put_u32(out, (l.var.0 << 1) | u32::from(l.positive));
}

/// Cursor over the child's byte stream.
pub(crate) struct Dec<'a> {
    pub(super) rest: &'a [u8],
}

impl<'a> Dec<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Dec { rest: bytes }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.rest.len() < n {
            return None;
        }
        let (head, tail) = self.rest.split_at(n);
        self.rest = tail;
        Some(head)
    }
    pub(super) fn get_u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    pub(super) fn get_u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn get_i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn get_u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    /// Length prefixes are bounded by the bytes actually received, so a corrupt
    /// stream cannot make us reserve a huge vector.
    fn get_len(&mut self) -> Option<usize> {
        let n = usize::try_from(self.get_u64()?).ok()?;
        (n <= self.rest.len()).then_some(n)
    }
    fn get_str(&mut self) -> Option<&'a str> {
        let n = self.get_len()?;
        std::str::from_utf8(self.take(n)?).ok()
    }
    fn get_literal(&mut self) -> Option<Literal> {
        let packed = self.get_u32()?;
        Some(Literal::new(VarId(packed >> 1), packed & 1 == 1))
    }
}

/// The byte-count prefix is validated against the bytes remaining before use,
/// so a corrupt stream cannot force a huge allocation or call `elem` more
/// times than the stream can supply.
pub(super) fn get_vec<T>(
    d: &mut Dec<'_>,
    mut elem: impl FnMut(&mut Dec<'_>) -> Option<T>,
) -> Option<Vec<T>> {
    let n = usize::try_from(d.get_u64()?).ok()?;
    if n > d.rest.len() {
        return None;
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(elem(d)?);
    }
    Some(out)
}

fn put_formula(out: &mut Vec<u8>, f: &CnfFormula) {
    let CnfFormula { num_vars, clauses } = f;
    put_u32(out, *num_vars);
    put_len(out, clauses.len());
    for c in clauses {
        put_len(out, c.literals.len());
        for &l in &c.literals {
            put_literal(out, l);
        }
    }
}

fn get_formula(d: &mut Dec<'_>) -> Option<CnfFormula> {
    let num_vars = d.get_u32()?;
    let clauses = get_vec(d, |d| {
        // Construct the struct directly: the literals came from a formula that
        // already satisfied `Clause::new`'s precondition, and re-running its
        // O(k²) check over a multi-million-clause formula is pure overhead.
        let literals = get_vec(d, |d| d.get_literal())?;
        Some(Clause { literals })
    })?;
    Some(CnfFormula { num_vars, clauses })
}

/// Rationals travel as their exact `num/den` decimal text — the same lossless
/// form the weighted Arjun FFI already uses for weights.
fn put_rational(out: &mut Vec<u8>, r: &num_rational::BigRational) {
    put_str(out, &format!("{}/{}", r.numer(), r.denom()));
}

fn get_rational(d: &mut Dec<'_>) -> Option<num_rational::BigRational> {
    crate::cnf::parse_weight(d.get_str()?).ok()
}

/// A variable map travels as its entries, with `None` (a source variable with no
/// target counterpart) written as 0 — not a legal DIMACS literal, so the two
/// cases stay distinguishable.
fn put_var_map<Src: Space, Tgt: Space>(out: &mut Vec<u8>, map: &VarMap<Src, Tgt>) {
    put_len(out, map.len());
    for e in map.iter() {
        put_i32(out, e.unwrap_or(0));
    }
}

fn get_var_map<Src: Space, Tgt: Space>(d: &mut Dec<'_>) -> Option<VarMap<Src, Tgt>> {
    get_vec(d, |d| {
        d.get_i32().map(|l| if l == 0 { None } else { Some(l) })
    })
    .map(VarMap::from_entries)
}

impl ForkPayload for ArjunResult {
    fn encode(&self, out: &mut Vec<u8>) {
        let ArjunResult {
            formula,
            multiplier_exp,
            backbone,
            equiv,
            learnt_clauses,
            input_to_reduced_lit,
        } = self;
        put_formula(out, formula);
        put_u32(out, *multiplier_exp);
        put_len(out, backbone.len());
        for &l in backbone {
            put_literal(out, l);
        }
        put_len(out, equiv.len());
        for &(a, b) in equiv {
            put_literal(out, a);
            put_literal(out, b);
        }
        put_len(out, learnt_clauses.len());
        for cl in learnt_clauses {
            put_len(out, cl.len());
            for &l in cl {
                put_i32(out, l);
            }
        }
        put_var_map(out, input_to_reduced_lit);
    }

    fn decode(d: &mut Dec<'_>) -> Option<Self> {
        let formula = get_formula(d)?;
        let multiplier_exp = d.get_u32()?;
        let backbone = get_vec(d, |d| d.get_literal())?;
        let equiv = get_vec(d, |d| Some((d.get_literal()?, d.get_literal()?)))?;
        let learnt_clauses = get_vec(d, |d| get_vec(d, |d| d.get_i32()))?;
        let input_to_reduced_lit = get_var_map(d)?;
        Some(ArjunResult {
            formula,
            multiplier_exp,
            backbone,
            equiv,
            learnt_clauses,
            input_to_reduced_lit,
        })
    }
}

impl ForkPayload for ArjunWeightedResult {
    fn encode(&self, out: &mut Vec<u8>) {
        let ArjunWeightedResult {
            formula,
            weights,
            multiplier,
            input_to_reduced_lit,
        } = self;
        put_formula(out, formula);
        let weight_pairs = weights.to_dimacs_pairs();
        put_len(out, weight_pairs.len());
        for (lit, w) in &weight_pairs {
            put_i32(out, *lit);
            put_rational(out, w);
        }
        put_rational(out, multiplier);
        put_var_map(out, input_to_reduced_lit);
    }

    fn decode(d: &mut Dec<'_>) -> Option<Self> {
        let formula = get_formula(d)?;
        let weight_pairs = get_vec(d, |d| Some((d.get_i32()?, get_rational(d)?)))?;
        let weights = Weights::from_dimacs_pairs(&weight_pairs, formula.num_vars as usize);
        let multiplier = get_rational(d)?;
        let input_to_reduced_lit = get_var_map(d)?;
        Some(ArjunWeightedResult {
            formula,
            weights,
            multiplier,
            input_to_reduced_lit,
        })
    }
}
