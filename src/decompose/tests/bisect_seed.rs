use crate::decompose::bisect_seed::*;

/// The preservation property: a seedless caller (`base_seed = 0`) runs on
/// the historical constant-folded stream, restart for restart. Because the
/// bisectors are pure functions of this seed, equal seeds ⇒ byte-identical
/// partitions, so this pins the whole no-seed code path.
#[test]
fn zero_base_seed_preserves_the_legacy_stream() {
    assert_eq!(fmix64(0), 0, "the fixed point the invariant rests on");
    for restart in 0..8 {
        let legacy = (restart as u64) * 7919 + 42;
        assert_eq!(restart_seed(0, restart), legacy, "restart {restart}");
    }
}

/// The repair: the two seeds goatd's schedule actually uses (`s` and
/// `s + 42`) land on different RNG streams at every restart. Before the
/// fix `base_seed` was discarded and these were equal.
#[test]
fn schedule_slot_seeds_land_on_different_streams() {
    for base in [1u64, 7, 42, 12_345, u64::MAX / 3] {
        for restart in 0..8 {
            assert_ne!(
                restart_seed(base, restart),
                restart_seed(base.wrapping_add(42), restart),
                "base {base}, restart {restart}",
            );
        }
    }
}

/// A nonzero seed must actually move the stream off the legacy one — a mix
/// that collided back onto `legacy` would reinstate the bug for that seed.
#[test]
fn nonzero_base_seed_leaves_the_legacy_stream() {
    for base in [1u64, 7, 42, 12_345] {
        for restart in 0..8 {
            assert_ne!(
                restart_seed(base, restart),
                restart_seed(0, restart),
                "base {base}, restart {restart}",
            );
        }
    }
}

// End-to-end on the two bisectors. The seed pair is `(s, s + 42)` — exactly
// the pair goatd's five-slot schedule hands its two `NestedDissMinCover`
// slots.

const SLOT_A: u64 = 12_345;
const SLOT_B: u64 = SLOT_A + 42;

/// Deterministic pseudo-random graph. Random structure (rather than a grid)
/// so the FM local optimum genuinely depends on the RNG stream: on a highly
/// symmetric graph every stream converges to the same cut and the test
/// would not discriminate.
fn random_graph(n: usize, m: usize) -> Vec<(u32, u32)> {
    let mut rng = crate::decompose::rng::Xorshift64::from_state(0x2545_f491_4f6c_dd1d);
    let mut next = || rng.next_u64();
    let mut edges: Vec<(u32, u32)> = Vec::with_capacity(m + n);
    // A Hamiltonian-ish ring first, so the graph is connected.
    for v in 0..n {
        edges.push((v as u32, ((v + 1) % n) as u32));
    }
    while edges.len() < m {
        let u = (next() as usize) % n;
        let v = (next() as usize) % n;
        if u != v {
            edges.push((u.min(v) as u32, u.max(v) as u32));
        }
    }
    edges.sort_unstable();
    edges.dedup();
    edges
}

fn assert_proper_bisection(part: &[u8], n: usize) {
    assert_eq!(part.len(), n, "partition covers every vertex");
    assert!(part.contains(&0), "side 0 non-empty");
    assert!(part.contains(&1), "side 1 non-empty");
}

fn primal_part(seed: u64, edges: &[(u32, u32)], n: usize) -> Vec<u8> {
    crate::decompose::multilevel_bisect::multilevel_bisect(n, edges, 0.2, seed)
}

fn hg_part(seed: u64, hyperedges: &[Vec<u32>], n: usize) -> Vec<u8> {
    crate::decompose::multilevel_hg_bisect::multilevel_hg_bisect(
        n,
        hyperedges,
        None,
        crate::decompose::BisectDials {
            imbalance: crate::decompose::multilevel_hg_bisect::IMBALANCE_BALANCED,
            base_seed: seed,
            effort_scale: 1.0,
        },
    )
}

/// THE REGRESSION: goatd's two schedule seeds must reach the primal
/// bisector and produce genuinely different partitions. Before the fix the
/// bisector discarded the seed and these two were byte-identical.
#[test]
fn primal_bisect_differs_across_schedule_seeds() {
    let n = 200;
    let edges = random_graph(n, 800);
    let a = primal_part(SLOT_A, &edges, n);
    let b = primal_part(SLOT_B, &edges, n);
    assert_proper_bisection(&a, n);
    assert_proper_bisection(&b, n);
    assert_ne!(
        a, b,
        "distinct schedule seeds must give distinct partitions"
    );
    // Same seed, same answer: the fix must not make the bisector
    // nondeterministic.
    assert_eq!(a, primal_part(SLOT_A, &edges, n), "deterministic per seed");
}

/// Same regression in the hypergraph twin.
#[test]
fn hg_bisect_differs_across_schedule_seeds() {
    let n = 120;
    let hyperedges: Vec<Vec<u32>> = random_graph(n, 480)
        .into_iter()
        .map(|(u, v)| vec![u, v])
        .collect();
    let a = hg_part(SLOT_A, &hyperedges, n);
    let b = hg_part(SLOT_B, &hyperedges, n);
    assert_proper_bisection(&a, n);
    assert_proper_bisection(&b, n);
    assert_ne!(
        a, b,
        "distinct schedule seeds must give distinct partitions"
    );
    assert_eq!(a, hg_part(SLOT_A, &hyperedges, n), "deterministic per seed");
}
