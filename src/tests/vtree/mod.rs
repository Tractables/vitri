//! The vtree type: its shapes, its file format, and one construction on it.

use crate::vtree::*;

mod equality;
mod file_format;
mod lca;
mod reverse_rightmost_path;
mod rotation_shape;
mod structure;

/// The variables on the leaves under `node`, left to right. A rotation
/// re-brackets a decomposition without disturbing that order, and sorting the
/// same list gives the variable SET a node stands for — what the node IS,
/// independent of the id the tree happened to number it with.
fn leaves_under(vtree: &Vtree, node: VtreeIdx) -> Vec<u32> {
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(cur) = stack.pop() {
        match vtree.node(cur) {
            VtreeNode::Leaf { var, .. } => out.push(var.0),
            VtreeNode::Internal { left, right, .. } => {
                stack.push(*right);
                stack.push(*left);
            }
        }
    }
    out
}
