//! The `.vtree` text format — the one the SDD library reads and writes.
//!
//! A vtree leaves this crate as text and comes back as text, so both
//! directions live together: what one writes the other has to accept.

use crate::error::VitriError;

use super::{VarId, Vtree, VtreeIdx, VtreeNode};

impl Vtree {
    /// Parse the `.vtree` text format — the one the SDD library reads.
    ///
    /// Format: `vtree N` header, then N lines of `L <id> <var_1indexed>` or `I <id> <left> <right>`.
    /// The last node listed is assumed to be the root.
    ///
    /// # Errors
    ///
    /// [`VitriError::Input`] describing what is wrong with `s`: a missing or
    /// invalid header, a malformed node line, an unparseable id, a node or
    /// variable id outside the range the header declares, a variable carried by
    /// two leaves, or node lines that do not describe a single tree.
    pub fn from_vtree_text(s: &str) -> Result<Self, VitriError> {
        Self::parse_vtree_text(s).map_err(VitriError::input)
    }

    /// The parse itself, reporting a plain sentence. Private: the sentence
    /// becomes a [`VitriError`] at the one public entry point above.
    fn parse_vtree_text(s: &str) -> Result<Self, String> {
        let mut lines = s.lines();

        let header = lines.next().ok_or("empty vtree file")?;
        let n: usize = header
            .strip_prefix("vtree ")
            .ok_or("missing 'vtree N' header")?
            .trim()
            .parse()
            .map_err(|_| "invalid node count in header")?;
        if n == 0 {
            return Err("header declares 0 nodes; a vtree has at least one".to_string());
        }

        // Every id a node line names — the node's own, and an internal node's
        // two children — has to address a node the header declared.
        let check_id = |what: &str, id: usize| -> Result<(), String> {
            if id < n {
                Ok(())
            } else {
                Err(format!(
                    "{what} {id} is out of range for the {n} nodes the header declares"
                ))
            }
        };

        let mut nodes = vec![None; n];
        let mut num_vars: u32 = 0;
        let mut last_id = 0usize;
        // A vtree carries each variable on exactly one leaf. A file naming one
        // twice is caught here rather than left to the leaf-count assertion a
        // consumer of the tree eventually trips over.
        let mut leaf_of_var: std::collections::HashMap<u32, usize> =
            std::collections::HashMap::new();

        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            match parts[0] {
                "L" => {
                    if parts.len() != 3 {
                        return Err(format!("bad leaf line: {}", line));
                    }
                    let id: usize = parts[1]
                        .parse()
                        .map_err(|_| format!("bad id: {}", parts[1]))?;
                    check_id("node id", id)?;
                    let var_1: u32 = parts[2]
                        .parse()
                        .map_err(|_| format!("bad var: {}", parts[2]))?;
                    if var_1 == 0 {
                        return Err(format!(
                            "leaf {id} names variable 0; vtree variables are 1-based"
                        ));
                    }
                    if let Some(first) = leaf_of_var.insert(var_1, id) {
                        return Err(format!(
                            "leaves {first} and {id} both name variable {var_1}; a vtree carries \
                             each variable on exactly one leaf"
                        ));
                    }
                    let var = VarId(var_1 - 1); // SDD format is 1-indexed
                    num_vars = num_vars.max(var_1);
                    nodes[id] = Some(VtreeNode::Leaf { var, parent: None });
                    last_id = id;
                }
                "I" => {
                    if parts.len() != 4 {
                        return Err(format!("bad internal line: {}", line));
                    }
                    let id: usize = parts[1]
                        .parse()
                        .map_err(|_| format!("bad id: {}", parts[1]))?;
                    check_id("node id", id)?;
                    let left: u32 = parts[2]
                        .parse()
                        .map_err(|_| format!("bad left: {}", parts[2]))?;
                    check_id("left child", left as usize)?;
                    let right: u32 = parts[3]
                        .parse()
                        .map_err(|_| format!("bad right: {}", parts[3]))?;
                    check_id("right child", right as usize)?;
                    nodes[id] = Some(VtreeNode::Internal {
                        left: VtreeIdx(left),
                        right: VtreeIdx(right),
                        parent: None,
                    });
                    last_id = id;
                }
                _ => return Err(format!("unknown line type: {}", line)),
            }
        }

        let nodes: Vec<VtreeNode> = nodes
            .into_iter()
            .enumerate()
            .map(|(i, n)| n.ok_or_else(|| format!("missing node {}", i)))
            .collect::<Result<_, _>>()?;

        let root = VtreeIdx(last_id as u32);
        // In-range ids alone still admit a cycle or a second component, either
        // of which the traversals below this point would follow forever. Walk
        // the child edges once from the root: reaching every node exactly once
        // is what makes the node lines a tree.
        let mut seen = vec![false; n];
        let mut stack = vec![root];
        seen[root.idx()] = true;
        let mut reached = 1usize;
        while let Some(idx) = stack.pop() {
            if let VtreeNode::Internal { left, right, .. } = nodes[idx.idx()] {
                for child in [left, right] {
                    if seen[child.idx()] {
                        return Err(format!(
                            "node {} is reachable twice; the node lines are not a tree",
                            child.idx()
                        ));
                    }
                    seen[child.idx()] = true;
                    reached += 1;
                    stack.push(child);
                }
            }
        }
        if reached != n {
            return Err(format!(
                "{} of the {n} declared nodes are unreachable from the root",
                n - reached
            ));
        }

        Ok(Self::from_nodes(nodes, root, num_vars))
    }

    /// Serialize this vtree in the `.vtree` text format — the one the SDD
    /// library reads.
    ///
    /// Nodes appear bottom-up (children before parents), with 1-indexed variable ids.
    /// The output can be loaded by pysdd via `Vtree.from_file(path)`.
    ///
    /// The id printed for a node is its position in [`Vtree::bottomup`], which
    /// the format requires to precede its parent's. On a tree that has been
    /// rotated that position is no longer the node's index in this crate's node
    /// array, so a [`VtreeIdx`] does not survive the round trip; the tree does.
    ///
    /// This is a serialization format, not a canonical form. Every tree this
    /// crate *builds* is numbered by one bottom-up pass over its shape, so among
    /// freshly constructed trees equal text does mean equal tree — but a
    /// rotation renumbers only what it has to, and two trees equal under
    /// [`Vtree::same_tree`] can serialize differently once one of them has been
    /// rotated. Compare trees with `same_tree`.
    pub fn to_vtree_text(&self) -> String {
        let n = self.nodes.len();
        let mut out = format!("vtree {}\n", n);
        for idx in self.bottomup() {
            let id = self.topo_pos[idx.idx()];
            match self.node(idx) {
                VtreeNode::Leaf { var, .. } => {
                    out.push_str(&format!("L {} {}\n", id, var.to_dimacs() as u32));
                }
                VtreeNode::Internal { left, right, .. } => {
                    out.push_str(&format!(
                        "I {} {} {}\n",
                        id,
                        self.topo_pos[left.idx()],
                        self.topo_pos[right.idx()]
                    ));
                }
            }
        }
        out
    }
}
