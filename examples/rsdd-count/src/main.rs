use std::{env, error::Error, fs, io::Cursor};

use rsdd::{
    builder::{BottomUpBuilder, sdd::CompressionSddBuilder},
    repr::{Cnf, DDNNFPtr, Literal, VTree, VarLabel, WmcParams},
    util::semirings::RealSemiring,
};
use vitri::{
    CnfFormula,
    vtree::{Vtree, VtreeIdx, VtreeNode},
};

fn to_rsdd(tree: &Vtree, node: VtreeIdx) -> VTree {
    match tree.node(node) {
        VtreeNode::Leaf { var, .. } => VTree::new_leaf(VarLabel::new_usize(var.idx())),
        VtreeNode::Internal { left, right, .. } => VTree::new_node(
            Box::new(to_rsdd(tree, *left)),
            Box::new(to_rsdd(tree, *right)),
        ),
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: vitri-rsdd-count-example <reduced.cnf> <vtree.vtree>".into());
    }
    let cnf_text = fs::read_to_string(&args[0])?;
    let (formula, meta) = CnfFormula::from_dimacs(Cursor::new(&cnf_text))?;
    if !matches!(
        meta.mode(),
        vitri::cnf::Mode::Mc | vitri::cnf::Mode::Compile
    ) || meta.declared_show_vars().is_some()
        || meta.declared_weights().is_some()
    {
        return Err("this example counts unweighted, unprojected CNFs only".into());
    }
    let tree = Vtree::from_vtree_text(&fs::read_to_string(&args[1])?)?;
    if tree.num_vars() != formula.num_vars || tree.num_leaves() != formula.num_vars {
        return Err("the vtree must contain every CNF variable exactly once".into());
    }
    // Normalized dyadic weights keep all intermediate values exact in f64
    // within this bound; larger inputs need a different counting arithmetic.
    if formula.num_vars > 52 {
        return Err("this counting example supports at most 52 variables".into());
    }
    let builder = CompressionSddBuilder::new(to_rsdd(&tree, tree.root()));
    let clauses: Vec<Vec<Literal>> = formula
        .clauses
        .iter()
        .map(|clause| {
            clause
                .iter()
                .map(|lit| Literal::new(VarLabel::new_usize(lit.var.idx()), lit.positive))
                .collect()
        })
        .collect();
    let circuit = builder.compile_cnf(&Cnf::new(&clauses));
    let mut weights = WmcParams::default();
    for var in 0..formula.num_vars {
        weights.set_weight(
            VarLabel::new(var.into()),
            RealSemiring(0.5),
            RealSemiring(0.5),
        );
    }
    // Weights summing to one account for variables absent from a circuit branch.
    let probability = circuit.unsmoothed_wmc(&weights).0;
    let count = probability * 2_f64.powi(formula.num_vars as i32);
    println!("Reduced count: {count:.0}");
    Ok(())
}
