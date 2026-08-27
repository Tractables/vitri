//! The TD → vtree conversion, driven through the public entry point and
//! reading vocabulary a caller holding its own decomposition uses.

use crate::cnf::{Clause, CnfFormula, Literal};
use crate::decompose::{
    Binarization, Place, Reading, Root, TreeDecomposition, td_to_vtree_reading,
};
use crate::tests::common::{assert_covers_all_vars, make_formula, make_td};
use crate::tests::score_fixture::vtree_peak_context_width;
use crate::vtree::VarId;

/// Co-occurrence tie-break test.
///
/// TD layout (5 variables: x0..x4, root = bag 0):
///
///   bag 0 (depth 0): {x0, x1, x2}        ← root
///   bag 1 (depth 1): {x0, x3, x4}         ← child of bag 0
///   bag 2 (depth 1): {x0, x1, x2}         ← child of bag 0 (same depth as bag 1)
///
/// x0 appears in all three bags; bags 1 and 2 share depth 1, so `Deepest`
/// assignment alone leaves its bag ambiguous and BFS's last-write-wins would
/// drop it in bag 2.
///
/// Formula: clauses [x0,x3] and [x0,x4], so primal_adj[x0] = {x3,x4}. Bag 1
/// ({x0,x3,x4}) scores 2 (both neighbours present); bag 2 ({x0,x1,x2}) scores
/// 0 (neither present), so the tie-break routes x0 to bag 1 and its leaf lands
/// beside x3 and x4 rather than beside x1 and x2.
#[test]
fn cooc_tiebreak_picks_richer_bag() {
    let formula = make_formula(5, vec![vec![1, 4], vec![1, 5]]);

    let td = make_td(
        vec![vec![0, 1, 2], vec![0, 3, 4], vec![0, 1, 2]],
        vec![(0, 1), (0, 2)],
        5,
    );

    let reading = Reading {
        root: Some(Root::First),
        place: Some(Place::Deep),
        binarize: Some(Binarization::Balanced),
    };
    let vtree = td_to_vtree_reading(&td, 5, reading, Some(&formula), None);
    assert_eq!(vtree.num_leaves(), 5);

    // Bag assignment is internal state, so read it back off the vtree: a bag's
    // variables occupy one contiguous run of leaves, and x0 joins the run
    // holding its clause partners.
    let leaves: Vec<u32> = vtree.leaf_bottomup().map(|(_, v)| v.0).collect();
    let pos = |v: u32| leaves.iter().position(|&l| l == v).unwrap();
    let mut bag1 = [pos(0), pos(3), pos(4)];
    bag1.sort_unstable();
    assert_eq!(
        bag1[2] - bag1[0],
        2,
        "x0 should sit beside x3 and x4 in {leaves:?}",
    );
}

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

fn td_treewidth(td: &TreeDecomposition) -> usize {
    td.treewidth() as usize
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

/// Measurement-only helper (kept `#[ignore]`d): prints prod vs edge peak ctx
/// across a couple of hub shapes. Not an assertion — run with
/// `--ignored --nocapture` when calibrating.
#[test]
#[ignore = "measurement only"]
fn edge_ctx_measurement() {
    for &(h, b, l) in &[(16u32, 24u32, 8u32), (16, 12, 6), (10, 8, 5)] {
        let (formula, td) = hub_of_clusters(h, b, l);
        let nv = formula.num_vars;
        let tw = td_treewidth(&td) as u32;
        let prod = td_to_vtree_reading(&td, nv, Reading::default(), Some(&formula), None);
        let edge = td_to_vtree_reading(&td, nv, edge_reading(), Some(&formula), None);
        eprintln!(
            "hub={h} branches={b} local={l} nv={nv} treewidth={tw} \
             prod_ctx={} edge_ctx={}",
            vtree_peak_context_width(&prod, &formula),
            vtree_peak_context_width(&edge, &formula),
        );
    }
}
