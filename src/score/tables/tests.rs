//! The split and cut quantities against the numbers the Python tools the
//! aggregate ranker was fitted from report for the same (CNF, vtree) pair.

use super::{Feature, Tables};
use crate::cnf::CnfFormula;
use crate::vtree::Vtree;

/// The vtree file's node ids, one per vitri node index.
///
/// Parsing a vtree renumbers its nodes, so a dump keyed by the file's ids has
/// to map back. The two trees have the same shape, so one walk from the two
/// roots pairs them.
fn file_ids(text: &str, vtree: &Vtree) -> Vec<u32> {
    let mut children: Vec<Option<(u32, u32)>> = Vec::new();
    let mut root = 0u32;
    for line in text.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }
        let id: usize = parts[1].parse().expect("a node id");
        if children.len() <= id {
            children.resize(id + 1, None);
        }
        children[id] = match parts[0] {
            "I" => Some((
                parts[2].parse().expect("a left child"),
                parts[3].parse().expect("a right child"),
            )),
            _ => None,
        };
        root = id as u32;
    }
    let mut ids = vec![u32::MAX; vtree.num_nodes()];
    let mut stack = vec![(root, vtree.root())];
    while let Some((file, node)) = stack.pop() {
        ids[node.idx()] = file;
        if let Some((file_left, file_right)) = children[file as usize] {
            let (left, right) = vtree.children(node);
            stack.push((file_left, left));
            stack.push((file_right, right));
        }
    }
    assert!(
        ids.iter().all(|&id| id != u32::MAX),
        "the file and the parsed tree have the same shape"
    );
    ids
}

const SPLIT_CNF: &str = "\
p cnf 8 10
1 5 0
1 5 0
-1 5 0
1 -5 0
2 6 0
3 7 0
1 2 5 0
1 2 0
1 3 0
5 6 0
";

const SPLIT_VTREE: &str = "\
vtree 15
L 0 1
L 1 2
L 2 3
L 3 4
L 4 5
L 5 6
L 6 7
L 7 8
I 8 0 1
I 9 2 3
I 10 8 9
I 11 4 5
I 12 6 7
I 13 11 12
I 14 10 13
";

/// What `vtree_metrics.py --nodes` and `cutnodes.py` report for that tree, in
/// the order [`SPLIT_COLUMNS`] names.
const SPLIT_COLUMNS: [&str; 9] = [
    "local_join_density_subtree",
    "local_join_density_total",
    "signed_split_distinct",
    "unsigned_split_distinct",
    "signed_split_entropy_bits",
    "cutrank",
    "twin_in",
    "twin_out",
    "below",
];

/// Every internal node of the pair. Node 14 is the root, which `cutnodes.py`
/// skips because every variable is below it: the fit read zeros for its four
/// cut columns, and so does this crate. Nodes 9, 12 and 13 carry no clause, so
/// their five split columns are zero while their cut is not.
const SPLIT_EXPECTED: [(u32, [f64; 9]); 7] = [
    (
        14,
        [
            2.1,
            2.1,
            6.0,
            4.0,
            2.521_640_636_343_318,
            0.0,
            0.0,
            0.0,
            0.0,
        ],
    ),
    (13, [0.0, 0.0, 0.0, 0.0, 0.0, 3.0, 3.0, 3.0, 4.0]),
    (12, [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0]),
    (11, [1.0, 0.1, 1.0, 1.0, 0.0, 2.0, 2.0, 2.0, 2.0]),
    (10, [0.5, 0.1, 1.0, 1.0, 0.0, 3.0, 3.0, 3.0, 4.0]),
    (9, [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0]),
    (8, [1.0, 0.1, 1.0, 1.0, 0.0, 2.0, 2.0, 3.0, 2.0]),
];

/// Every split and cut quantity, at every internal node, against the numbers
/// the Python tools the model was fitted from report for the same pair.
#[test]
fn the_split_and_cut_quantities_are_the_tool_s() {
    let (formula, _) = CnfFormula::from_dimacs(std::io::Cursor::new(SPLIT_CNF.as_bytes()))
        .expect("the pair's CNF parses");
    let vtree = Vtree::from_vtree_text(SPLIT_VTREE).expect("the pair's vtree parses");
    let ids = file_ids(SPLIT_VTREE, &vtree);
    let tables = Tables::build(&vtree, &formula, true, true);
    let mut checked = 0usize;
    for (node, left, right) in vtree.internal_bottomup() {
        let Some((_, expected)) = SPLIT_EXPECTED.iter().find(|(id, _)| *id == ids[node.idx()])
        else {
            continue;
        };
        for (name, want) in SPLIT_COLUMNS.iter().zip(expected) {
            let feature = Feature::from_name(name).expect("a quantity this crate computes");
            let got = tables.value(feature, node, left, right);
            assert!(
                (got - want).abs() <= 1e-12 * want.abs().max(1.0),
                "node {} {name}: {got} against the tool's {want}",
                ids[node.idx()],
            );
        }
        checked += 1;
    }
    assert_eq!(
        checked,
        SPLIT_EXPECTED.len(),
        "every listed node is in the tree"
    );
}
