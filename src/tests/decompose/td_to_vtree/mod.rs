//! The TD → vtree conversion over its public entry points: what the tree it
//! builds covers, and which reading of a decomposition the search picks.
//!
//! The conversion tests that reach a private item are beside the module, in
//! `src/decompose/td_to_vtree/tests/`. Nothing else tests this conversion.

mod coverage;
mod search;

use crate::cnf::{Clause, CnfFormula, Literal};
use crate::decompose::{Binarization, Place, Reading, Root, TreeDecomposition};
use crate::tests::common::make_td;
use crate::vtree::VarId;

/// The edge-aligned binarization with shallow placement (so separator lifting
/// fires) and centroid rooting: the reading a caller spells
/// `binarize=edge,place=shallow,root=centroid` on a `flowcutter-*` spec.
fn edge_reading() -> Reading {
    Reading {
        root: Some(Root::Centroid),
        place: Some(Place::Shallow),
        binarize: Some(Binarization::Edge),
    }
}

/// Synthetic "hub of clusters": a `hub`-variable clique separator shared by
/// `branches` clusters of `local` variables each. Every cluster's clauses
/// touch the whole hub, so the hub is a genuine full separator. Returns the
/// CNF plus a hand-built tree decomposition (root bag = hub; one child bag =
/// hub ∪ cluster per branch). `td_treewidth = hub + local − 1`.
fn hub_of_clusters(hub: u32, branches: u32, local: u32) -> (CnfFormula, TreeDecomposition) {
    let num_vars = hub + branches * local;
    let mut clauses: Vec<Clause> = Vec::new();
    for i in 0..hub {
        for j in (i + 1)..hub {
            clauses.push(Clause::new(vec![
                Literal::pos(VarId(i)),
                Literal::pos(VarId(j)),
            ]));
        }
    }
    let mut bags = vec![(0..hub).collect()];
    let mut tree_edges = Vec::new();
    for b in 0..branches {
        let base = hub + b * local;
        let locs: Vec<u32> = (base..base + local).collect();
        for i in 0..local as usize {
            for j in (i + 1)..local as usize {
                clauses.push(Clause::new(vec![
                    Literal::pos(VarId(locs[i])),
                    Literal::pos(VarId(locs[j])),
                ]));
            }
        }
        for &lv in &locs {
            for h in 0..hub {
                clauses.push(Clause::new(vec![
                    Literal::pos(VarId(lv)),
                    Literal::pos(VarId(h)),
                ]));
            }
        }
        let bag_id = bags.len();
        let mut verts: Vec<u32> = (0..hub).collect();
        verts.extend(&locs);
        bags.push(verts);
        tree_edges.push((0, bag_id));
    }
    (
        CnfFormula { num_vars, clauses },
        make_td(bags, tree_edges, num_vars),
    )
}
