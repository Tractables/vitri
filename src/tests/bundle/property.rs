use super::*;

/// A deterministic PRNG, so a failure is reproducible from the seed printed in
/// the assertion message. SplitMix64: tiny, and good enough to shuffle small
/// integers.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

/// The weight pool: every entry is a non-dyadic rational or an integer > 1, so a
/// float or power-of-two shortcut anywhere in the chain fails the exact
/// comparison. Deliberately includes weights that do NOT sum to 1 — nothing in
/// the lift may assume a probability normalization.
const WEIGHT_POOL: &[(i64, i64)] = &[
    (1, 3),
    (2, 3),
    (5, 7),
    (3, 11),
    (2, 1),
    (1, 5),
    (7, 9),
    (4, 13),
    (1, 1),
    (3, 2),
];

/// Generate a small random CNF with the structures preprocessing actually reacts
/// to: forced units (backbone), equivalence pairs, Tseitin definitions, free
/// variables, and plain random clauses. Returns the clause list as signed DIMACS.
fn random_clauses(rng: &mut Rng, n: usize) -> Vec<Vec<i32>> {
    let mut clauses: Vec<Vec<i32>> = Vec::new();
    let lit = |rng: &mut Rng, v: usize| -> i32 {
        let d = v as i32 + 1;
        if rng.chance(50) { d } else { -d }
    };

    // Backbone: a unit clause or two.
    for _ in 0..rng.below(3) {
        let v = rng.below(n);
        clauses.push(vec![lit(rng, v)]);
    }
    // Equivalences, as the two implications. `x ≡ y` or `x ≡ ¬y`.
    for _ in 0..rng.below(3) {
        let (a, b) = (rng.below(n), rng.below(n));
        if a == b {
            continue;
        }
        let (da, db) = (a as i32 + 1, b as i32 + 1);
        if rng.chance(50) {
            clauses.push(vec![-da, db]);
            clauses.push(vec![da, -db]);
        } else {
            clauses.push(vec![-da, -db]);
            clauses.push(vec![da, db]);
        }
    }
    // A Tseitin definition `g ⇔ (a ∧ b)`, which is what DVE and Arjun eliminate.
    for _ in 0..rng.below(3) {
        let (g, a, b) = (rng.below(n), rng.below(n), rng.below(n));
        if g == a || g == b || a == b {
            continue;
        }
        let (dg, da, db) = (g as i32 + 1, a as i32 + 1, b as i32 + 1);
        clauses.push(vec![-dg, da]);
        clauses.push(vec![-dg, db]);
        clauses.push(vec![dg, -da, -db]);
    }
    // Plain random clauses over a PREFIX of the variables, which leaves the tail
    // free some of the time.
    let live = 1 + rng.below(n);
    for _ in 0..(2 + rng.below(2 * n)) {
        let width = 2 + rng.below(2);
        let mut cl: Vec<i32> = Vec::new();
        for _ in 0..width {
            let v = rng.below(live);
            let l = lit(rng, v);
            if !cl.contains(&l) && !cl.contains(&-l) {
                cl.push(l);
            }
        }
        if !cl.is_empty() {
            clauses.push(cl);
        }
    }
    clauses
}

/// Render a random instance as DIMACS in `mode`, with a random show set (never
/// empty, sometimes the whole variable set) and random non-dyadic weights on the
/// variables the mode weights.
fn random_instance(rng: &mut Rng, mode: Mode, n: usize) -> String {
    let clauses = random_clauses(rng, n);
    let mut show: Vec<usize> = (0..n).filter(|_| rng.chance(60)).collect();
    if show.is_empty() {
        show.push(rng.below(n));
    }

    let mut s = format!("p cnf {} {}\nc t {}\n", n, clauses.len(), mode.token());
    if mode.is_projected() {
        s.push_str("c p show");
        for v in &show {
            s.push_str(&format!(" {}", v + 1));
        }
        s.push_str(" 0\n");
    }
    if mode.is_weighted() {
        // PWMC weights only the SHOW variables — a projected-out variable
        // contributes existence, not weight — while WMC weights everything.
        let weighted: Vec<usize> = if mode.is_projected() {
            show.clone()
        } else {
            (0..n).collect()
        };
        for v in weighted {
            let (pn, pd) = WEIGHT_POOL[rng.below(WEIGHT_POOL.len())];
            let (nn, nd) = WEIGHT_POOL[rng.below(WEIGHT_POOL.len())];
            s.push_str(&format!("c p weight {} {}/{} 0\n", v + 1, pn, pd));
            s.push_str(&format!("c p weight -{} {}/{} 0\n", v + 1, nn, nd));
        }
    }
    for cl in &clauses {
        for l in cl {
            s.push_str(&format!("{l} "));
        }
        s.push_str("0\n");
    }
    s
}

#[test]
fn property_all_modes_lift_exactly() {
    let modes = [Mode::Mc, Mode::Wmc, Mode::Pmc, Mode::Pwmc];
    // Every iteration is a full preprocess + write + re-parse + two brute-force
    // passes, and the oracles are exponential — but the instances are ≤ 8
    // variables, so a few hundred still runs in well under a second. Raise it
    // (locally) when hunting a composition bug: the generator is seeded, so a
    // failure names the seed that reproduces it.
    let iterations: u64 = 200;
    for seed in 0..iterations {
        for mode in modes {
            let mut rng = Rng(seed.wrapping_mul(0x1_0000_0001) ^ ((mode as u64) << 40));
            let n = 4 + rng.below(5); // 4..=8 variables
            let dimacs = random_instance(&mut rng, mode, n);
            let (_, meta) = parse(&dimacs);
            // A generated instance always matches its own declared mode, so this
            // resolution cannot fail; asserting it keeps a generator bug from
            // silently preprocessing under a different mode.
            assert_eq!(
                RunConfig::default().resolve_mode(&meta).map(|r| r.mode),
                Ok(mode)
            );

            let rt = round_trip(&format!("prop-{}-{seed}", mode.token()), &dimacs);
            assert!(
                rt.reparsed.num_vars <= 18,
                "seed {seed} mode {} produced a reduced formula too large to brute-force ({} vars)",
                mode.token(),
                rt.reparsed.num_vars,
            );
            // Any failure below prints the record; print the instance too, since
            // reproducing needs both.
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rt.assert_sound()))
                .unwrap_or_else(|e| {
                    eprintln!(
                        "--- failing instance (seed {seed}, mode {}) ---\n{dimacs}",
                        mode.token()
                    );
                    std::panic::resume_unwind(e)
                });
        }
    }
}

/// How many variables still OCCUR in a clause.
///
/// The measure of "did preprocessing do anything", and deliberately not
/// `num_vars`: the projection-preserving chain PRESERVES variable ids by contract
/// (so the show set, the weights and Arjun's map compose across it without a
/// second renumbering), which means an eliminated variable stops occurring but
/// the `p cnf` count stays put. Counting the header would score that chain as
/// inert no matter how much it removed.
fn live_vars(f: &CnfFormula) -> usize {
    let mut seen = std::collections::HashSet::new();
    for c in &f.clauses {
        for l in &c.literals {
            seen.insert(l.var.0);
        }
    }
    seen.len()
}

/// The same sweep for `compile`, whose contract is stronger than a count: over
/// the same generator, the reduced bundle must reconstruct the original FUNCTION
/// exactly. Separate from the sweep above because the generator declares a mode
/// in the file and `compile` is reached only by asking for it.
#[test]
fn property_compile_reconstructs_the_function() {
    let config = RunConfig {
        mode: Some(Mode::Compile),
        ..Default::default()
    };
    let mut fired = 0usize;
    for seed in 0..60u64 {
        let mut rng = Rng(seed.wrapping_mul(0xA24B_AED4) ^ 0xC1);
        let n = 4 + rng.below(4); // 4..=7 variables
        let dimacs = random_instance(&mut rng, Mode::Mc, n);
        let rt = round_trip_with(&format!("prop-compile-{seed}"), &dimacs, &config);
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rt.assert_sound();
            assert_function_reconstructs(&rt);
        }))
        .unwrap_or_else(|e| {
            eprintln!("--- failing instance (seed {seed}, mode compile) ---\n{dimacs}");
            std::panic::resume_unwind(e)
        });
        fired += usize::from(equivalence_fired(&rt));
    }
    // The sweep must actually exercise the stage whose reconstruction it checks.
    assert!(
        fired > 0,
        "the equivalence reduction fired on no instance of the sweep"
    );
}

/// Preprocessing must not be vacuous: over the same generator, at least some
/// instances of every mode must come back genuinely smaller. Without this the
/// property sweep above would pass unchanged on a pipeline that gave up and
/// emitted its input — every identity it asserts is trivially true of the
/// identity pipeline.
#[test]
fn property_preprocessing_actually_reduces() {
    for mode in [Mode::Mc, Mode::Wmc, Mode::Pmc, Mode::Pwmc] {
        let mut shrank = 0usize;
        for seed in 0..12u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9) ^ ((mode as u64) << 32));
            let n = 6 + rng.below(3);
            let dimacs = random_instance(&mut rng, mode, n);
            let (formula, meta) = parse(&dimacs);
            let bundle = preprocess(&formula, &meta, &RunConfig::default()).expect("preprocessing");
            if live_vars(&bundle.reduced) < live_vars(&formula) {
                shrank += 1;
            }
        }
        assert!(
            shrank > 0,
            "mode {} eliminated no variable at all across the sweep — the chain is inert",
            mode.token(),
        );
    }
}
