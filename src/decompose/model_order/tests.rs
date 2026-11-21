use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use super::bracket::{Tree, join_split};
use super::orders::{CANDIDATES, order};
use super::spectral::{Weight, fiedler_order};
use super::*;
use crate::decompose::reading::block_bounds;

fn wall(secs: u64) -> Allowance {
    let nominal = Duration::from_secs(secs);
    Allowance::of_run(PreprocessClock::WallClock, nominal, nominal, None)
}

fn random_sets(rng: &mut StdRng, n: usize, count: usize, max: usize) -> Vec<Vec<u32>> {
    (0..count)
        .map(|_| {
            let mut vars: Vec<u32> = (0..n as u32).collect();
            let len = rng.random_range(1..=max.min(n));
            for i in 0..len {
                let j = rng.random_range(i..n);
                vars.swap(i, j);
            }
            vars.truncate(len);
            vars
        })
        .collect()
}

/// Repair sets with a weight each; with `chain`, also every pair of
/// neighbouring variables, so that no variable is left out.
fn weighted(
    rng: &mut StdRng,
    n: usize,
    count: usize,
    max: usize,
    chain: bool,
) -> Vec<(Vec<u32>, f64)> {
    let mut out: Vec<(Vec<u32>, f64)> = random_sets(rng, n, count, max)
        .into_iter()
        .map(|s| (s, rng.random::<f64>() + 0.1))
        .collect();
    if chain {
        out.extend((1..n as u32).map(|v| (vec![v - 1, v], 0.05 * rng.random::<f64>())));
    }
    out
}

/// The weight as a dense matrix, each entry summed over the terms in order.
fn dense(sets: &[(Vec<u32>, f64)], n: usize, star: bool) -> Vec<Vec<f64>> {
    let mut m = vec![vec![0.0; n]; n];
    for (s, c) in sets.iter().filter(|s| s.0.len() >= 2) {
        if star {
            for &d in &s[1..] {
                m[s[0] as usize][d as usize] += c;
                m[d as usize][s[0] as usize] += c;
            }
        } else {
            for &a in s {
                for &b in s {
                    if a != b {
                        m[a as usize][b as usize] += c;
                    }
                }
            }
        }
    }
    m
}

fn weight_of(sets: &[(Vec<u32>, f64)], n: usize, star: bool) -> Weight {
    let mut w = Weight::new(n, star);
    for (s, c) in sets {
        w.push(s.iter().copied(), *c);
    }
    w
}

#[test]
fn weight_products_match_the_dense_matrix() {
    let mut rng = StdRng::seed_from_u64(3);
    for trial in 0..40 {
        let n = rng.random_range(2..30);
        let count = rng.random_range(1..20);
        let sets = weighted(&mut rng, n, count, 8, false);
        let star = trial % 2 == 1;
        let m = dense(&sets, n, star);
        let w = weight_of(&sets, n, star);
        let z: Vec<f64> = (0..n).map(|_| rng.random::<f64>() - 0.5).collect();
        let mut y = vec![0.0; n];
        w.apply(&z, &mut y);
        for a in 0..n {
            let want: f64 = (0..n).map(|b| m[a][b] * z[b]).sum();
            assert!((y[a] - want).abs() < 1e-12, "trial {trial}");
        }
        let max = m.iter().flatten().copied().fold(0.0, f64::max);
        assert_eq!(w.max_entry(), max, "trial {trial}");
        let keep: Vec<usize> = (0..n).filter(|_| rng.random::<bool>()).collect();
        let sub = w.restrict(&keep);
        let ones = vec![1.0; keep.len()];
        let mut y = vec![0.0; keep.len()];
        sub.apply(&ones, &mut y);
        for (i, &a) in keep.iter().enumerate() {
            let want: f64 = keep.iter().map(|&b| m[a][b]).sum();
            assert!((y[i] - want).abs() < 1e-12, "trial {trial}");
        }
    }
}

/// Eigenpairs of a small symmetric matrix by cyclic Jacobi rotations,
/// eigenvalues ascending, eigenvectors as columns of the returned rows.
fn jacobi(mut a: Vec<Vec<f64>>) -> (Vec<f64>, Vec<Vec<f64>>) {
    let n = a.len();
    let mut v: Vec<Vec<f64>> = (0..n)
        .map(|i| (0..n).map(|j| f64::from(u8::from(i == j))).collect())
        .collect();
    for _ in 0..100 {
        let off: f64 = (0..n)
            .flat_map(|p| (p + 1..n).map(move |q| (p, q)))
            .map(|(p, q)| a[p][q] * a[p][q])
            .sum();
        if off < 1e-30 {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                if a[p][q].abs() < 1e-300 {
                    continue;
                }
                let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let (c, s) = (1.0 / (t * t + 1.0).sqrt(), t / (t * t + 1.0).sqrt());
                for row in a.iter_mut() {
                    let (akp, akq) = (row[p], row[q]);
                    row[p] = c * akp - s * akq;
                    row[q] = s * akp + c * akq;
                }
                let (upper, lower) = a.split_at_mut(q);
                for (x, y) in upper[p].iter_mut().zip(lower[0].iter_mut()) {
                    let (apk, aqk) = (*x, *y);
                    *x = c * apk - s * aqk;
                    *y = s * apk + c * aqk;
                }
                for row in v.iter_mut() {
                    let (vp, vq) = (row[p], row[q]);
                    row[p] = c * vp - s * vq;
                    row[q] = s * vp + c * vq;
                }
            }
        }
    }
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&i, &j| a[i][i].total_cmp(&a[j][j]));
    let vals = idx.iter().map(|&i| a[i][i]).collect();
    let vecs = idx
        .iter()
        .map(|&i| v.iter().map(|r| r[i]).collect())
        .collect();
    (vals, vecs)
}

/// The Fiedler order from the dense normalised Laplacian, as the reference
/// code computes it, in either orientation.
fn dense_fiedler(m: &[Vec<f64>]) -> Vec<usize> {
    let n = m.len();
    let max = m.iter().flatten().copied().fold(0.0, f64::max).max(1e-300);
    let w: Vec<Vec<f64>> = (0..n)
        .map(|i| {
            (0..n)
                .map(|j| if i == j { 0.0 } else { m[i][j] / max + 1e-6 })
                .collect()
        })
        .collect();
    let d: Vec<f64> = w.iter().map(|r| r.iter().sum()).collect();
    let lap: Vec<Vec<f64>> = (0..n)
        .map(|i| {
            (0..n)
                .map(|j| f64::from(u8::from(i == j)) - w[i][j] / d[i].sqrt() / d[j].sqrt())
                .collect()
        })
        .collect();
    let (_, vecs) = jacobi(lap);
    let v: Vec<f64> = (0..n).map(|i| vecs[1][i] / d[i].sqrt()).collect();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| v[a].total_cmp(&v[b]));
    order
}

#[test]
fn lanczos_finds_the_dense_fiedler_order() {
    let mut rng = StdRng::seed_from_u64(5);
    let mut checked = 0;
    for trial in 0..60 {
        let n = rng.random_range(3..40);
        let count = rng.random_range(n..4 * n);
        let sets = weighted(&mut rng, n, count, 6, true);
        let star = trial % 3 == 2;
        let m = dense(&sets, n, star);
        let (got, solve) = fiedler_order(&weight_of(&sets, n, star), false);
        let want = dense_fiedler(&m);
        let mut rev = want.clone();
        rev.reverse();
        assert!(
            got == want || got == rev,
            "trial {trial} n {n}: {got:?} {want:?}"
        );
        assert!(solve.steps < n, "trial {trial}");
        let (flipped, _) = fiedler_order(&weight_of(&sets, n, star), true);
        let mut back = flipped.clone();
        back.reverse();
        assert_eq!(back, got, "trial {trial}");
        checked += 1;
    }
    assert_eq!(checked, 60);
}

#[test]
fn orders_cover_every_variable() {
    let mut rng = StdRng::seed_from_u64(7);
    let n = 120;
    let sets: Vec<(Vec<u32>, f64)> = random_sets(&mut rng, n, 400, 40)
        .into_iter()
        .map(|s| (s, 1.0))
        .collect();
    for c in CANDIDATES {
        let (o, _) = order(c, n, &sets, false, None);
        let mut s = o.clone();
        s.sort_unstable();
        assert_eq!(s, (0..n).collect::<Vec<_>>(), "{c:?}");
    }
    // A recursive order from its flat partner's order is the one found alone.
    let flat = order(Candidate::Squared, n, &sets, false, None).0;
    assert!(Candidate::RecursiveSquared.refines(Candidate::Squared));
    assert_eq!(
        order(Candidate::RecursiveSquared, n, &sets, false, Some(flat)).0,
        order(Candidate::RecursiveSquared, n, &sets, false, None).0
    );
}

/// The join sum of every bracketing of blocks `s..=t`.
fn brackets(r: &[f64], g: usize, s: usize, t: usize) -> Vec<f64> {
    let cost = |a: usize, b: usize| r[a * g + b].exp2();
    if s == t {
        return vec![cost(s, s)];
    }
    let mut out = Vec::new();
    for k in s..t {
        for &a in &brackets(r, g, s, k) {
            for &b in &brackets(r, g, k + 1, t) {
                let join = (r[s * g + k] + r[(k + 1) * g + t]).exp2();
                out.push(a + b + join + cost(s, t));
            }
        }
    }
    out
}

/// The join sum of the bracketing `split` gives.
fn sums(r: &[f64], g: usize, split: &[usize], s: usize, t: usize) -> f64 {
    let cost = r[s * g + t].exp2();
    if s == t {
        return cost;
    }
    let k = split[s * g + t];
    let join = (r[s * g + k] + r[(k + 1) * g + t]).exp2();
    sums(r, g, split, s, k) + sums(r, g, split, k + 1, t) + join + cost
}

#[test]
fn the_join_dp_finds_the_least_bracketing() {
    let mut rng = StdRng::seed_from_u64(11);
    for trial in 0..200 {
        let g = rng.random_range(1..7);
        let mut r = vec![0.0; g * g];
        for s in 0..g {
            for t in s..g {
                if !(s == 0 && t == g - 1) {
                    r[s * g + t] = f64::from(rng.random_range(0..12u8)) / 2.0;
                }
            }
        }
        let bounds: Vec<usize> = (0..=g).map(|i| 4 * i).collect();
        let all = if g == 1 {
            vec![r[0].exp2()]
        } else {
            brackets(&r, g, 0, g - 1)
        };
        // The root's own reading is 0 and not paid.
        let least = all.iter().copied().fold(f64::INFINITY, f64::min);
        let j = sums(&r, g, &join_split(&r, g, &bounds), 0, g - 1);
        assert!(j <= least * (1.0 + 1e-6), "trial {trial}");
    }
}

#[test]
fn trees_are_vtrees_over_the_order() {
    let order: Vec<usize> = vec![4, 0, 3, 1, 2, 8, 5, 7, 6];
    let bounds = block_bounds(order.len());
    assert_eq!(bounds, vec![0, 4, 9]);
    let tree = Tree::new(order, &bounds, &[0, 0, 0, 0]);
    assert_eq!(
        tree.text(),
        "vtree 17\nL 0 5\nL 1 1\nL 2 4\nL 3 2\nI 4 2 3\nI 5 1 4\nI 6 0 5\nL 7 3\nL 8 9\n\
         L 9 6\nL 10 8\nL 11 7\nI 12 10 11\nI 13 9 12\nI 14 8 13\nI 15 7 14\nI 16 6 15\n"
    );
    let v = tree.to_vtree();
    assert_eq!(v.num_leaves(), 9);
    assert_eq!(v.num_nodes(), 17);
}

/// A random 3-CNF with a planted model, so that it is satisfiable.
fn planted(rng: &mut StdRng, n: usize, m: usize) -> CnfFormula {
    let truth: Vec<bool> = (0..n).map(|_| rng.random()).collect();
    let mut text = format!("p cnf {n} {m}\n");
    for _ in 0..m {
        loop {
            let lits: Vec<i32> = (0..3)
                .map(|_| {
                    let v = rng.random_range(1..=n as i32);
                    if rng.random() { v } else { -v }
                })
                .collect();
            if lits
                .iter()
                .any(|&l| truth[l.unsigned_abs() as usize - 1] == (l > 0))
            {
                for l in lits {
                    text.push_str(&format!("{l} "));
                }
                text.push_str("0\n");
                break;
            }
        }
    }
    CnfFormula::from_dimacs(std::io::Cursor::new(text))
        .unwrap()
        .0
}

/// A 3-CNF with a planted model whose clauses each fall in a window of three
/// consecutive variables of a hidden order, so a tree along that order is
/// narrow and a tree that ignores it is not.
fn banded(rng: &mut StdRng, n: usize, m: usize) -> CnfFormula {
    let mut hidden: Vec<i32> = (1..=n as i32).collect();
    for i in (1..n).rev() {
        hidden.swap(i, rng.random_range(0..=i));
    }
    let truth: Vec<bool> = (0..n).map(|_| rng.random()).collect();
    let mut text = format!("p cnf {n} {m}\n");
    for _ in 0..m {
        loop {
            let start = rng.random_range(0..n - 2);
            let lits: Vec<i32> = hidden[start..start + 3]
                .iter()
                .map(|&v| if rng.random() { v } else { -v })
                .collect();
            if lits
                .iter()
                .any(|&l| truth[l.unsigned_abs() as usize - 1] == (l > 0))
            {
                for l in lits {
                    text.push_str(&format!("{l} "));
                }
                text.push_str("0\n");
                break;
            }
        }
    }
    CnfFormula::from_dimacs(std::io::Cursor::new(text))
        .unwrap()
        .0
}

#[test]
fn build_gives_a_tree_or_none_when_out_of_allowance() {
    let mut rng = StdRng::seed_from_u64(13);
    let formula = planted(&mut rng, 90, 300);
    let tree = build(&formula, &wall(60)).expect("a tree");
    assert_eq!(tree.num_leaves(), 90);
    assert!(build(&formula, &wall(0)).is_none());
}

#[test]
fn a_build_repairs_after_the_blend_and_picks_from_both() {
    let mut rng = StdRng::seed_from_u64(13);
    let formula = planted(&mut rng, 90, 300);
    let mut spend = Spend::new(&wall(60));
    let learned = learn(&formula, DRAW, true, None, &mut spend).expect("a tree");
    assert_eq!(learned.tree.order.len(), 90);
    assert!(spend.stages.pairs > 0 && spend.stages.minimal > 0);
    assert!(spend.stages.source.is_some());
    // With no room above the reserve, no repair runs and the blend's tree
    // stands.
    let train = draw(&formula, BUILD_SEED, BUILD_DRAW, 0, &mut spend, 0).expect("models");
    let mut tight = Spend::new(&wall(60));
    let sets = repairs_within(&formula, &train, Duration::from_secs(60), &mut tight);
    assert!(sets.is_empty() && tight.stages.pairs <= 1);
}

#[test]
fn the_gap_after_the_blend_stops_the_build_or_lets_the_repairs_run() {
    let mut rng = StdRng::seed_from_u64(13);
    let formula = planted(&mut rng, 90, 300);
    // On a random 3-CNF the balanced tree leaves the blend tree behind, and
    // the build stops before the repairs; on a banded formula a random tree
    // leaves it ahead, and the repairs run.
    let banded = banded(&mut rng, 90, 300);
    let ahead = pick(&banded, &random_vtree(&mut rng, 90), DRAW, true, &wall(60));
    let gap = ahead.stages.gap.expect("a gap with a portfolio tree");
    let omega = ahead.stages.omega.expect("Omegas with a gap");
    assert!(gap <= GAP_STOP && omega_within(omega.0, omega.1) && !ahead.stages.stopped);
    assert!(ahead.stages.pairs > 0 && ahead.figures.is_some());
    let behind = pick(&formula, &Vtree::balanced(90), DRAW, true, &wall(60));
    let gap = (behind.stages.gap)
        .or(behind.stages.early.map(|(g, _)| g))
        .expect("a gap with a portfolio tree");
    assert!(gap > GAP_STOP && behind.stages.stopped);
    assert!(behind.stages.pairs == 0 && behind.figures.is_none() && behind.tree.is_none());
    let mut spend = Spend::new(&wall(60));
    learn(&banded, DRAW, true, None, &mut spend).expect("a tree");
    assert!(spend.stages.gap.is_none() && !spend.stages.stopped);
}

#[test]
fn a_blend_tree_past_the_omega_margin_goes_on_without_repairs() {
    // Within the margin, the plan's repair flag and its gap line stand.
    assert_eq!(after_gap(true, (10, 10)), (GAP_STOP, true));
    assert_eq!(
        after_gap(true, (10 + OMEGA_MARGIN - 1, 10)),
        (GAP_STOP, true)
    );
    assert_eq!(after_gap(false, (10, 10)), (-S_MARGIN, false));
    // At the margin or over it, no repairs and the line without them.
    assert_eq!(after_gap(true, (10 + OMEGA_MARGIN, 10)), (-S_MARGIN, false));
    assert_eq!(after_gap(true, (63, 15)), (-S_MARGIN, false));
    assert_eq!(after_gap(false, (63, 15)), (-S_MARGIN, false));
}

#[test]
fn without_repairs_the_blend_tree_alone_meets_the_take_line() {
    let mut rng = StdRng::seed_from_u64(13);
    let formula = planted(&mut rng, 90, 300);
    let banded = banded(&mut rng, 90, 300);
    let ahead = pick(&banded, &random_vtree(&mut rng, 90), DRAW, false, &wall(60));
    let gap = ahead.stages.gap.expect("a gap with a portfolio tree");
    assert!(gap <= -S_MARGIN && !ahead.stages.stopped);
    assert!(ahead.stages.pairs == 0 && ahead.figures.is_some());
    assert_eq!(ahead.stages.source, Some(Source::Blend));
    let behind = pick(&formula, &Vtree::balanced(90), DRAW, false, &wall(60));
    let gap = (behind.stages.gap)
        .or(behind.stages.early.map(|(g, _)| g))
        .expect("a gap with a portfolio tree");
    assert!(gap > -S_MARGIN && behind.stages.stopped);
    assert!(behind.figures.is_none() && behind.tree.is_none());
}

#[test]
fn a_blend_tree_far_behind_on_the_building_draw_stops_before_the_held_out_draw() {
    let mut rng = StdRng::seed_from_u64(13);
    let formula = planted(&mut rng, 90, 300);
    let banded = banded(&mut rng, 90, 300);
    let ahead = pick(&banded, &random_vtree(&mut rng, 90), DRAW, false, &wall(60));
    let (early, at) = ahead
        .stages
        .early
        .expect("an early gap with a portfolio tree");
    assert!(early <= EARLY_STOP && at <= ahead.spent);
    assert!(ahead.stages.draws[1] > Duration::ZERO && ahead.figures.is_some());
    let behind = pick(&formula, &Vtree::balanced(90), DRAW, false, &wall(60));
    let (early, _) = behind
        .stages
        .early
        .expect("an early gap with a portfolio tree");
    assert_eq!(early > EARLY_STOP, behind.stages.draws[1].is_zero());
    assert!(behind.stages.stopped && behind.tree.is_none());
    let mut spend = Spend::new(&wall(60));
    learn(&banded, DRAW, false, None, &mut spend).expect("a tree");
    assert!(spend.stages.early.is_none());
}

/// A random binary tree over variables `1..=n`.
fn random_vtree(rng: &mut StdRng, n: u32) -> Vtree {
    Vtree::random(n, rng.random())
}

/// The orphans of every internal node, by the definition.
fn omega_by_definition(tree: &Tree, clauses: &[Vec<i32>]) -> usize {
    let mut most = 0;
    for (v, kids) in tree.kids.iter().enumerate() {
        if kids.is_none() {
            continue;
        }
        let (lo, hi) = tree.run[v];
        let inside: Vec<usize> = tree.order[lo as usize..hi as usize].to_vec();
        let held: std::collections::HashSet<usize> = clauses
            .iter()
            .filter(|c| {
                c.iter()
                    .all(|l| inside.contains(&(l.unsigned_abs() as usize - 1)))
            })
            .flat_map(|c| c.iter().map(|l| l.unsigned_abs() as usize - 1))
            .collect();
        most = most.max(inside.iter().filter(|x| !held.contains(x)).count());
    }
    most
}

#[test]
fn omega_counts_variables_in_no_clause_inside_a_node() {
    // (x1 v x2)(x3 v x4): over ((1 3)(2 4)) each child of the root has two
    // orphans, over ((1 2)(3 4)) none has any.
    let clauses = vec![vec![1, 2], vec![3, 4]];
    let list = ClauseList::new(&clauses);
    let pairs = |a: [usize; 2], b: [usize; 2]| {
        let mut arena = crate::vtree::VtreeArena::with_capacity(7);
        let mut leaf = |v: usize| arena.leaf(crate::vtree::VarId::new(v as u32).unwrap());
        let (a0, a1, b0, b1) = (leaf(a[0]), leaf(a[1]), leaf(b[0]), leaf(b[1]));
        let l = arena.internal(a0, a1);
        let r = arena.internal(b0, b1);
        let root = arena.internal(l, r);
        Vtree::from_nodes(arena.into_nodes(), root, 4)
    };
    let crossed = Tree::from_vtree(&pairs([1, 3], [2, 4]));
    assert_eq!(figures::omega(&crossed, &list), 2);
    let aligned = Tree::from_vtree(&pairs([1, 2], [3, 4]));
    assert_eq!(figures::omega(&aligned, &list), 0);
}

#[test]
fn omega_and_static_width_match_their_definitions_on_random_trees() {
    let mut rng = StdRng::seed_from_u64(29);
    for round in 0..40 {
        let n = rng.random_range(2..30u32);
        let m = rng.random_range(1..40);
        let clauses: Vec<Vec<i32>> = (0..m)
            .map(|_| {
                let len = rng.random_range(1..=4.min(n as usize));
                (0..len)
                    .map(|_| {
                        let v = rng.random_range(1..=n as i32);
                        if rng.random() { v } else { -v }
                    })
                    .collect()
            })
            .collect();
        let list = ClauseList::new(&clauses);
        let vtree = random_vtree(&mut rng, n);
        let tree = Tree::from_vtree(&vtree);
        assert!(tree.to_vtree().same_tree(&vtree), "round {round}");
        assert_eq!(
            figures::omega(&tree, &list),
            omega_by_definition(&tree, &clauses),
            "round {round}"
        );
        let mut side = vec![0u8; n as usize];
        for (lo, hi) in &tree.run {
            let mut inside = vec![false; n as usize];
            for &x in &tree.order[*lo as usize..*hi as usize] {
                inside[x] = true;
            }
            let var = |l: &i32| l.unsigned_abs() as usize - 1;
            let crossing: Vec<&Vec<i32>> = clauses
                .iter()
                .filter(|c| c.iter().any(|l| inside[var(l)]) && !c.iter().all(|l| inside[var(l)]))
                .collect();
            let open = |side: bool| {
                let vars: std::collections::HashSet<usize> = crossing
                    .iter()
                    .flat_map(|c| c.iter().map(var))
                    .filter(|&x| inside[x] == side)
                    .collect();
                vars.len()
            };
            let want = crossing.len().min(open(true)).min(open(false));
            assert_eq!(
                figures::static_width(&list, &inside, &mut side),
                want,
                "round {round}"
            );
            assert!(side.iter().all(|&s| s == 0));
        }
    }
}

#[test]
fn pick_scores_both_trees_inside_the_allowance_or_gives_none() {
    let mut rng = StdRng::seed_from_u64(13);
    let formula = banded(&mut rng, 90, 300);
    let portfolio = random_vtree(&mut rng, 90);
    let verdict = pick(&formula, &portfolio, DRAW, true, &wall(60));
    let (mine, theirs) = verdict.figures.expect("both trees scored");
    assert!(mine.s.is_finite() && theirs.s.is_finite());
    let takes = mine.s <= theirs.s - S_MARGIN && omega_within(mine.omega, theirs.omega);
    assert_eq!(verdict.tree.is_some(), takes);
    if let Some(tree) = verdict.tree {
        assert_eq!(tree.num_leaves(), 90);
    }
    let out = pick(&formula, &portfolio, DRAW, true, &wall(0));
    assert!(out.tree.is_none() && out.figures.is_none());
    let small = pick(&formula, &portfolio, MIN_DRAW, true, &wall(60));
    assert!(small.figures.is_some() && !small.stages.stopped);
}

#[test]
fn under_charged_work_the_build_ends_at_the_run_deadline() {
    let mut rng = StdRng::seed_from_u64(13);
    let formula = banded(&mut rng, 90, 300);
    let portfolio = random_vtree(&mut rng, 90);
    let clock = PreprocessClock::Deterministic {
        configured_wall_ms: None,
    };
    let nominal = Duration::from_secs(60);
    let charged = Allowance::of_run(clock, nominal, nominal, None);
    let within = pick(&formula, &portfolio, DRAW, true, &charged);
    assert!(within.figures.is_some());
    let passed = Allowance {
        deadline: Some(Instant::now()),
        ..charged
    };
    let out = pick(&formula, &portfolio, DRAW, true, &passed);
    assert!(out.tree.is_none() && out.figures.is_none());
}

#[test]
fn the_prior_needs_no_draw_and_counts_the_widest_node() {
    let mut rng = StdRng::seed_from_u64(13);
    let formula = banded(&mut rng, 90, 300);
    let portfolio = random_vtree(&mut rng, 90);
    let p = prior(&formula, &portfolio);
    // The widest node alone puts 2^W in the sum.
    assert!(p.width > 0);
    assert!(p.s_bound >= p.width as f64 * 9.0 / 8.0 - 1e-9);
}

#[test]
fn the_allowance_follows_the_payoff_and_skips_a_build_that_does_not_fit() {
    let mut rng = StdRng::seed_from_u64(13);
    let formula = planted(&mut rng, 90, 300);
    let at = |s_bound: f64| Prior { s_bound, width: 0 };
    let share = Duration::from_secs(12);
    let full = Plan {
        nominal: share,
        draw: DRAW,
        repairs: false,
    };
    assert_eq!(allowance(&formula, &at(1e6), share), Some(full));
    assert_eq!(allowance(&formula, &at(0.0), share), None);
    // Below the floor no share starts the build.
    assert!(allowance(&formula, &at(MIN_S_BOUND), REPAIR_SHARE).is_some());
    assert!(allowance(&formula, &at(MIN_S_BOUND - 0.1), REPAIR_SHARE).is_none());
    assert_eq!(allowance(&formula, &at(1e6), Duration::ZERO), None);
    let low = allowance(&formula, &at(30.0), share).expect("fits");
    let high = allowance(&formula, &at(40.0), share).expect("fits");
    assert!(low.nominal < high.nominal && high.nominal <= share);
    // The draws grow with the gain over the cost, between the two bounds.
    assert!(MIN_DRAW <= low.draw && low.draw <= high.draw && high.draw <= DRAW);
    assert!(low.draw < DRAW);
    // The repairs follow the share alone.
    let long = allowance(&formula, &at(1e6), REPAIR_SHARE).expect("fits");
    assert!(long.repairs && long.nominal == REPAIR_SHARE);
    // A share no prior can make fit is told apart without one.
    assert!(may_fit(&formula, share) && !may_fit(&formula, Duration::ZERO));
    let tight = Duration::from_millis((2.0 * BUILD_SLACK * build_cost(&formula)) as u64 / 3);
    assert!(!may_fit(&formula, tight) && allowance(&formula, &at(1e6), tight).is_none());
}
