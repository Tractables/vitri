//! Candidate leaf orders from repair sets.
//!
//! A repair set is a variable u with the set D of other variables that must
//! flip with it. Three weights read the sets: every pair of `{u} + D` gets
//! `1 / |D|` or `1 / |D|^2`, or only the pairs of u with each member of D get
//! `1 / |D|`, each times the set's own scale. A weight gives its Fiedler
//! order, or a recursive seriation that orders the two halves of the Fiedler
//! order again, each facing the side it is tied to.

use super::spectral::{Solve, Weight, fiedler_order};

/// The recursive seriation keeps parts of at most this many variables as
/// they are.
const SERIATION_LEAF: usize = 24;

/// One candidate order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Candidate {
    All,
    Squared,
    RecursiveSquared,
    RecursiveStar,
}

/// Every candidate, in the order ties between them are broken.
pub(super) const CANDIDATES: [Candidate; 4] = [
    Candidate::All,
    Candidate::Squared,
    Candidate::RecursiveSquared,
    Candidate::RecursiveStar,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pairs {
    All,
    Squared,
    Star,
}

impl Candidate {
    fn pairs(self) -> (Pairs, bool) {
        match self {
            Candidate::All => (Pairs::All, false),
            Candidate::Squared => (Pairs::Squared, false),
            Candidate::RecursiveSquared => (Pairs::Squared, true),
            Candidate::RecursiveStar => (Pairs::Star, true),
        }
    }

    /// Whether `self` is the recursive seriation of the weight whose Fiedler
    /// order is `flat`: then `flat`'s order is its first split.
    pub(super) fn refines(self, flat: Candidate) -> bool {
        let (pairs, recursive) = self.pairs();
        recursive && flat.pairs() == (pairs, false)
    }
}

fn weight(pairs: Pairs, n: usize, sets: &[(Vec<u32>, f64)]) -> Weight {
    let mut w = Weight::new(n, pairs == Pairs::Star);
    for (s, scale) in sets.iter().filter(|s| s.0.len() >= 2) {
        let d = (s.len() - 1) as f64;
        let c = if pairs == Pairs::Squared {
            scale / (d * d)
        } else {
            scale / d
        };
        w.push(s.iter().copied(), c);
    }
    w
}

/// The leaf order of `candidate` over `n` variables from the weighted sets
/// (each the centre first, for a repair set the flipped variable, then the
/// others). `flip` takes the other
/// orientation of the first Fiedler vector the order is built from. `first`,
/// for a candidate that [refines](Candidate::refines) another, may give that
/// one's order under the same `flip`, so that its Fiedler vector is not found
/// again. Also returns the slowest Lanczos run.
pub(super) fn order(
    candidate: Candidate,
    n: usize,
    sets: &[(Vec<u32>, f64)],
    flip: bool,
    first: Option<Vec<usize>>,
) -> (Vec<usize>, Solve) {
    let (pairs, recursive) = candidate.pairs();
    let w = weight(pairs, n, sets);
    if !recursive {
        return fiedler_order(&w, flip);
    }
    let mut seriation = Seriation {
        w: &w,
        scale: 1.0 / w.max_entry().max(1e-300),
        flip,
        first,
        slowest: Solve::default(),
        tie: vec![0.0; n],
        inside: vec![0.0; n],
    };
    let idx: Vec<usize> = (0..n).collect();
    let order = seriation.order(&idx, &vec![0.0; n], &vec![0.0; n]);
    (order, seriation.slowest)
}

struct Seriation<'a> {
    w: &'a Weight,
    /// One over the largest pair weight, for the ties to each side.
    scale: f64,
    /// The orientation of the first Fiedler vector, cleared once used.
    flip: bool,
    /// The first Fiedler order when it is given, taken once used.
    first: Option<Vec<usize>>,
    slowest: Solve,
    tie: Vec<f64>,
    inside: Vec<f64>,
}

impl Seriation<'_> {
    /// Order `idx`, given each variable's tie to what lies left of `idx`
    /// (`left`) and right of it (`right`).
    fn order(&mut self, idx: &[usize], left: &[f64], right: &[f64]) -> Vec<usize> {
        if idx.len() <= SERIATION_LEAF {
            return idx.to_vec();
        }
        let flip = std::mem::take(&mut self.flip);
        let mut o = match self.first.take() {
            Some(first) => first,
            None => {
                let (local, solve) = fiedler_order(&self.w.restrict(idx), flip);
                if solve.steps > self.slowest.steps {
                    self.slowest = solve;
                }
                local.iter().map(|&l| idx[l]).collect()
            }
        };
        let mid = (o.len() - 1) as f64 / 2.0;
        let facing: f64 = o
            .iter()
            .enumerate()
            .map(|(i, &v)| (i as f64 - mid) * (right[v] - left[v]))
            .sum();
        if facing < 0.0 {
            o.reverse();
        }
        let (l, r) = o.split_at(o.len() / 2);
        let to_r = self.ties(r);
        let to_l = self.ties(l);
        let right_of_l: Vec<f64> = right.iter().zip(&to_r).map(|(a, b)| a + b).collect();
        let left_of_r: Vec<f64> = left.iter().zip(&to_l).map(|(a, b)| a + b).collect();
        let mut out = self.order(l, left, &right_of_l);
        out.extend(self.order(r, &left_of_r, right));
        out
    }

    /// Each variable's weight to the variables of `part`, scaled to a
    /// largest pair weight of 1.
    fn ties(&mut self, part: &[usize]) -> Vec<f64> {
        self.inside.iter_mut().for_each(|x| *x = 0.0);
        for &v in part {
            self.inside[v] = 1.0;
        }
        self.w.apply(&self.inside, &mut self.tie);
        self.tie.iter().map(|x| x * self.scale).collect()
    }
}
