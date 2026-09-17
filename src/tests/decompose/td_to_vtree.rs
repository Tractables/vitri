//! The TD → vtree conversion, driven through the public entry point and
//! reading vocabulary a caller holding its own decomposition uses.

use crate::cnf::{Clause, CnfFormula, Literal};
use crate::decompose::{
    Binarization, Place, Reading, Root, TreeDecomposition, td_to_vtree_reading,
};
use crate::tests::common::{assert_covers_all_vars, make_td};
use crate::vtree::VarId;

/// The edge-aligned binarization with shallow placement (so separator lifting fires)
/// and centroid rooting — the reading a caller spells
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
/// hub ∪ cluster per branch), whose bags hold graph vertices, so a vertex `u`
/// is the literal's `VarId::from_idx(u)`. `td_treewidth = hub + local − 1`.
fn hub_of_clusters(hub: u32, branches: u32, local: u32) -> (CnfFormula, TreeDecomposition) {
    let num_vars = hub + branches * local;
    let mut clauses: Vec<Clause> = Vec::new();
    for i in 0..hub {
        for j in (i + 1)..hub {
            clauses.push(Clause::new(vec![
                Literal::pos(VarId::from_idx(i as usize)),
                Literal::pos(VarId::from_idx(j as usize)),
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
                    Literal::pos(VarId::from_idx(locs[i] as usize)),
                    Literal::pos(VarId::from_idx(locs[j] as usize)),
                ]));
            }
        }
        for &lv in &locs {
            for h in 0..hub {
                clauses.push(Clause::new(vec![
                    Literal::pos(VarId::from_idx(lv as usize)),
                    Literal::pos(VarId::from_idx(h as usize)),
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

#[test]
fn edge_one_leaf_per_var_valid_tree() {
    let (formula, td) = hub_of_clusters(8, 6, 4);
    let nv = formula.num_vars;
    let vtree = td_to_vtree_reading(&td, nv, edge_reading(), Some(&formula), None);
    assert_covers_all_vars(&vtree, nv, "the TD-edge-aligned conversion");
}

#[test]
fn edge_binarization_deterministic() {
    let (formula, td) = hub_of_clusters(8, 5, 4);
    let nv = formula.num_vars;
    let a = td_to_vtree_reading(&td, nv, edge_reading(), Some(&formula), None);
    let b = td_to_vtree_reading(&td, nv, edge_reading(), Some(&formula), None);
    let al: Vec<u32> = a.leaf_bottomup().map(|(_, v)| v.0).collect();
    let bl: Vec<u32> = b.leaf_bottomup().map(|(_, v)| v.0).collect();
    assert_eq!(
        al, bl,
        "the edge-aligned binarization must be deterministic"
    );
}
