use super::*;
use crate::tests::learnt_clauses::assert_learnts_are_implied;

/// Drive Arjun's stages directly and harvest the redundant/learnt clauses,
/// then verify the two properties the SAT-prune feeding relies on:
///   (1) NUMBERING — every harvested clause is in the reduced formula's var
///       space (all vars < reduced.num_vars), the SAME numbering as
///       `cur_formula`; and
///   (2) SOUNDNESS — each harvested clause C is IMPLIED by the reduced
///       formula: `reduced ∧ ¬C` is UNSAT (checked with a fresh CaDiCaL).
/// A learnt clause that fails either would let the SAT-prune solver kill a
/// live node (wrong count), so this is the guard for the whole feature.
#[test]
fn arjun_learnts_harvest_sound_and_in_reduced_space() {
    use crate::preprocess::cadical_ffi::{CaDiCal, Status};

    // Reduction config note: a full-count (`all_indep`) reduction of a SMALL
    // formula collapses to 0v/0c (Arjun solves it outright) → no residual, no
    // learnts. A residual-LEAVING reduction is what the oracle records learnts
    // on (and what a hard PRODUCTION instance yields under `all_indep`). We
    // provoke a residual deterministically with a projection (`set_sampl`) +
    // `no_bve`; the harvest getter, numbering, and `reduced ⊨ C` soundness this
    // test checks are IDENTICAL to the production `all_indep` path — both read
    // the one `s->cur` (`red_clauses`/`cur_formula`).
    let clauses: &[&[i32]] = &[
        &[1, 2, 3],
        &[-1, -2, 4],
        &[2, -3, 5],
        &[-4, 5, 6],
        &[1, -5, -6],
        &[3, 4, -6],
        &[7, 8, -1],
        &[-7, 9, 2],
        &[8, -9, 10],
        &[-8, -10, 11],
        &[9, 10, -12],
        &[-11, 12, 1],
        &[4, 7, -10],
        &[-3, -8, 11],
        &[5, -9, 12],
        &[6, -7, -11],
        &[-2, 8, 12],
        &[1, -4, 9],
        &[2, 5, -7],
        &[-6, 10, -12],
        &[3, -5, 8],
        &[-1, 7, 11],
        &[4, -8, -9],
        &[-3, 6, 10],
        &[2, -4, -11],
        &[5, 9, -12],
        &[-1, -6, 8],
        &[3, 7, -10],
        &[-2, -5, 11],
        &[1, 6, -9],
    ];
    let mut a = ArjunLib::new(ArjunOptions::default().seed).expect("shim ctor");
    a.new_vars(12);
    for c in clauses {
        a.add_clause_dimacs(c);
    }
    a.set_sampl(&[0, 1, 2, 3, 4, 5]);
    assert!(a.stage_minimize_indep(false), "minimize stage failed");
    // oracle ON so red_clauses (learnts) are collected; no_bve leaves a residual.
    assert!(
        a.stage_simplify(false, true, false, true),
        "simplify stage failed"
    );

    let reduced = a.cur_formula();
    let nv = reduced.num_vars;
    let raw = a.red_clauses();
    // Apply the same surviving-var filter reduce_anytime uses.
    let learnts: Vec<Vec<i32>> = raw
        .iter()
        .filter(|cl| !cl.is_empty() && cl.iter().all(|&l| l.unsigned_abs().saturating_sub(1) < nv))
        .cloned()
        .collect();
    eprintln!(
        "[test] red_clauses harvested: {} raw, {} after surviving-var filter, reduced {}v/{}c",
        raw.len(),
        learnts.len(),
        nv,
        reduced.clauses.len(),
    );
    // Guard against a silently-empty getter (broken shim/ABI): this config
    // reliably yields learnts.
    assert!(
        !learnts.is_empty(),
        "expected a non-empty learnt-clause harvest"
    );

    for cl in &learnts {
        for &l in cl {
            assert!(
                l.unsigned_abs().saturating_sub(1) < nv,
                "harvested learnt lit {l} out of reduced var space (nv={nv})",
            );
        }
    }

    for cl in &learnts {
        let mut s = CaDiCal::new().expect("the solver allocates");
        if nv > 0 {
            s.reserve(nv as i32);
        }
        for rc in &reduced.clauses {
            for lit in &rc.literals {
                s.add(lit.to_dimacs());
            }
            s.add(0);
        }
        for &l in cl {
            s.add(-l);
            s.add(0);
        }
        assert_eq!(
            s.solve(),
            Status::Unsatisfiable,
            "reduced ∧ ¬C is SAT — harvested learnt {cl:?} is NOT implied (unsound)",
        );
    }
}

/// Model-count invariance the knob relies on: appending the harvested learnt
/// clauses to the reduced formula must not remove any model (they are
/// implied), so the reduced formula's full count is identical with the
/// learnts present vs absent — i.e. "count is the same knob-on vs knob-off".
#[test]
fn arjun_learnts_appended_preserve_count() {
    let mut a = ArjunLib::new(ArjunOptions::default().seed).expect("shim ctor");
    a.new_vars(12);
    // Residual-leaving (projected + no_bve) setup, so the harvest is
    // non-empty and the invariant is actually exercised.
    let clauses: &[&[i32]] = &[
        &[1, 2, 3],
        &[-1, -2, 4],
        &[2, -3, 5],
        &[-4, 5, 6],
        &[1, -5, -6],
        &[3, 4, -6],
        &[7, 8, -1],
        &[-7, 9, 2],
        &[8, -9, 10],
        &[-8, -10, 11],
        &[9, 10, -12],
        &[-11, 12, 1],
        &[4, 7, -10],
        &[-3, -8, 11],
        &[5, -9, 12],
        &[6, -7, -11],
        &[-2, 8, 12],
        &[1, -4, 9],
    ];
    for c in clauses {
        a.add_clause_dimacs(c);
    }
    a.set_sampl(&[0, 1, 2, 3, 4, 5]);
    assert!(a.stage_minimize_indep(false));
    assert!(a.stage_simplify(false, true, false, true));

    let reduced = a.cur_formula();
    let nv = reduced.num_vars;
    assert!(nv <= 20, "brute count needs small nv");

    let learnts: Vec<Vec<i32>> = a
        .red_clauses()
        .into_iter()
        .filter(|cl| !cl.is_empty() && cl.iter().all(|&l| l.unsigned_abs().saturating_sub(1) < nv))
        .collect();

    assert_learnts_are_implied(&reduced, &learnts);
}
