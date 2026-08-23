//! The round-trip harness every case in this module is written against:
//! run a bundle out to disk, read it back, and assert what the record
//! promises against what the files say.
//!
//! [`RoundTrip::assert_sound`] is the whole contract in one call; the
//! individual assertions are separate so a failure names the half that
//! broke.

use super::*;

/// Does the partial assignment `fixed` (indexed by 0-based variable, `None` =
/// unassigned) extend to some total model of `f`?
pub(super) fn extends_to_model(f: &CnfFormula, fixed: &[Option<bool>]) -> bool {
    let open: Vec<usize> = (0..fixed.len()).filter(|&i| fixed[i].is_none()).collect();
    assert!(
        open.len() <= 20,
        "too many unassigned vars to brute-force an extension"
    );

    let mut a: Vec<bool> = fixed.iter().map(|v| v.unwrap_or(false)).collect();
    for bits in 0u32..(1u32 << open.len()) {
        for (k, &v) in open.iter().enumerate() {
            a[v] = (bits >> k) & 1 == 1;
        }
        let sat = f
            .clauses
            .iter()
            .all(|c| c.literals.iter().any(|l| a[l.var.0 as usize] == l.positive));
        if sat {
            return true;
        }
    }
    false
}

/// Parse the record's / a `c p weight` line's `"num/den"` form back into an exact
/// rational. Deliberately a separate implementation from the writer's: a test
/// that re-used `rational_string`'s inverse could not catch a formatting bug.
pub(super) fn parse_rational(s: &str) -> BigRational {
    let (n, d) = s.split_once('/').expect("weights are written as num/den");
    BigRational::new(
        n.parse().expect("numerator"),
        d.parse().expect("denominator"),
    )
}

/// A record's literal list read back as the table it describes.
pub(super) fn record_weights(w: &[LiteralWeight], num_vars: usize) -> Weights<Reduced> {
    let pairs: Vec<(i32, BigRational)> = w
        .iter()
        .map(|lw| (lw.literal, parse_rational(&lw.weight)))
        .collect();
    Weights::from_dimacs_pairs(&pairs, num_vars)
}

/// The mode's count over `f`, as an exact rational — the one function both sides
/// of the identity go through, so "the reduced count" and "the original count"
/// cannot accidentally mean two different questions.
pub(super) fn mode_count(
    mode: Mode,
    f: &CnfFormula,
    show: Option<&[u32]>,
    w: &[(BigRational, BigRational)],
) -> BigRational {
    let all: Vec<u32> = (0..f.num_vars).collect();
    let show = show.unwrap_or(&all);
    match mode {
        // `compile` carries any declared weights through untouched instead of
        // counting under them, so its lift is the PLAIN count's.
        Mode::Mc | Mode::Compile => BigRational::from_integer(brute_force_mc(f).into()),
        Mode::Pmc => BigRational::from_integer(brute_force_pmc(f, show).into()),
        Mode::Wmc | Mode::Pwmc => brute_force_pwmc(f, show, |v, val| {
            let (wn, wp) = &w[v as usize];
            if val { wp.clone() } else { wn.clone() }
        }),
    }
}

pub(super) struct RoundTrip {
    pub(super) mode: Mode,
    pub(super) original: CnfFormula,
    pub(super) original_show: Option<Vec<u32>>,
    pub(super) original_weights: Weights<Original>,
    pub(super) reparsed: CnfFormula,
    pub(super) reparsed_meta: CnfMeta,
    pub(super) reduced_cnf_text: String,
    /// Whether the INPUT carried `c p weight` lines, which is what decides the
    /// pass-through under a mode that does not count under weights.
    pub(super) declares_weights: bool,
    pub(super) record: PreprocessRecord,
}

pub(super) fn round_trip(tag: &str, dimacs: &str) -> RoundTrip {
    round_trip_with(tag, dimacs, &RunConfig::default())
}

pub(super) fn round_trip_with(tag: &str, dimacs: &str, config: &RunConfig) -> RoundTrip {
    let (original, meta) = parse(dimacs);
    let mode = config
        .resolve_mode(&meta)
        .expect("test config must match the test instance")
        .mode;
    let bundle = preprocess(&original, &meta, config).expect("preprocessing must run");

    let dir = Scratch::new(tag);
    let paths = bundle.write_to_dir(dir.path()).expect("bundle must write");

    let reduced_cnf_text =
        std::fs::read_to_string(&paths.reduced_cnf).expect("reduced.cnf must be readable");
    let (reparsed, reparsed_meta) = parse(&reduced_cnf_text);

    let json = std::fs::read_to_string(&paths.record).expect("preprocess.json must be readable");
    serde_json::from_str::<serde_json::Value>(&json).expect("preprocess.json must be valid JSON");

    let original_weights = match (mode.is_weighted(), meta.declared_weights()) {
        (true, Some(t)) => t.resolve(original.num_vars as usize),
        _ => Weights::uniform(original.num_vars as usize),
    };
    RoundTrip {
        mode,
        original,
        original_show: meta
            .declared_show_vars()
            .map(|s| s.iter_vars().map(|v| v.0).collect()),
        original_weights,
        declares_weights: meta.declared_weights().is_some(),
        reparsed,
        reparsed_meta,
        reduced_cnf_text,
        record: bundle.record,
    }
}

impl RoundTrip {
    /// The show set the REDUCED count must be taken over, 0-based, read off the
    /// re-parsed `c p show` line rather than the record — so a record that
    /// disagrees with the file it accompanies fails
    /// [`Self::assert_show_set_round_trips`] instead of silently making this
    /// assertion pass.
    pub(super) fn reduced_show(&self) -> Option<Vec<u32>> {
        self.reparsed_meta
            .declared_show_vars()
            .map(|s| s.iter_vars().map(|v| v.0).collect())
    }

    /// The weights the REDUCED count must be taken under, read off the record.
    pub(super) fn reduced_weights(&self) -> Weights<Reduced> {
        match self.record.reduced_weights.as_ref() {
            Some(w) => record_weights(w, self.reparsed.num_vars as usize),
            None => Weights::uniform(self.reparsed.num_vars as usize),
        }
    }

    /// **The soundness assertion itself**, and the only identity this crate
    /// promises: `count(reduced) × 2^pow2 × weight_lift == count(original)`, in
    /// the mode's own count.
    pub(super) fn assert_lift_exact(&self) {
        let reduced = mode_count(
            self.mode,
            &self.reparsed,
            self.reduced_show().as_deref(),
            self.reduced_weights().as_pairs(),
        );
        let pow2 =
            BigRational::from_integer(BigUint::from(2u32).pow(self.record.count_lift_pow2).into());
        let lifted = reduced * pow2 * parse_rational(&self.record.weight_lift);
        let expected = mode_count(
            self.mode,
            &self.original,
            self.original_show.as_deref(),
            self.original_weights.as_pairs(),
        );
        assert_eq!(
            lifted,
            expected,
            "count(reduced) * 2^{} * {} must equal count(original) in mode {}; record = {}",
            self.record.count_lift_pow2,
            self.record.weight_lift,
            self.record.mode.token(),
            self.record.to_json_string(),
        );
    }

    pub(super) fn assert_mode_consistent(&self) {
        assert_eq!(self.record.mode, self.mode);
        if self.mode.is_weighted() {
            assert_eq!(
                self.record.count_lift_pow2, 0,
                "a weighted mode has no cardinality lift — every factor is a rational",
            );
        } else {
            assert_eq!(
                self.record.weight_lift, "1/1",
                "an unweighted mode has no weighted lift",
            );
        }
        // The emitted CNF must declare the same track, so a consumer reading
        // only the file solves the same problem as one reading only the record.
        // `compile` is the exception: it is not a track, so it gets no `c t` line
        // and the file re-parses at the default.
        let expect_header = if self.mode == Mode::Compile {
            Mode::Mc
        } else {
            self.mode
        };
        assert_eq!(
            self.reparsed_meta.mode, expect_header,
            "reduced.cnf's `c t` header must name the track the record does",
        );
        if self.mode == Mode::Compile {
            assert!(
                !self.reduced_cnf_text.contains("c t "),
                "`compile` is not a track, so reduced.cnf must carry no `c t` line, got:\n{}",
                self.reduced_cnf_text,
            );
        }
    }

    /// The variable map must be a genuine injection reduced → original, and the
    /// two directions must be exact inverses — signs included. A map that
    /// silently aliased two reduced variables onto one original, or that dropped
    /// a polarity flip, would still satisfy the count identity while making every
    /// lifted model wrong.
    pub(super) fn assert_map_consistent(&self) {
        let r = &self.record;
        assert_eq!(
            r.reduced_to_original_dimacs.len(),
            self.reparsed.num_vars as usize,
            "the map must have one entry per variable of the emitted `p cnf` header",
        );

        // The inverse is not serialized, so build it here and require it to be
        // an exact two-way inverse — signs included.
        let mut inverse: Vec<Option<i32>> = vec![None; r.original_num_vars as usize];
        let mut seen = std::collections::HashSet::new();
        for (i, entry) in r.reduced_to_original_dimacs.iter().enumerate() {
            let Some(o) = entry else { continue };
            let ov = o.unsigned_abs();
            assert!(
                o != 0 && ov <= r.original_num_vars,
                "original id {o} out of range ±1..=±{}",
                r.original_num_vars,
            );
            assert!(seen.insert(ov), "two reduced vars map to original var {ov}");
            let reduced_lit = i as i32 + 1;
            inverse[ov as usize - 1] = Some(if o > 0 { reduced_lit } else { -reduced_lit });
        }
        for (o, entry) in inverse.iter().enumerate() {
            let Some(red) = *entry else { continue };
            let expect = if red > 0 {
                o as i32 + 1
            } else {
                -(o as i32 + 1)
            };
            assert_eq!(
                r.reduced_to_original_dimacs.get(VarId::from_dimacs(red)),
                Some(expect),
                "inverse map points at a reduced var that maps elsewhere",
            );
        }
    }

    /// Every variable named free contributes a factor of 2, so the exponent is at
    /// least the length of that list. It can exceed it: Arjun reports its own
    /// contribution as an aggregate exponent rather than as variables to name.
    /// Weighted modes have no exponent at all — their factors are rationals inside
    /// `weight_lift`.
    pub(super) fn assert_lift_accounted(&self) {
        if self.mode.is_weighted() {
            return;
        }
        let r = &self.record;
        assert!(
            r.count_lift_pow2 >= r.free_vars_original_dimacs.len() as u32,
            "the 2^k exponent must cover every named free var; record = {}",
            r.to_json_string(),
        );
    }

    /// **The property a consumer enumerating models depends on.** Take every model
    /// of the emitted `reduced.cnf`, map it back through the map (honoring the
    /// sign), pin the recorded forced literals, and require that the resulting
    /// partial assignment of the ORIGINAL formula extends to a model of it.
    ///
    /// This is what a wrong map breaks and the count identity does not catch: a
    /// permuted, mis-signed or off-by-one map can leave `count(reduced)` exactly
    /// right while every model it names is a non-model of the original.
    ///
    /// "Extends to a model" rather than "is a model" is the correct statement:
    /// variables preprocessing eliminated because they are *determined* (an
    /// equivalence partner, a defined/BVE'd variable) are deliberately absent
    /// from the map, and the consumer recovers them by propagation — what must
    /// hold is that a consistent recovery exists.
    ///
    /// Under a PROJECTED mode the reduced formula's models are not models of the
    /// original at all (the projected variables were ∃-eliminated, so the reduced
    /// formula is a weaker constraint), and the meaningful statement is about the
    /// SHOW projection instead — see [`Self::assert_show_projections_lift_back`].
    pub(super) fn assert_models_lift_back(&self) {
        if self.mode.is_projected() {
            return self.assert_show_projections_lift_back();
        }
        let rn = self.reparsed.num_vars as usize;
        let on = self.original.num_vars as usize;
        assert!(
            rn <= 20 && on <= 20,
            "brute-force model lift is for small cases only"
        );

        let mut checked = 0usize;
        for a in 0u32..(1u32 << rn) {
            let sat = self.reparsed.clauses.iter().all(|c| {
                c.literals
                    .iter()
                    .any(|l| ((a >> l.var.0) & 1 == 1) == l.positive)
            });
            if !sat {
                continue;
            }
            // Partial original assignment: `None` = preprocessing determined it
            // and the consumer must recover it.
            let mut fixed: Vec<Option<bool>> = vec![None; on];
            let mut set = |v: usize, val: bool, what: &str| {
                if let Some(prev) = fixed[v] {
                    assert_eq!(
                        prev,
                        val,
                        "{what} contradicts an earlier assignment of var {}",
                        v + 1
                    );
                }
                fixed[v] = Some(val);
            };
            for (r, entry) in self.record.reduced_to_original_dimacs.iter().enumerate() {
                let Some(o) = entry else { continue };
                let bit = (a >> r) & 1 == 1;
                set(
                    o.unsigned_abs() as usize - 1,
                    if o > 0 { bit } else { !bit },
                    "the map",
                );
            }
            for &lit in &self.record.forced_literals_original_dimacs {
                set(lit.unsigned_abs() as usize - 1, lit > 0, "a forced literal");
            }
            assert!(
                extends_to_model(&self.original, &fixed),
                "a model of reduced.cnf does not lift back to any model of the original \
                 (reduced assignment bits {a:#b}); record = {}",
                self.record.to_json_string(),
            );
            checked += 1;
        }
        // A vacuous pass would hide everything above.
        if brute_force_mc(&self.original) != BigUint::ZERO {
            assert!(
                checked > 0,
                "the reduced formula must have at least one model to lift"
            );
        }
    }

    /// The projected analogue: every FEASIBLE show-projection of `reduced.cnf`
    /// must name a feasible show-projection of the original, through the map.
    ///
    /// Only the show variables the map still names are checked — Arjun folds free
    /// and determined show variables into its multiplier, and those have no
    /// counterpart to check by construction. What must not happen is a retained
    /// show variable whose reduced-space assignment does not correspond to
    /// anything the original admits.
    pub(super) fn assert_show_projections_lift_back(&self) {
        let rn = self.reparsed.num_vars as usize;
        let on = self.original.num_vars as usize;
        assert!(
            rn <= 20 && on <= 20,
            "brute-force lift is for small cases only"
        );
        let Some(red_show) = self.reduced_show() else {
            return;
        };
        let Some(orig_show) = self.original_show.as_ref() else {
            return;
        };

        // Reduced show var → the ORIGINAL show var it stands for, with polarity.
        let named: Vec<(usize, usize, bool)> = red_show
            .iter()
            .filter_map(|&rv| {
                let o = self.record.reduced_to_original_dimacs.get(VarId(rv))?;
                let ov = o.unsigned_abs() as usize - 1;
                orig_show
                    .contains(&(ov as u32))
                    .then_some((rv as usize, ov, o > 0))
            })
            .collect();
        if named.is_empty() {
            return;
        }

        for a in 0u32..(1u32 << rn) {
            let sat = self.reparsed.clauses.iter().all(|c| {
                c.literals
                    .iter()
                    .any(|l| ((a >> l.var.0) & 1 == 1) == l.positive)
            });
            if !sat {
                continue;
            }
            let mut fixed: Vec<Option<bool>> = vec![None; on];
            for &(rv, ov, same) in &named {
                let bit = (a >> rv) & 1 == 1;
                fixed[ov] = Some(if same { bit } else { !bit });
            }
            assert!(
                extends_to_model(&self.original, &fixed),
                "a feasible show-projection of reduced.cnf names one the original refuses \
                 (reduced assignment bits {a:#b}); record = {}",
                self.record.to_json_string(),
            );
        }
    }

    pub(super) fn assert_forced_literals_are_forced(&self) {
        for &lit in &self.record.forced_literals_original_dimacs {
            let var = lit.unsigned_abs() - 1;
            let mut probe = self.original.clone();
            probe.clauses.push(Clause::new(vec![Literal::new(
                crate::vtree::VarId(var),
                lit < 0, // the opposite polarity
            )]));
            assert_eq!(
                brute_force_mc(&probe),
                BigUint::ZERO,
                "literal {lit} is recorded as forced but the opposite polarity has models",
            );
        }
    }

    /// The show set must be in REDUCED numbering, must be present in both the
    /// record and the emitted CNF, and the two must agree. Re-deriving it from the
    /// input instead is the silent miscount this whole chain exists to prevent —
    /// Arjun rewrites the set AND renumbers it.
    pub(super) fn assert_show_set_round_trips(&self) {
        // `compile` carries the input's own declaration through, so the presence
        // of a show set is keyed on what the FILE declared, not on the mode.
        let expect_show = if self.mode == Mode::Compile {
            self.original_show.is_some()
        } else {
            self.mode.is_projected()
        };
        if !expect_show {
            assert!(
                self.record.show_vars_reduced_dimacs.is_none(),
                "a mode with no projection to carry must not claim a show set",
            );
            assert!(
                self.reparsed_meta.declared_show_vars().is_none(),
                "reduced.cnf must then carry no `c p show` line either",
            );
            return;
        }
        let recorded = self
            .record
            .show_vars_reduced_dimacs
            .as_ref()
            .expect("a projected bundle must record its show set");
        let emitted = self
            .reduced_show()
            .expect("a projected reduced.cnf must carry a `c p show` line");
        assert_eq!(
            recorded.to_dimacs(),
            emitted.iter().map(|v| v + 1).collect::<Vec<u32>>(),
            "the record's show set and the emitted `c p show` line must agree",
        );
        for v in recorded.to_dimacs() {
            assert!(
                v >= 1 && v <= self.reparsed.num_vars,
                "show var {v} is outside the reduced space 1..={}",
                self.reparsed.num_vars,
            );
        }
    }

    /// Weights likewise: present exactly under a weighted mode, in reduced
    /// numbering, exact, and identical whether read from the record or from the
    /// `c p weight` lines of the emitted CNF.
    pub(super) fn assert_weights_round_trip(&self) {
        // Same rule as the show set: under `compile` the declaration decides.
        let expect_weights = if self.mode == Mode::Compile {
            self.declares_weights
        } else {
            self.mode.is_weighted()
        };
        if !expect_weights {
            assert!(
                self.record.reduced_weights.is_none(),
                "a mode with no weights to carry must not claim a weight table",
            );
            return;
        }
        if self.record.unsat {
            return;
        }
        let recorded = self
            .record
            .reduced_weights
            .as_ref()
            .expect("a weighted bundle must record its reduced weights");
        for w in recorded {
            let v = w.literal.unsigned_abs();
            assert!(
                w.literal != 0 && v <= self.reparsed.num_vars,
                "weighted literal {} is outside the reduced space",
                w.literal,
            );
            // Exact, and canonical: `num/den` in lowest terms.
            let r = parse_rational(&w.weight);
            assert_eq!(
                rational_string(&r),
                w.weight,
                "weights must be written canonically"
            );
        }
        let from_record = record_weights(recorded, self.reparsed.num_vars as usize);
        let from_file: Weights<Reduced> = match self.reparsed_meta.declared_weights() {
            Some(t) => t.resolve(self.reparsed.num_vars as usize),
            None => Weights::uniform(self.reparsed.num_vars as usize),
        };
        assert_eq!(
            from_record, from_file,
            "the record's weights and the emitted `c p weight` lines must agree",
        );
    }

    pub(super) fn assert_sound(&self) {
        self.assert_mode_consistent();
        self.assert_lift_exact();
        self.assert_map_consistent();
        self.assert_lift_accounted();
        self.assert_models_lift_back();
        self.assert_forced_literals_are_forced();
        self.assert_show_set_round_trips();
        self.assert_weights_round_trip();
    }

    /// Preprocessing must have actually removed variables. Without this a
    /// pipeline that silently gave up (emitting the input unchanged with lift 0)
    /// would pass every identity above.
    pub(super) fn assert_reduced_below(&self, ceiling: u32) {
        assert!(
            self.reparsed.num_vars < ceiling,
            "expected preprocessing to drop below {ceiling} vars, got {} (mode {})",
            self.reparsed.num_vars,
            self.record.mode.token(),
        );
    }
}

/// The reconstruction runs entirely off `original_to_reduced_dimacs`: each
/// original variable is a signed literal of a reduced variable, a constant, or
/// free. An original assignment satisfies the original formula iff it agrees
/// with every constant entry, assigns every original variable sharing a
/// reduced variable consistently, and the reduced assignment it induces
/// satisfies `reduced.cnf`.
///
/// Driving it off the total map alone is deliberate: the reduced-indexed map
/// cannot name a dropped equivalence partner, so a reconstruction written
/// through it would silently ignore one and pass.
pub(super) fn assert_function_reconstructs(rt: &RoundTrip) {
    let on = rt.original.num_vars as usize;
    assert!(
        on <= 16,
        "exhaustive reconstruction is for small cases only"
    );
    let map = rt
        .record
        .original_to_reduced_dimacs
        .as_ref()
        .expect("compile writes the total original→reduced map");
    assert_eq!(
        map.len(),
        on,
        "the map must have an entry per original variable; record = {}",
        rt.record.to_json_string(),
    );
    let rn = rt.reparsed.num_vars as usize;

    for &lit in &rt.record.forced_literals_original_dimacs {
        assert_eq!(
            map.get(VarId::from_dimacs(lit)),
            Some(OriginalTarget::Constant(lit > 0)),
            "forced literal {lit} disagrees with its map entry; record = {}",
            rt.record.to_json_string(),
        );
    }
    for &v in &rt.record.free_vars_original_dimacs {
        assert_eq!(
            map.get(VarId::from_dimacs(v as i32)),
            Some(OriginalTarget::Free),
            "free variable {v} disagrees with its map entry; record = {}",
            rt.record.to_json_string(),
        );
    }
    // Every reduced variable must be named by some original one — `compile`
    // introduces none — or the reconstruction below could not assign it.
    let mut named = vec![false; rn];
    for target in map.iter() {
        if let OriginalTarget::Literal(l) = target {
            named[l.unsigned_abs() as usize - 1] = true;
        }
    }
    assert!(
        named.iter().all(|n| *n),
        "a reduced variable is named by no original variable; record = {}",
        rt.record.to_json_string(),
    );

    for a in 0u32..(1u32 << on) {
        let val = |v1: u32| (a >> (v1 - 1)) & 1 == 1;
        let original_sat = rt
            .original
            .clauses
            .iter()
            .all(|c| c.literals.iter().any(|l| val(l.var.0 + 1) == l.positive));

        // Entries must be mutually consistent: two originals folded onto one
        // reduced variable constrain each other.
        let mut reduced_bits: Vec<Option<bool>> = vec![None; rn];
        let mut agrees = true;
        for original in 0..on {
            let bit = val(original as u32 + 1);
            match map.get(VarId(original as u32)).expect("the map is total") {
                OriginalTarget::Literal(l) => {
                    let r = l.unsigned_abs() as usize - 1;
                    let reduced_bit = if l > 0 { bit } else { !bit };
                    match reduced_bits[r] {
                        Some(prev) => agrees &= prev == reduced_bit,
                        None => reduced_bits[r] = Some(reduced_bit),
                    }
                }
                OriginalTarget::Constant(value) => agrees &= bit == value,
                OriginalTarget::Free => {}
            }
        }
        let reconstructed = agrees
            && rt.reparsed.clauses.iter().all(|c| {
                c.literals.iter().any(|l| {
                    reduced_bits[l.var.0 as usize].expect("every reduced variable is named")
                        == l.positive
                })
            });
        assert_eq!(
            original_sat,
            reconstructed,
            "reconstruction disagrees with the original at assignment {a:#b}; record = {}",
            rt.record.to_json_string(),
        );
    }
}

/// Whether the equivalence reduction actually dropped a partner here: two
/// distinct original variables naming the same reduced variable is exactly that,
/// and it is the thing a reconstruction test must confirm happened — one that
/// runs on a no-op reduction proves nothing about the stage.
pub(super) fn equivalence_fired(rt: &RoundTrip) -> bool {
    let map = rt
        .record
        .original_to_reduced_dimacs
        .as_ref()
        .expect("compile writes the total original→reduced map");
    let mut claimed = vec![false; rt.reparsed.num_vars as usize];
    map.iter().any(|target| match target {
        OriginalTarget::Literal(l) => {
            std::mem::replace(&mut claimed[l.unsigned_abs() as usize - 1], true)
        }
        _ => false,
    })
}
