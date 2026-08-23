//! Helpers shared across the test suite: a scratch directory, the formula and
//! decomposition builders, the shared fixture texts, and the record fixtures.
//!
//! Not every item is used from both sides — the record fixtures are the JSON
//! targets' alone, and several builders are only reached from the unit tests —
//! but each is here because it has more than one user, and one spelling of a
//! fixture is what keeps two tests from disagreeing about what it says.
//!
//! Rust gives each `tests/*/main.rs` its own crate, so this file is pulled in
//! with `#[path]` by each crate that needs it rather than compiled once — the
//! unit tests inside the library included, which reach it through
//! `crate::tests::common`. Everything here is written against the public API,
//! which is what lets one spelling serve both sides.
#![allow(dead_code)] // each user needs a different subset of what is here.

use std::path::{Path, PathBuf};

use num_rational::BigRational;
use vitri::bundle::{LiteralWeight, PreprocessRecord, RECORD_FORMAT_TAG};
use vitri::cnf::{Clause, CnfFormula, CnfMeta, Literal, Mode, ShowSet, VarId};
use vitri::decompose::{GraphKind, TdBag, TreeDecomposition};
use vitri::preprocess::{OriginalMap, OriginalTarget, VarMap};
use vitri::vtree::{Vtree, VtreeNode};

/// A scratch directory of one test's own, deleted when the test ends —
/// including when it ends by panicking, which is when a leftover directory is
/// least welcome and most likely.
///
/// The name carries the test's own tag, the process id and the clock, so two
/// tests running at once never share a directory and one that does survive
/// says which test left it.
pub(crate) struct Scratch(PathBuf);

impl Scratch {
    pub(crate) fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock must be past the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("vitri-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Scratch(dir)
    }

    /// The directory itself.
    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    /// Write `text` as `name` inside the scratch directory and return its path.
    pub(crate) fn file(&self, name: &str, text: &str) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, text).expect("fixture write");
        p
    }

    /// A path inside the scratch directory that nothing has created yet.
    pub(crate) fn out(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The literal over 0-based variable `var`.
pub(crate) fn lit(var: u32, positive: bool) -> Literal {
    Literal::new(VarId(var), positive)
}

/// A clause of 0-based `(variable, polarity)` pairs.
pub(crate) fn clause(lits: &[(u32, bool)]) -> Clause {
    Clause::new(lits.iter().map(|&(v, p)| lit(v, p)).collect())
}

/// A formula written in signed DIMACS literals (1-based, negative for a negated
/// literal), each clause sorted by variable — the shape a parsed formula has,
/// and the one the resolution kernels are entitled to assume.
pub(crate) fn make_formula(num_vars: u32, clauses_raw: Vec<Vec<i32>>) -> CnfFormula {
    let clauses = clauses_raw
        .into_iter()
        .map(|lits| {
            let mut literals: Vec<Literal> = lits.into_iter().map(Literal::from).collect();
            literals.sort_by_key(|l| l.var);
            Clause::new(literals)
        })
        .collect();
    CnfFormula { num_vars, clauses }
}

/// A uniform grid CNF: every clause is width 2 and every variable occurs twice,
/// so both halves of the coloring-like predicate pass. This is the class the
/// conditional skip targets.
pub(crate) fn grid_fixture() -> CnfFormula {
    parse("p cnf 4 4\n1 2 0\n2 3 0\n3 4 0\n4 1 0\n").0
}

/// Clause widths 2, 3, 5, 8: dispersion well past the coloring-like predicate's
/// width ceiling, so bounded variable addition stays on under `auto`.
pub(crate) fn mixed_width_fixture() -> CnfFormula {
    parse("p cnf 8 4\n1 2 0\n1 2 3 0\n1 2 3 4 5 0\n1 2 3 4 5 6 7 8 0\n").0
}

/// Five variables, one connected component, irreducible enough that the default
/// preprocessing leaves something to build a vtree over.
pub(crate) const IRREDUCIBLE_5: &str = "p cnf 5 5\n1 2 0\n-1 3 0\n-2 -3 4 0\n2 3 -4 0\n4 5 0\n";

/// Every variable forced or free, so preprocessing resolves the instance
/// outright and nothing is left to build a vtree over.
pub(crate) const FULLY_RESOLVED: &str = "p cnf 3 2\n1 0\n2 0\n";

/// Refuted by unit propagation alone.
pub(crate) const REFUTED: &str = "p cnf 2 2\n1 0\n-1 0\n";

/// Two variables declared and a clause naming a fifth: the id-range rule broken
/// where it is easiest to break it. What the two readers of this want differs —
/// one asks what the parser returns, the other what the binary prints — so the
/// text is here and the expectations stay with them.
pub(crate) const CLAUSE_ID_ABOVE_COUNT: &str = "p cnf 2 1\n1 5 0\n";

/// The same rule broken on a `c p show` line instead. Worth its own fixture
/// because a show id used to travel further before anything looked at it than a
/// clause literal did.
pub(crate) const SHOW_ID_ABOVE_COUNT: &str = "c t pmc\np cnf 2 1\nc p show 9 0\n1 2 0\n";

/// A clause written in signed DIMACS literals (1-based, negative for a negated
/// literal), in the order given.
pub(crate) fn clause_dimacs(lits: &[i32]) -> Clause {
    Clause::new(lits.iter().map(|&l| Literal::from(l)).collect())
}

/// The formula and the metadata a DIMACS text parses to.
pub(crate) fn parse(dimacs: &str) -> (CnfFormula, CnfMeta) {
    CnfFormula::from_dimacs(std::io::Cursor::new(dimacs.to_string())).expect("test CNF must parse")
}

/// One chain per entry of `sizes`, laid end to end over a single variable
/// space: the i-th spans `sizes[i]` consecutive variables and shares none with
/// its neighbours, so each chain is exactly one independent component.
///
/// Component size is what most callers vary — either side of the tiny
/// threshold, or small enough that no deadline is consulted — so the shape is
/// written once here.
pub(crate) fn chain_components(sizes: &[u32]) -> CnfFormula {
    let mut clauses = Vec::new();
    let mut next = 0u32;
    for &size in sizes {
        for a in next..next + size - 1 {
            clauses.push(Clause::new(vec![lit(a, true), lit(a + 1, false)]));
        }
        next += size;
    }
    CnfFormula {
        num_vars: next,
        clauses,
    }
}

/// Sixty variables in one chain with chords across it, so every variable is
/// tied into a single component. Wide enough that a portfolio really runs its
/// catalog rather than taking a small-formula shortcut, and the chords give the
/// candidate strategies a genuine decomposition choice instead of all
/// converging on the chain's one sensible vtree.
pub(crate) fn wide_component() -> CnfFormula {
    let n = 60u32;
    let mut formula = chain_components(&[n]);
    for a in 0..n - 7 {
        formula.clauses.push(Clause::new(vec![
            lit(a, false),
            lit(a + 5, true),
            lit(a + 7, true),
        ]));
    }
    formula
}

/// [`wide_component`] as DIMACS text, which is the only form the binary can be
/// handed one. `track_and_show` is written ahead of the header when given, and
/// brings a `c p show` line over the odd variables with it, so the same shape
/// can be asked of a projected mode.
pub(crate) fn wide_component_dimacs(track_and_show: Option<&str>) -> String {
    let formula = wide_component();
    let mut text = String::new();
    if let Some(header) = track_and_show {
        text.push_str(header);
    }
    text.push_str(&format!(
        "p cnf {} {}\n",
        formula.num_vars,
        formula.clauses.len()
    ));
    if track_and_show.is_some() {
        text.push_str("c p show");
        for v in (1..=formula.num_vars).step_by(2) {
            text.push_str(&format!(" {v}"));
        }
        text.push_str(" 0\n");
    }
    for clause in &formula.clauses {
        for literal in clause.iter() {
            text.push_str(&format!("{} ", literal.to_dimacs()));
        }
        text.push_str("0\n");
    }
    text
}

/// A vtree file split into the node count its header states and one token list
/// per line after it, in the order the file gives them — which the format
/// guarantees is children before parents. What the tokens mean is each caller's
/// own subject; this only stops two readers from disagreeing about where a
/// field sits on the line.
pub(crate) fn tokenize_vtree_text(text: &str) -> (usize, Vec<Vec<String>>) {
    let mut lines = text.lines();
    let header: Vec<&str> = lines
        .next()
        .expect("a vtree file leads with its header")
        .split_whitespace()
        .collect();
    assert_eq!(header[0], "vtree", "the header names the format");
    let declared = header[1].parse().expect("the header states a node count");
    let nodes = lines
        .map(|l| l.split_whitespace().map(str::to_string).collect())
        .collect();
    (declared, nodes)
}

/// The structural contract every vtree construction owes its caller: each
/// variable `0..n` sits on exactly one leaf, the tree has exactly `n` leaves and
/// `n - 1` internal nodes, every child points back at its parent, and only the
/// root has none. `what` names the construction under test, so a failure in a
/// loop over several says which one broke.
pub(crate) fn assert_covers_all_vars(vt: &Vtree, n: u32, what: &str) {
    assert_eq!(vt.num_leaves(), n, "leaf count ({what})");
    let mut seen = vec![false; n as usize];
    for (_idx, var) in vt.leaf_bottomup() {
        assert!(
            !seen[var.idx()],
            "variable {var:?} is on more than one leaf ({what})"
        );
        seen[var.idx()] = true;
    }
    assert!(
        seen.iter().all(|&s| s),
        "some variable has no leaf ({what})"
    );
    let mut internals = 0;
    for idx in vt.bottomup() {
        match vt.node(idx) {
            VtreeNode::Leaf { .. } => {}
            VtreeNode::Internal { left, right, .. } => {
                internals += 1;
                assert_ne!(left, right, "internal node with one child twice ({what})");
                for child in [*left, *right] {
                    assert_eq!(
                        vt.node(child).parent(),
                        Some(idx),
                        "child does not point back at its parent ({what})"
                    );
                }
            }
        }
    }
    assert_eq!(
        internals,
        n as usize - 1,
        "a binary tree over {n} leaves has n-1 internal nodes ({what})"
    );
    assert_eq!(
        vt.node(vt.root()).parent(),
        None,
        "the root must have no parent ({what})"
    );
}

/// A `TreeDecomposition` built by hand, without going through the PACE parser.
/// `tree_edges` names bags by index and is undirected — each edge is recorded
/// on both sides.
pub(crate) fn make_td(
    bags: Vec<Vec<u32>>,
    tree_edges: Vec<(usize, usize)>,
    num_vars: u32,
) -> TreeDecomposition {
    let mut adj = vec![Vec::new(); bags.len()];
    for &(a, b) in &tree_edges {
        adj[a].push(b);
        adj[b].push(a);
    }
    TreeDecomposition {
        kind: GraphKind::Primal,
        bags: bags
            .into_iter()
            .enumerate()
            .map(|(id, vertices)| TdBag { id, vertices })
            .collect(),
        adj,
        num_vars,
    }
}

/// The exact rational `n/d`.
pub(crate) fn rat(n: i64, d: i64) -> BigRational {
    BigRational::new(n.into(), d.into())
}

/// A 64-bit linear congruential generator with the constants Knuth lists for
/// MMIX. Not cryptographic and not meant to be — the only property required is
/// that it produce the same sequence everywhere, forever, so a generated
/// fixture is a fixture and not a sample.
pub(crate) struct Lcg(u64);

impl Lcg {
    pub(crate) fn new(seed: u64) -> Self {
        Lcg(seed)
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        // The high bits of an LCG are the well-distributed ones; the low bits
        // cycle with short periods, and `below` would inherit that bias.
        self.0 >> 32
    }

    /// A value in `0..n`, carrying the modulo bias a plain remainder has. At
    /// the small bounds a fixture draws from, that bias moves no structural
    /// property of what comes out.
    pub(crate) fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

/// A record exercising every field, and every entry kind of the
/// original→reduced map: a negative literal (a polarity flip), a positive one,
/// a constant, and the `Free` entry for a variable nothing constrains. The
/// three JSON types that encoding uses are all present, which is what lets a
/// consumer tell the kinds apart without a tag.
///
/// Reduced variable 1 IS original variable 3; reduced variable 2 was introduced
/// by preprocessing; reduced variable 3 is the NEGATION of original variable 2.
/// The map back reads the same correspondence the other way.
///
/// One fixture, used both to pin the bytes this serializes to and to check that
/// parsing them returns it: a record shaped for one of those and not the other
/// would leave the untested half of the contract to a second fixture that
/// drifts.
pub(crate) fn full_record() -> PreprocessRecord {
    PreprocessRecord {
        format: RECORD_FORMAT_TAG.to_string(),
        mode: Mode::Pwmc,
        count_lift_pow2: 3,
        weight_lift: "7/8".to_string(),
        original_num_vars: 4,
        reduced_to_original_dimacs: VarMap::from_entries(vec![Some(3), None, Some(-2)]),
        original_to_reduced_dimacs: Some(OriginalMap::from_entries(vec![
            OriginalTarget::Literal(-3),
            OriginalTarget::Literal(3),
            OriginalTarget::Constant(false),
            OriginalTarget::Free,
        ])),
        forced_literals_original_dimacs: vec![-4],
        free_vars_original_dimacs: vec![2],
        unsat: false,
        show_vars_reduced_dimacs: Some(ShowSet::from_dimacs_ids(&[1, 3]).expect("valid ids")),
        reduced_weights: Some(vec![LiteralWeight {
            literal: -1,
            weight: "1/2".to_string(),
        }]),
    }
}

/// A record whose optional lists are all empty, so the writer omits their keys
/// entirely — the shape a plain `mc` run with nothing to report emits.
pub(crate) fn sparse_record() -> PreprocessRecord {
    PreprocessRecord {
        format: RECORD_FORMAT_TAG.to_string(),
        mode: Mode::Mc,
        count_lift_pow2: 0,
        weight_lift: "1/1".to_string(),
        original_num_vars: 1,
        reduced_to_original_dimacs: VarMap::identity(1),
        original_to_reduced_dimacs: None,
        forced_literals_original_dimacs: Vec::new(),
        free_vars_original_dimacs: Vec::new(),
        unsat: false,
        show_vars_reduced_dimacs: None,
        reduced_weights: None,
    }
}
