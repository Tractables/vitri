//! The FORCE vtree builder.
//!
//! Shared by both topics: the structured fixture [`axis_formula`], the config
//! builder [`cfg_with`], and the quality measure [`max_load`].

use crate::cnf::CnfFormula;
use crate::decompose::force::*;
use crate::tests::common::{assert_covers_all_vars, clause_dimacs};
use crate::vtree::Vtree;

mod axes;
mod build;

/// A deterministic ~100-variable formula.
fn axis_formula() -> CnfFormula {
    let n = 100u32;
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut rv = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n as u64) as i32 + 1
    };
    let mut clauses = Vec::new();
    for _ in 0..300 {
        let a = rv();
        let mut b = rv();
        while b == a {
            b = rv();
        }
        let mut c = rv();
        while c == a || c == b {
            c = rv();
        }
        clauses.push(clause_dimacs(&[a, -b, c]));
    }
    CnfFormula {
        num_vars: n,
        clauses,
    }
}

fn cfg_with(mutate: impl FnOnce(&mut ForceConfig)) -> ForceConfig {
    let mut c = ForceConfig::new(ForceMode::Mst);
    mutate(&mut c);
    c
}

/// Max clause-LCA load over internal nodes, through the same helpers the feedback
/// loop optimizes against.
fn max_load(vt: &Vtree, f: &CnfFormula) -> u32 {
    let (_lca, loads) = crate::score::clause_lca_nodes(vt, f);
    max_internal_load(vt, &loads)
}
