use std::time::Duration;

use crate::cnf::{Clause, CnfFormula, Literal};
use crate::config::PreprocessClock;
use crate::preprocess::implicates::{
    Allowance, Found, Keep, StoppedBy, candidate_cells, find, find_shared,
};
use crate::preprocess::unit_propagation::ScopedUnits;
use crate::tests::pmc_oracle::{
    brute_force_mc, brute_force_pmc, brute_force_pwmc, brute_force_wmc, satisfies,
};
use crate::vtree::Vtree;
use num_rational::BigRational;

fn cnf(num_vars: u32, clauses: &[&[i32]]) -> CnfFormula {
    CnfFormula::new(
        num_vars,
        clauses
            .iter()
            .map(|c| Clause::new(c.iter().map(|&l| Literal::from(l)).collect()))
            .collect(),
    )
    .expect("valid clauses")
}

fn tree(text: &str) -> Vtree {
    Vtree::from_vtree_text(text).expect("valid vtree")
}

fn wall(ms: u64) -> Allowance {
    let ms = Duration::from_millis(ms);
    Allowance::of_run(PreprocessClock::WallClock, ms, ms, None)
}

fn charged(ms: u64) -> Allowance {
    let ms = Duration::from_millis(ms);
    let clock = PreprocessClock::Deterministic {
        configured_wall_ms: None,
    };
    Allowance::of_run(clock, ms, ms, None)
}

fn dimacs(found: &Found) -> Vec<Vec<i32>> {
    found
        .clauses
        .iter()
        .map(|c| c.literals.iter().map(|l| l.to_dimacs()).collect())
        .collect()
}

/// A random 3-CNF over `n` variables with `m` clauses, from a fixed seed.
fn random_3cnf(n: u32, m: usize, seed: u64) -> CnfFormula {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut clauses = Vec::new();
    while clauses.len() < m {
        let mut c: Vec<i32> = Vec::new();
        while c.len() < 3 {
            let v = (next() % u64::from(n)) as i32 + 1;
            if c.iter().all(|l| l.abs() != v) {
                c.push(if next() % 2 == 0 { v } else { -v });
            }
        }
        clauses.push(Clause::new(c.into_iter().map(Literal::from).collect()));
    }
    CnfFormula::new(n, clauses).expect("valid clauses")
}

/// Satisfiable seeded 3-CNFs with the trees the pass is run over.
fn cases() -> Vec<(CnfFormula, Vtree)> {
    let mut out = Vec::new();
    for seed in 1..40 {
        let f = random_3cnf(12, 40, seed);
        if (0..1u64 << 12).any(|a| satisfies(&f, a)) {
            out.push((f, Vtree::random(12, seed)));
        }
    }
    out.push((random_3cnf(12, 30, 99), Vtree::balanced(12)));
    out.push((random_3cnf(12, 30, 98), Vtree::linear(12)));
    out
}

fn with_clauses(f: &CnfFormula, extra: &[Clause]) -> CnfFormula {
    let mut clauses = f.clauses().to_vec();
    clauses.extend_from_slice(extra);
    CnfFormula::new(f.num_vars(), clauses).expect("valid clauses")
}

fn weight(v: u32, positive: bool) -> BigRational {
    let w = BigRational::new((v as i64 + 1).into(), (v as i64 + 3).into());
    if positive {
        w
    } else {
        BigRational::from_integer(1.into()) - w
    }
}

#[test]
fn every_emitted_clause_holds_in_every_model() {
    let mut emitted = 0;
    for (f, t) in cases() {
        let found = find(&f, &t, &wall(10_000));
        emitted += found.clauses.len();
        let only = |c: &Clause| CnfFormula::new(f.num_vars(), vec![c.clone()]).expect("valid");
        for a in (0..1u64 << f.num_vars()).filter(|&a| satisfies(&f, a)) {
            for c in &found.clauses {
                assert!(satisfies(&only(c), a), "{c:?} fails a model of the formula");
            }
        }
    }
    assert!(emitted > 0, "the cases must exercise the pass");
}

#[test]
fn adding_the_clauses_leaves_the_model_count_unchanged() {
    for (f, t) in cases() {
        let found = find(&f, &t, &wall(10_000));
        assert_eq!(
            brute_force_mc(&f),
            brute_force_mc(&with_clauses(&f, &found.clauses))
        );
    }
}

#[test]
fn adding_the_clauses_leaves_the_projected_count_unchanged() {
    for (f, t) in cases() {
        let found = find(&f, &t, &wall(10_000));
        let show = [1, 3, 5, 7, 9];
        assert_eq!(
            brute_force_pmc(&f, &show),
            brute_force_pmc(&with_clauses(&f, &found.clauses), &show)
        );
    }
}

#[test]
fn adding_the_clauses_leaves_the_weighted_count_unchanged() {
    for (f, t) in cases() {
        let found = find(&f, &t, &wall(10_000));
        assert_eq!(
            brute_force_wmc(&f, weight),
            brute_force_wmc(&with_clauses(&f, &found.clauses), weight)
        );
    }
}

#[test]
fn adding_the_clauses_leaves_the_projected_weighted_count_unchanged() {
    for (f, t) in cases() {
        let found = find(&f, &t, &wall(10_000));
        let show = [2, 4, 6, 8];
        assert_eq!(
            brute_force_pwmc(&f, &show, weight),
            brute_force_pwmc(&with_clauses(&f, &found.clauses), &show, weight)
        );
    }
}

/// Leaf positions left to right of `t`, by variable index.
fn positions(t: &Vtree) -> Vec<usize> {
    let mut pos = vec![0; t.num_vars() as usize];
    let mut stack = vec![t.root()];
    let mut cursor = 0;
    while let Some(n) = stack.pop() {
        if t.node(n).is_leaf() {
            pos[t.leaf_var(n).idx()] = cursor;
            cursor += 1;
        } else {
            let (l, r) = t.children(n);
            stack.extend([r, l]);
        }
    }
    pos
}

#[test]
fn every_variable_of_each_clause_is_open_at_their_lca() {
    for (f, t) in cases() {
        let pos = positions(&t);
        let found = find(&f, &t, &wall(10_000));
        for c in &found.clauses {
            let lca = c.literals[1..]
                .iter()
                .fold(t.leaf_of(c.literals[0].var), |n, l| {
                    t.lca(n, t.leaf_of(l.var))
                });
            let leaves: Vec<usize> = (0..t.num_vars() as usize)
                .filter(|&v| {
                    let mut n = t.leaf_of(crate::cnf::VarId::from_idx(v));
                    loop {
                        if n == lca {
                            break true;
                        }
                        match t.node(n).parent() {
                            Some(p) => n = p,
                            None => break false,
                        }
                    }
                })
                .map(|v| pos[v])
                .collect();
            let (lo, hi) = (*leaves.iter().min().unwrap(), *leaves.iter().max().unwrap());
            for v in c.literals.iter().map(|l| l.var) {
                let reaches_out = f.clauses().iter().any(|cl| {
                    cl.literals.iter().any(|l| l.var == v)
                        && cl
                            .literals
                            .iter()
                            .any(|l| pos[l.var.idx()] < lo || pos[l.var.idx()] > hi)
                });
                assert!(reaches_out, "{v:?} of {c:?} is not open at its LCA");
            }
        }
    }
}

#[test]
fn no_emitted_clause_is_a_clause_of_the_formula() {
    for (f, t) in cases() {
        let found = find(&f, &t, &wall(10_000));
        let key = |c: &Clause| {
            let mut k: Vec<i32> = c.literals.iter().map(|l| l.to_dimacs()).collect();
            k.sort_unstable();
            k
        };
        for c in &found.clauses {
            assert!(
                f.clauses().iter().all(|d| key(d) != key(c)),
                "{c:?} is in the formula"
            );
        }
    }
}

/// `1 → 3 → 2` with the tree `((1 2) 3)`: `¬1 ∨ 2` holds by propagation
/// through 3, which lies outside the node of 1 and 2, so it is kept.
#[test]
fn a_clause_propagation_proves_is_emitted() {
    let f = cnf(3, &[&[-1, 3], &[-3, 2]]);
    let t = tree("vtree 5\nL 0 1\nL 1 2\nI 2 0 1\nL 3 3\nI 4 2 3\n");
    let found = find(&f, &t, &wall(10_000));
    assert_eq!(dimacs(&found), vec![vec![-1, 2]]);
}

/// `¬1 ∨ 2` follows from the four clauses `¬1 ∨ 2 ∨ ±3 ∨ ±4` only by a case
/// split on 3 and 4.
#[test]
fn a_clause_only_search_proves_is_emitted() {
    let f = cnf(
        4,
        &[
            &[-1, 2, 3, 4],
            &[-1, 2, 3, -4],
            &[-1, 2, -3, 4],
            &[-1, 2, -3, -4],
        ],
    );
    let t = tree("vtree 7\nL 0 1\nL 1 2\nI 2 0 1\nL 3 3\nL 4 4\nI 5 3 4\nI 6 2 5\n");
    let found = find(&f, &t, &wall(10_000));
    assert_eq!(dimacs(&found), vec![vec![-1, 2]]);
}

#[test]
fn no_ternary_clause_holds_an_implied_binary_one() {
    let mut ternary = 0;
    for (f, t) in cases() {
        let found = find(&f, &t, &wall(10_000));
        let models: Vec<u64> = (0..1u64 << f.num_vars())
            .filter(|&a| satisfies(&f, a))
            .collect();
        for c in found.clauses.iter().filter(|c| c.literals.len() == 3) {
            ternary += 1;
            for skip in 0..3 {
                let sub: Vec<Literal> = (0..3)
                    .filter(|&k| k != skip)
                    .map(|k| c.literals[k])
                    .collect();
                let only = CnfFormula::new(f.num_vars(), vec![Clause::new(sub)]).expect("valid");
                assert!(
                    models.iter().any(|&a| !satisfies(&only, a)),
                    "{c:?} holds an implied binary clause"
                );
            }
        }
    }
    assert!(ternary > 0, "the cases must exercise the triple step");
}

/// `1 ∧ 2 → 4 → 3` with the tree `(((1 2) 3) 4)`: `¬1 ∨ ¬2 ∨ 3` holds only
/// through 4, outside the node of 1, 2 and 3, and no binary part of it holds.
#[test]
fn a_ternary_clause_implied_through_the_outside_is_emitted() {
    let f = cnf(4, &[&[-1, -2, 4], &[-4, 3]]);
    let t = tree("vtree 7\nL 0 1\nL 1 2\nI 2 0 1\nL 3 3\nI 4 2 3\nL 5 4\nI 6 4 5\n");
    let found = find(&f, &t, &wall(10_000));
    assert_eq!(dimacs(&found), vec![vec![-1, -2, 3]]);
}

/// `¬1 ∨ ¬2 ∨ 3` follows from `¬1 ∨ ¬2 ∨ 5` and `¬5 ∨ 3`, both inside the node
/// `((1 2) (3 5))`, so it is not emitted. Over `(((1 2) 3) (5 4))` they lie
/// outside the node of 1, 2 and 3, and it is.
#[test]
fn a_ternary_clause_its_node_implies_is_not_emitted() {
    let f = cnf(5, &[&[-1, -2, 5], &[-5, 3], &[1, 4], &[2, 4], &[3, 4]]);
    let inside =
        tree("vtree 9\nL 0 1\nL 1 2\nI 2 0 1\nL 3 3\nL 4 5\nI 5 3 4\nI 6 2 5\nL 7 4\nI 8 6 7\n");
    let found = find(&f, &inside, &wall(10_000));
    assert!(found.clauses.iter().all(|c| c.literals.len() < 3));

    let outside =
        tree("vtree 9\nL 0 1\nL 1 2\nI 2 0 1\nL 3 3\nI 4 2 3\nL 5 5\nL 6 4\nI 7 5 6\nI 8 4 7\n");
    let found = find(&f, &outside, &wall(10_000));
    assert!(dimacs(&found).contains(&vec![-1, -2, 3]));
}

#[test]
fn two_runs_on_the_deterministic_clock_emit_the_same_clauses() {
    for (f, t) in cases() {
        let a = find(&f, &t, &charged(1_000));
        let b = find(&f, &t, &charged(1_000));
        assert_eq!(dimacs(&a), dimacs(&b));
    }
}

#[test]
fn a_zero_allowance_emits_nothing() {
    for (f, t) in cases() {
        assert!(find(&f, &t, &charged(0)).clauses.is_empty());
    }
}

#[test]
fn a_large_component_gets_clauses_from_its_capped_candidates() {
    let (big, big_tree) = chains();
    assert!(candidate_cells(&big, &big_tree, Keep::of_run(Duration::from_secs(10))) > 0);

    let found = find(&big, &big_tree, &charged(10_000));
    let clauses = dimacs(&found);
    assert!(!clauses.is_empty());
    for c in &clauses {
        assert!(
            c.len() == 2 && c[0] < 0 && -c[0] % 2 == 1 && c[1] == 1 - c[0],
            "{c:?}"
        );
    }
}

/// x → z → y for 1,000 triples, x = 2i + 1 and y = 2i + 2 side by side in the
/// first half of a balanced tree and every z in its second, so ¬x ∨ y is
/// implied but not by the clauses inside the node of x and y.
fn chains() -> (CnfFormula, Vtree) {
    let n = 3_000u32;
    let rows: Vec<[i32; 2]> = (0..1_000)
        .flat_map(|i| {
            let (x, y, z) = (2 * i + 1, 2 * i + 2, 2_001 + i);
            [[-x, z], [-z, y]]
        })
        .collect();
    let rows: Vec<&[i32]> = rows.iter().map(|c| &c[..]).collect();
    (cnf(n, &rows), Vtree::balanced(n))
}

/// Proofs still coming when the allowance runs out carry the pass past it,
/// but never past twice the allowance.
#[test]
fn a_pass_still_proving_at_its_allowance_goes_on_up_to_twice_it() {
    let (f, t) = chains();
    let mut extended = false;
    for ms in [40, 60, 80, 120, 160, 240] {
        let found = find(&f, &t, &charged(ms));
        let allowance = Duration::from_millis(ms);
        assert!(
            found.spent <= allowance * 2 + allowance / 10,
            "{ms} ms: spent {:?}",
            found.spent
        );
        if found.spent > allowance + allowance / 10 && !found.clauses.is_empty() {
            extended = true;
        }
    }
    assert!(extended, "no allowance was extended");
}

/// A big component and a small one: the small one's share follows the
/// candidate counts as the caps bound them, and both get clauses.
#[test]
fn the_allowance_is_divided_by_capped_candidate_counts() {
    let big = random_3cnf(3_000, 6_000, 7);
    let big_tree = Vtree::balanced(3_000);
    let (small, small_tree) = cases().remove(0);
    let keep = Keep::of_run(Duration::from_millis(1_000));
    let (cb, cs) = (
        candidate_cells(&big, &big_tree, keep),
        candidate_cells(&small, &small_tree, keep),
    );
    assert!(cb > 0 && cs > 0);
    let shared = find_shared(
        &[(&big, &big_tree), (&small, &small_tree)],
        Duration::from_millis(1_000),
        PreprocessClock::Deterministic {
            configured_wall_ms: None,
        },
        None,
    );
    assert!(shared[0].is_some());
    let rest = shared[1].as_ref().expect("the small part has candidates");
    assert_eq!(rest.stopped_by, None, "the small part ran to its end");
}

/// `1 → 3 → 2` over `((1 2) 3)`: the one clause found, `¬1 ∨ 2`, has its
/// lowest common ancestor at the node of 1 and 2, which holds no clause of the
/// formula, so the run's pass drops it.
#[test]
fn a_set_mostly_above_no_clause_of_the_formula_is_dropped() {
    let f = cnf(3, &[&[-1, 3], &[-3, 2]]);
    let t = tree("vtree 5\nL 0 1\nL 1 2\nI 2 0 1\nL 3 3\nI 4 2 3\n");
    assert_eq!(find(&f, &t, &wall(10_000)).at_empty_nodes, 1);
    let shared = find_shared(
        &[(&f, &t)],
        Duration::from_millis(10_000),
        PreprocessClock::WallClock,
        None,
    );
    let found = shared[0].as_ref().expect("the part has candidates");
    assert!(found.dropped);
    assert!(found.clauses.is_empty());
}

/// The same chain closed by `1 ∨ ¬2`, a clause inside the node of 1 and 2:
/// `¬1 ∨ 2` now sits at a node that holds a clause, and is kept.
#[test]
fn a_set_at_nodes_holding_clauses_is_kept() {
    let f = cnf(3, &[&[-1, 3], &[-3, 2], &[1, -2]]);
    let t = tree("vtree 5\nL 0 1\nL 1 2\nI 2 0 1\nL 3 3\nI 4 2 3\n");
    let shared = find_shared(
        &[(&f, &t)],
        Duration::from_millis(10_000),
        PreprocessClock::WallClock,
        None,
    );
    let found = shared[0].as_ref().expect("the part has candidates");
    assert!(!found.dropped);
    assert_eq!(found.at_empty_nodes, 0);
    assert_eq!(dimacs(found), vec![vec![-1, 2]]);
}

/// A random 3-CNF a little under the satisfiability threshold implies almost
/// no binary clause, so the proof steps go without a proof until the drought
/// ends them, long before the allowance would.
#[test]
fn a_drought_of_proofs_ends_the_pass_before_its_allowance() {
    let f = random_3cnf(1_000, 3_800, 3);
    let t = Vtree::balanced(1_000);
    let found = find(&f, &t, &charged(60_000));
    assert_eq!(found.stopped_by, Some(StoppedBy::Drought));
    assert!(found.spent < Duration::from_secs(30));
    // However quick the component is to set up, the drought is a fifth of the
    // allowance.
    assert!(found.spent >= Duration::from_secs(12));
}

/// The drought is a fifth of the run's whole allowance, whatever share of it
/// one component gets.
#[test]
fn the_drought_is_a_fifth_of_the_runs_allowance() {
    let clock = PreprocessClock::WallClock;
    let a = Allowance::of_run(
        clock,
        Duration::from_millis(500),
        Duration::from_millis(2_000),
        None,
    );
    assert_eq!(a.drought, Duration::from_millis(400));
    let a = Allowance::of_run(
        clock,
        Duration::from_secs(60),
        Duration::from_secs(60),
        None,
    );
    assert_eq!(a.drought, Duration::from_secs(12));
}

/// The keep caps are 8,192 and 65,536 up to a run allowance of 2,000 ms and
/// grow in proportion past it, whatever share of it one component gets.
#[test]
fn the_keep_caps_grow_with_the_runs_allowance() {
    let base = Keep {
        node: 8_192,
        component: 65_536,
    };
    assert_eq!(Keep::of_run(Duration::from_millis(500)), base);
    assert_eq!(Keep::of_run(Duration::from_millis(2_000)), base);
    assert_eq!(
        Keep::of_run(Duration::from_secs(60)),
        Keep {
            node: 30 * 8_192,
            component: 30 * 65_536,
        }
    );
    let clock = PreprocessClock::WallClock;
    let a = Allowance::of_run(
        clock,
        Duration::from_millis(500),
        Duration::from_secs(6),
        None,
    );
    assert_eq!(a.keep, Keep::of_run(Duration::from_secs(6)));
}

#[test]
fn scoped_propagation_ignores_clauses_outside_its_window() {
    let mut units = ScopedUnits::new(3);
    units.push(&[-1, 3], 0, 2);
    units.push(&[-3, 2], 1, 2);
    assert!(units.refutes(&[1, -2], 0, 3).0);
    assert!(!units.refutes(&[1, -2], 0, 2).0);
}

#[test]
fn scoped_propagation_skips_an_inactive_clause() {
    let mut units = ScopedUnits::new(2);
    let id = units.push(&[-1, 2], 0, 1);
    assert!(units.refutes(&[1, -2], 0, 2).0);
    units.set_active(id, false);
    assert!(!units.refutes(&[1, -2], 0, 2).0);
}
