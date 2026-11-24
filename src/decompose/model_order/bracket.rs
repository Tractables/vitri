//! Trees over a leaf order: interval dynamic programs on block readings, and
//! held-out scores of the trees they build.
//!
//! The order is cut into blocks (`reading::block_bounds`), each block a chain
//! of its leaves, and the join DP chooses how to bracket the blocks. With
//! `R(v)` the clipped reading of node v, it minimises the sum of `2^R(v)` over
//! non-root nodes plus `2^(R(l) + R(r))` for every internal node with children
//! l and r.

use crate::decompose::reading::Readings;
use crate::vtree::{VarId, Vtree, VtreeArena, VtreeIdx};

/// Held-out scores leave out nodes of at most this many leaves.
const SCORED_ABOVE: usize = 8;

/// Two join DP sums within this ratio of each other are a tie.
const TIE: f64 = 1e-9;

/// How each interval of blocks `s..=t` splits under the join DP:
/// `split[s * g + t] = k` joins `s..=k` and `k+1..=t`. Ties on the sum go to the lower worst join term,
/// then to the split more even in leaves, then to the leftmost.
pub(super) fn join_split(r: &[f64], g: usize, bounds: &[usize]) -> Vec<usize> {
    let cost: Vec<f64> = r.iter().map(|x| x.exp2()).collect();
    let mut best = vec![0.0; g * g];
    let mut worst = vec![f64::NEG_INFINITY; g * g];
    let mut split = vec![0; g * g];
    for t in 0..g {
        best[t * g + t] = cost[t * g + t];
    }
    let mut sums = Vec::with_capacity(g);
    let mut joins = Vec::with_capacity(g);
    for len in 2..=g {
        for i in 0..=g - len {
            let j = i + len - 1;
            sums.clear();
            joins.clear();
            for k in i..j {
                let jb = r[i * g + k] + r[(k + 1) * g + j];
                sums.push(best[i * g + k] + best[(k + 1) * g + j] + jb.exp2());
                joins.push(worst[i * g + k].max(worst[(k + 1) * g + j]).max(jb));
            }
            let least = sums.iter().copied().fold(f64::INFINITY, f64::min);
            let mut pick: Vec<usize> = (0..sums.len())
                .filter(|&c| sums[c] <= least * (1.0 + TIE))
                .collect();
            if pick.len() > 1 {
                let m = pick.iter().map(|&c| joins[c]).fold(f64::INFINITY, f64::min);
                pick.retain(|&c| joins[c] <= m + TIE);
            }
            if pick.len() > 1 {
                let uneven = |c: usize| {
                    let k = i + c;
                    (bounds[k + 1] - bounds[i]).abs_diff(bounds[j + 1] - bounds[k + 1])
                };
                let m = pick.iter().map(|&c| uneven(c)).min().unwrap();
                pick.retain(|&c| uneven(c) == m);
            }
            let c = pick[0];
            split[i * g + j] = i + c;
            best[i * g + j] = sums[c] + if len < g { cost[i * g + j] } else { 0.0 };
            worst[i * g + j] = joins[c];
        }
    }
    split
}

/// A binary tree over a leaf order, nodes in the order they were made
/// (children first, the root last). Every node covers a run of the order.
#[derive(Clone)]
pub(super) struct Tree {
    pub order: Vec<usize>,
    /// Per node, its children (none for a leaf) and its run `lo..hi`.
    pub kids: Vec<Option<(u32, u32)>>,
    pub run: Vec<(u32, u32)>,
}

impl Tree {
    /// Blocks as right-linear chains, bracketed by `split`.
    pub(super) fn new(order: Vec<usize>, bounds: &[usize], split: &[usize]) -> Self {
        let g = bounds.len() - 1;
        let mut tree = Tree {
            order,
            kids: Vec::new(),
            run: Vec::new(),
        };
        if g > 0 {
            tree.bracket(bounds, split, g, 0, g - 1);
        }
        tree
    }

    /// `vtree` as a tree over its own leaf order, left to right.
    pub(super) fn from_vtree(vtree: &Vtree) -> Self {
        let mut tree = Tree {
            order: Vec::with_capacity(vtree.num_leaves() as usize),
            kids: Vec::with_capacity(vtree.num_nodes()),
            run: Vec::with_capacity(vtree.num_nodes()),
        };
        // Post-order: a node is pushed once both its children are.
        let mut made: Vec<u32> = Vec::new();
        let mut stack = vec![(vtree.root(), false)];
        while let Some((v, expanded)) = stack.pop() {
            if vtree.node(v).is_leaf() {
                let p = tree.order.len() as u32;
                tree.order.push(vtree.leaf_var(v).idx());
                made.push(tree.push(None, (p, p + 1)));
            } else if expanded {
                let r = made.pop().expect("right child made");
                let l = made.pop().expect("left child made");
                let run = (tree.run[l as usize].0, tree.run[r as usize].1);
                made.push(tree.push(Some((l, r)), run));
            } else {
                let (l, r) = vtree.children(v);
                stack.extend([(v, true), (r, false), (l, false)]);
            }
        }
        tree
    }

    fn push(&mut self, kids: Option<(u32, u32)>, run: (u32, u32)) -> u32 {
        self.kids.push(kids);
        self.run.push(run);
        (self.kids.len() - 1) as u32
    }

    fn bracket(&mut self, bounds: &[usize], split: &[usize], g: usize, s: usize, t: usize) -> u32 {
        if s == t {
            let (lo, hi) = (bounds[s] as u32, bounds[s + 1] as u32);
            let leaves: Vec<u32> = (lo..hi).map(|p| self.push(None, (p, p + 1))).collect();
            let mut cur = *leaves.last().unwrap();
            for (&x, p) in leaves.iter().rev().skip(1).zip((lo..hi - 1).rev()) {
                cur = self.push(Some((x, cur)), (p, hi));
            }
            return cur;
        }
        let k = split[s * g + t];
        let a = self.bracket(bounds, split, g, s, k);
        let b = self.bracket(bounds, split, g, k + 1, t);
        self.push(Some((a, b)), (bounds[s] as u32, bounds[t + 1] as u32))
    }

    fn root(&self) -> u32 {
        (self.kids.len() - 1) as u32
    }

    fn leaves(&self, v: u32) -> usize {
        let (lo, hi) = self.run[v as usize];
        (hi - lo) as usize
    }

    /// The held-out score: log2 of the sum, over nodes of more than eight
    /// leaves, of `2^R` for each such node but the root, plus `2^(R(l) +
    /// R(r))` of its children; readings clipped at the sample's cap.
    pub(super) fn score(&self, readings: &mut Readings<'_>) -> f64 {
        let cap = readings.sample().cap();
        let n = self.order.len();
        let mut held: Vec<Option<f64>> = vec![None; self.kids.len()];
        let mut inside = vec![false; n];
        let mut read = |v: u32, readings: &mut Readings<'_>| -> f64 {
            *held[v as usize].get_or_insert_with(|| {
                let (lo, hi) = self.run[v as usize];
                inside.iter_mut().for_each(|b| *b = false);
                for &x in &self.order[lo as usize..hi as usize] {
                    inside[x] = true;
                }
                readings.read(&inside)
            })
        };
        let root = self.root();
        let mut total = 0.0;
        for (v, kids) in self.kids.iter().enumerate() {
            let v = v as u32;
            let Some((l, r)) = *kids else { continue };
            if self.leaves(v) <= SCORED_ABOVE {
                continue;
            }
            if v != root {
                total += read(v, readings).min(cap).exp2();
            }
            let a = read(l, readings).min(cap);
            let b = read(r, readings).min(cap);
            total += (a + b).exp2();
        }
        total.max(1.0).log2()
    }

    /// The tree as a vtree over variables `1..=n`.
    pub(super) fn to_vtree(&self) -> Vtree {
        let mut arena = VtreeArena::with_capacity(self.kids.len());
        let mut idx: Vec<VtreeIdx> = Vec::with_capacity(self.kids.len());
        for (v, kids) in self.kids.iter().enumerate() {
            idx.push(match *kids {
                None => {
                    let var = self.order[self.run[v].0 as usize] as u32 + 1;
                    arena.leaf(VarId::new(var).expect("variable numbers start at 1"))
                }
                Some((l, r)) => arena.internal(idx[l as usize], idx[r as usize]),
            });
        }
        let n = self.order.len() as u32;
        Vtree::from_nodes(arena.into_nodes(), *idx.last().unwrap(), n)
    }

    /// The tree in the text format `vtree N`, then `L id var` and
    /// `I id left right` lines in node order, variables from 1.
    #[cfg(test)]
    pub(super) fn text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = format!("vtree {}\n", self.kids.len());
        for (v, kids) in self.kids.iter().enumerate() {
            match *kids {
                None => writeln!(out, "L {v} {}", self.order[self.run[v].0 as usize] + 1),
                Some((l, r)) => writeln!(out, "I {v} {l} {r}"),
            }
            .unwrap();
        }
        out
    }
}
