//! Codecs for the results returned by Arjun.
//!
//! Encoders destructure each result exhaustively so an added field cannot
//! silently stop crossing the process boundary.

use crate::cnf::{Clause, CnfFormula, Reduced, ShowSet, VarId};
use crate::cnf::{Space, Weights};

use super::arjun::{ArjunResult, ArjunWeightedResult};
use super::var_map::VarMap;

use super::fork_payload::{
    Dec, ForkPayload, get_vec, put_i32, put_len, put_literal, put_str, put_u32,
};

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
            independent_support,
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
        put_len(out, independent_support.len());
        for var in independent_support.iter_vars() {
            put_u32(out, var.get());
        }
        put_var_map(out, input_to_reduced_lit);
    }

    fn decode(d: &mut Dec<'_>) -> Option<Self> {
        let formula = get_formula(d)?;
        let multiplier_exp = d.get_u32()?;
        let backbone = get_vec(d, |d| d.get_literal())?;
        let equiv = get_vec(d, |d| Some((d.get_literal()?, d.get_literal()?)))?;
        let learnt_clauses = get_vec(d, |d| get_vec(d, |d| d.get_i32()))?;
        let independent_support =
            ShowSet::<Reduced>::from_vars(get_vec(d, |d| d.get_u32().and_then(VarId::new))?);
        let input_to_reduced_lit = get_var_map(d)?;
        Some(ArjunResult {
            formula,
            multiplier_exp,
            backbone,
            equiv,
            learnt_clauses,
            independent_support,
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
