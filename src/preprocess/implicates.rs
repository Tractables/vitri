//! Clauses a component's CNF implies, found over the vtree built for it.
//!
//! The pass runs after construction and reads the finished tree, so the tree is
//! the same whether it runs or not. A variable is open at an internal node when
//! some clause holding it reaches outside the node's leaves.
//!
//! It has two steps over one solver session. The pair step's candidates are
//! the binary clauses over a variable open in a node's left child and one open
//! in its right child, all four sign patterns, at every internal node. The
//! triple step's are the ternary clauses over three variables open at one node
//! of at most 128 leaves and 20 open variables, each at the smallest such node
//! (its home), with the binary clauses over two such variables alongside: a
//! ternary clause that holds a proven binary one is not prime and is dropped.
//!
//! A candidate is dropped when a model of the formula falsifies it: first 32
//! models drawn from the solver with random phases, then every model a search
//! call returns. The rest are proven one by one, bottom-up by the size of the
//! node, by unit propagation under the negated clause or else by a search
//! capped at a conflict limit; a candidate the cap stops waits for the next
//! round at a ten times larger cap. In the triple step each round proves the
//! binary candidates first, then the ternary ones. Every proven clause is added
//! to the solver, so later proofs use it. A candidate that holds a literal the
//! formula implies on its own is dropped rather than proven.
//!
//! A proven ternary clause is emitted only when the clauses inside its home,
//! with the emitted binary clauses, do not already imply it. Last, in each step
//! a proven clause is dropped when unit propagation over the formula's clauses
//! and the other kept clauses that lie inside its lowest common ancestor's
//! leaves already refutes its negation.
//!
//! Two stops keep the pass off components where it finds little. A component
//! with more than 1,500 variables is skipped. And once 400 ms pass with no
//! clause proven that could be emitted (a binary one in the pair step, a
//! ternary one in the triple step), counted from the component's start, both
//! proof steps end and the local check and minimizing run on what was proven.
//!
//! A component's whole set is dropped when more than three quarters of its
//! clauses have a lowest common ancestor whose leaves hold no clause of the
//! formula. Below such a node a compiler that follows the vtree has little
//! state for the clause to cut, so the clause mostly adds to what it builds.
//!
//! Each emitted clause is implied by the component's formula, so adding it
//! leaves the formula's models, and every count over them, unchanged.

use std::cell::Cell as Shared;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use rand::SeedableRng;
use rand::rngs::SmallRng;
use serde::Serialize;

use super::backbone::read_model;
use super::cadical_ffi::{Bounded, CaDiCal, Status, Terminator, note_solver_unavailable};
use super::meter::{PhaseMark, PreprocessMeter};
use super::unit_propagation::ScopedUnits;
use crate::bundle::PreprocessPhase;
use crate::cnf::{Clause, CnfFormula, Literal, VarId};
use crate::config::PreprocessClock;
use crate::vtree::{Vtree, VtreeIdx, VtreeNode};

/// Models drawn before any proof: the seed model and the rest with random
/// phases.
const MODELS: usize = 32;
/// The conflict caps of the pair step's rounds. The last also caps each solve
/// that draws a model.
const PAIR_CAPS: [i32; 3] = [1_000, 10_000, 100_000];
/// The conflict caps of the triple step's rounds; the last also caps each
/// call of the local check.
const TRIPLE_CAPS: [i32; 3] = [200, 2_000, 20_000];
/// A node with more left-times-right pairs of open variables is skipped.
const MAX_NODE_PAIRS: usize = 2_000_000;
/// The triple step's nodes: at most this many leaves...
const TRIPLE_MAX_LEAVES: u32 = 128;
/// ...and at most this many open variables.
const TRIPLE_MAX_OPEN: usize = 20;
/// The share of the triple step's time kept for the local check.
const LOCAL_SHARE: f64 = 0.1;
/// Seed of the random phases.
const PHASE_SEED: u64 = 1;
/// A component with more variables is skipped.
const MAX_VARS: u32 = 1_500;
/// The proof steps end once this long, in the clock's currency, passes with
/// no clause proven that could be emitted.
const DROUGHT: Duration = Duration::from_millis(400);
/// A component's clauses are all dropped when more than this share of them
/// have a lowest common ancestor whose leaves hold no clause of the formula.
const MAX_EMPTY_SHARE: f64 = 0.75;

/// What the pass may spend, in the currency of `clock`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Allowance {
    /// The run's preprocessing clock: wall time, or charged work.
    pub clock: PreprocessClock,
    /// The nominal allowance.
    pub nominal: Duration,
    /// The run's deadline, which the allowance never passes.
    pub deadline: Option<Instant>,
}

/// The clauses one call found, and its record.
pub(crate) struct Found {
    /// Emitted clauses, binary and ternary, sorted, over the formula's own
    /// variables.
    pub clauses: Vec<Clause>,
    /// What the pass spent, in the clock's currency.
    pub spent: Duration,
    /// The pair step: per step, per round and per search call.
    pub log: PassLog,
    /// The triple step.
    pub triples: TripleLog,
}

/// Candidates per outcome.
#[derive(Debug, Default, Serialize)]
pub(crate) struct Counts {
    pub in_cnf: usize,
    pub refuted_by_models: usize,
    pub refuted_by_search: usize,
    pub unit_dropped: usize,
    pub not_prime: usize,
    pub proven_by_propagation: usize,
    pub proven_by_search: usize,
    pub unknown: usize,
    pub short: usize,
    pub unchecked: usize,
}

/// One search call: cap, node leaves, answer, conflicts, microseconds.
type Call = (i32, u32, &'static str, i64, u64);

/// What a proof round does with each model an answer gives, beyond refuting
/// its own cells.
type OnModel<'a> = dyn FnMut(&[i32], &mut Spend) + 'a;

/// The record of the pair step: counts per outcome, rounds, search calls and
/// steps.
#[derive(Debug, Default, Serialize)]
pub(crate) struct PassLog {
    pub vars: u32,
    pub clauses: usize,
    pub nodes: usize,
    pub nodes_skipped: usize,
    pub cells: usize,
    pub unsat: bool,
    pub models: usize,
    pub units: Vec<i32>,
    #[serde(flatten)]
    pub counts: Counts,
    pub kept: usize,
    pub budget_hit: bool,
    pub allowance_ms: u64,
    pub spent_ms: u64,
    pub rounds: Vec<RoundLog>,
    pub calls: Vec<Call>,
    pub steps: Vec<StepLog>,
    /// Per kept clause, in output order: microseconds since the pass began
    /// and work charged when it was proven.
    pub kept_proven_at: Vec<(u64, u64)>,
    /// Per proven candidate, kept or not, in proof order: as `kept_proven_at`.
    pub proven_at: Vec<(u64, u64)>,
    /// What ended the component early: `size` (skipped), `drought` or
    /// `allowance`; none when it ran to the end.
    pub stopped_by: Option<&'static str>,
    /// When the drought ended the proof steps, in the clock's milliseconds.
    pub drought_ms: Option<u64>,
    /// Emitted clauses whose lowest common ancestor's leaves hold no clause
    /// of the formula.
    pub at_empty_nodes: usize,
    /// Whether the set was dropped for having too many of those
    /// (`MAX_EMPTY_SHARE`).
    pub dropped: bool,
}

/// The record of the triple step.
#[derive(Debug, Default, Serialize)]
pub(crate) struct TripleLog {
    pub nodes: usize,
    pub pair_cells: usize,
    pub triple_cells: usize,
    pub units: Vec<i32>,
    pub pairs: Counts,
    pub triples: Counts,
    /// Proven ternary clauses that hold a binary one proven after them.
    pub not_prime_late: usize,
    pub rounds: Vec<TripleRound>,
    pub pair_calls: Vec<Call>,
    pub triple_calls: Vec<Call>,
    pub local: LocalLog,
    pub emitted: usize,
    pub kept: usize,
    pub budget_hit: bool,
    pub steps: Vec<StepLog>,
    pub kept_proven_at: Vec<(u64, u64)>,
    /// Per proven binary and ternary candidate, kept or not, in proof order.
    pub pair_proven_at: Vec<(u64, u64)>,
    pub proven_at: Vec<(u64, u64)>,
}

/// One round of the triple step.
#[derive(Debug, Default, Serialize)]
pub(crate) struct TripleRound {
    pub cap: i32,
    pub pairs: RoundLog,
    /// Open ternary candidates dropped as holding a proven binary clause.
    pub not_prime: usize,
    pub triples: RoundLog,
}

/// The local check of proven ternary clauses.
#[derive(Debug, Default, Serialize)]
pub(crate) struct LocalLog {
    pub by_propagation: usize,
    pub by_search: usize,
    pub unknown: usize,
    pub calls: usize,
    pub unchecked: usize,
    pub max_over_cap: i64,
}

/// One proof round.
#[derive(Debug, Default, Serialize)]
pub(crate) struct RoundLog {
    pub cap: i32,
    pub calls: usize,
    pub by_propagation: usize,
    pub by_search: usize,
    pub refuted_by_search: usize,
    pub deferred: usize,
    pub propagation_after: usize,
    pub open_after: usize,
    /// The most conflicts one call of this round spent past its cap.
    pub max_over_cap: i64,
}

/// One step of the pass.
#[derive(Debug, Serialize)]
pub(crate) struct StepLog {
    pub step: &'static str,
    pub us: u64,
    pub propagations: u64,
    pub scanned: u64,
}

/// Static candidate count of `formula` over `vtree`: what the run's allowance
/// is divided by.
pub(crate) fn candidate_cells(formula: &CnfFormula, vtree: &Vtree) -> u64 {
    let layout = Layout::new(vtree);
    let reach = Reach::new(formula, &layout);
    open_parts(&layout, &reach)
        .iter()
        .map(|p| p.left.len() * p.right.len())
        .filter(|&pairs| pairs <= MAX_NODE_PAIRS)
        .map(|pairs| 4 * pairs as u64)
        .sum()
}

/// Whether the pass skips a component of `formula`.
fn too_large(formula: &CnfFormula) -> bool {
    formula.num_vars() > MAX_VARS
}

/// [`find`] over each of `parts` in turn, `total` divided among them, each
/// set then kept or dropped whole by [`drop_if_mostly_at_empty_nodes`]. Each
/// gets the share of what is left that its candidate count is of the
/// candidates left, so what one leaves unspent goes to the later ones. A part
/// [`find`] skips for its size counts no candidates. A part with no
/// candidates is skipped (`None`).
pub(crate) fn find_shared(
    parts: &[(&CnfFormula, &Vtree)],
    total: Duration,
    clock: PreprocessClock,
    deadline: Option<Instant>,
) -> Vec<Option<Found>> {
    let cells: Vec<u64> = parts
        .iter()
        .map(|(f, t)| {
            if too_large(f) {
                0
            } else {
                candidate_cells(f, t)
            }
        })
        .collect();
    let mut cells_left: u64 = cells.iter().sum();
    let mut spent = Duration::ZERO;
    let mut out = Vec::with_capacity(parts.len());
    for ((f, t), c) in parts.iter().zip(cells) {
        let skip = too_large(f);
        if c == 0 && !skip {
            out.push(None);
            continue;
        }
        let left = total.saturating_sub(spent);
        let share = if skip {
            Duration::ZERO
        } else {
            left.mul_f64(c as f64 / cells_left as f64)
        };
        cells_left -= c;
        let found = find(
            f,
            t,
            &Allowance {
                clock,
                nominal: share,
                deadline,
            },
        );
        spent += found.spent;
        out.push(Some(drop_if_mostly_at_empty_nodes(found)));
    }
    out
}

/// `found` with no clauses when more than [`MAX_EMPTY_SHARE`] of them have a
/// lowest common ancestor whose leaves hold no clause of the formula.
pub(crate) fn drop_if_mostly_at_empty_nodes(mut found: Found) -> Found {
    if found.log.at_empty_nodes as f64 > MAX_EMPTY_SHARE * found.clauses.len() as f64 {
        found.log.dropped = true;
        found.clauses.clear();
    }
    found
}

/// Binary clauses `formula` implies whose two variables are open at their
/// lowest common ancestor in `vtree`, and ternary clauses it implies over
/// three variables open at one small node, found within `allowance`.
pub(crate) fn find(formula: &CnfFormula, vtree: &Vtree, allowance: &Allowance) -> Found {
    if too_large(formula) {
        return Found {
            clauses: Vec::new(),
            spent: Duration::ZERO,
            log: PassLog {
                vars: formula.num_vars(),
                clauses: formula.clauses().len(),
                stopped_by: Some("size"),
                ..PassLog::default()
            },
            triples: TripleLog::default(),
        };
    }
    let begun = Instant::now();
    let layout = Layout::new(vtree);
    let reach = Reach::new(formula, &layout);
    let parts = open_parts(&layout, &reach);
    let mut log = PassLog {
        vars: formula.num_vars(),
        clauses: formula.clauses().len(),
        nodes: parts.len(),
        ..PassLog::default()
    };
    let mut pairs = Cells::pairs(&parts, &layout, &mut log);
    log.cells = pairs.len();
    let small = small_nodes(&layout, &reach);
    let mut tlog = TripleLog {
        nodes: small.len(),
        ..TripleLog::default()
    };
    let mut seen = HashSet::new();
    let mut tpairs = Cells::sets(&small, &layout, 2, &mut seen);
    let mut triples = Cells::sets(&small, &layout, 3, &mut seen);
    tlog.pair_cells = tpairs.len();
    tlog.triple_cells = triples.len();

    let mut meter = PreprocessMeter::new(allowance.clock);
    let allowed = meter.clamp(allowance.nominal, allowance.deadline);
    log.allowance_ms = allowed.as_millis() as u64;
    let mark = meter.begin(PreprocessPhase::Implicates, allowed);
    let mut kept_pairs = Vec::new();
    let mut kept_triples = Vec::new();
    if pairs.len() + triples.len() > 0 {
        let mut spend = Spend {
            meter,
            mark,
            limit: allowed / 2,
            begun,
            step_started: Instant::now(),
            props: 0,
            scanned: 0,
            step_props: 0,
            step_scanned: 0,
            last_proof: Duration::ZERO,
            drought_at: Rc::new(Shared::new(None)),
            dried: None,
        };
        spend.arm();
        if let Some(mut solver) = CaDiCal::new() {
            let n = formula.num_vars() as usize;
            add_formula(&mut solver, formula);
            let stop = spend.stop();
            let batch = draw_models(&mut solver, n, stop, &mut spend, &mut log);
            if let Some(batch) = batch {
                {
                    let stop = spend.stop();
                    let mut bounded = Bounded::new(&mut solver, stop);
                    prove_pairs(
                        formula,
                        &mut bounded,
                        &mut pairs,
                        &batch,
                        &mut spend,
                        &mut log,
                    );
                }
                kept_pairs = minimize_pairs(formula, &layout, &pairs, &mut spend, &mut log.steps);
                let used = spend.used();
                spend.limit = used + allowed.saturating_sub(used).mul_f64(1.0 - LOCAL_SHARE);
                let kept_lits: Vec<Vec<i32>> =
                    kept_pairs.iter().map(|&i| pairs.lits(i).to_vec()).collect();
                if triples.len() > 0 && spend.dried.is_none() {
                    {
                        let stop = spend.stop();
                        let mut bounded = Bounded::new(&mut solver, stop);
                        prove_triples(
                            formula,
                            &mut bounded,
                            &mut tpairs,
                            &mut triples,
                            &batch,
                            &mut spend,
                            &mut tlog,
                        );
                    }
                    drop(solver);
                    spend.disarm();
                    spend.limit = allowed;
                    let emitted = local_check(
                        formula, vtree, &layout, &kept_lits, &triples, &mut spend, &mut tlog,
                    );
                    tlog.emitted = emitted.len();
                    kept_triples = minimize_triples(
                        formula,
                        vtree,
                        &layout,
                        &kept_lits,
                        &triples,
                        &emitted,
                        &mut spend,
                        &mut tlog.steps,
                    );
                }
            }
        } else {
            note_solver_unavailable("implicates", "no implied clauses are emitted");
        }
        log.drought_ms = spend.dried.map(|d| d.as_millis() as u64);
        meter = spend.meter;
    }
    log.stopped_by = if log.drought_ms.is_some() {
        Some("drought")
    } else if log.budget_hit || tlog.budget_hit {
        Some("allowance")
    } else {
        None
    };
    log.proven_at = pairs.proofs.clone();
    tlog.pair_proven_at = tpairs.proofs.clone();
    tlog.proven_at = triples.proofs.clone();
    let spent = meter.elapsed(mark);
    log.spent_ms = spent.as_millis() as u64;
    let mut out_pairs: Vec<(Vec<i32>, (u64, u64))> = kept_pairs
        .iter()
        .map(|&i| (pairs.lits(i).to_vec(), pairs.proven_at[i]))
        .collect();
    out_pairs.sort();
    log.kept = out_pairs.len();
    log.kept_proven_at = out_pairs.iter().map(|(_, at)| *at).collect();
    let mut out_triples: Vec<(Vec<i32>, (u64, u64))> = kept_triples
        .iter()
        .map(|&i| (triples.lits(i).to_vec(), triples.proven_at[i]))
        .collect();
    out_triples.sort();
    tlog.kept = out_triples.len();
    tlog.kept_proven_at = out_triples.iter().map(|(_, at)| *at).collect();
    let mut clauses: Vec<Vec<i32>> = out_pairs
        .into_iter()
        .chain(out_triples)
        .map(|(c, _)| c)
        .collect();
    clauses.sort();
    log.at_empty_nodes = at_empty_nodes(formula, vtree, &layout, &clauses);
    Found {
        clauses: clauses
            .into_iter()
            .map(|c| Clause::new(c.into_iter().map(Literal::from).collect()))
            .collect(),
        spent,
        log,
        triples: tlog,
    }
}

/// How many of `clauses` have a lowest common ancestor in `vtree` whose leaves
/// hold no clause of `formula`.
fn at_empty_nodes(
    formula: &CnfFormula,
    vtree: &Vtree,
    layout: &Layout,
    clauses: &[Vec<i32>],
) -> usize {
    if clauses.is_empty() {
        return 0;
    }
    // Per vtree node: whether some clause of the formula lies inside it.
    let mut holds = vec![false; vtree.num_nodes()];
    for c in formula.clauses().iter().filter(|c| !c.literals.is_empty()) {
        let lits: Vec<i32> = c.literals.iter().map(|l| l.to_dimacs()).collect();
        holds[layout.lca(vtree, &lits).0.idx()] = true;
    }
    for t in vtree.bottomup() {
        if let VtreeNode::Internal { left, right, .. } = *vtree.node(t) {
            holds[t.idx()] |= holds[left.idx()] || holds[right.idx()];
        }
    }
    clauses
        .iter()
        .filter(|c| !holds[layout.lca(vtree, c).0.idx()])
        .count()
}

// ── The tree ────────────────────────────────────────────────────────────────

/// Leaf positions left to right, and each node's interval.
struct Layout {
    /// Variable index → leaf position.
    pos: Vec<u32>,
    /// Leaf position → variable index.
    order: Vec<u32>,
    /// Per vtree node: its leaves `lo[t]..hi[t]`.
    lo: Vec<u32>,
    hi: Vec<u32>,
    /// Internal nodes, children before parents.
    nodes: Vec<Span>,
    /// The vtree node of each entry of `nodes`.
    ids: Vec<VtreeIdx>,
}

/// An internal node's leaves `lo..hi`, its right child's from `mid`.
#[derive(Clone, Copy)]
struct Span {
    lo: u32,
    mid: u32,
    hi: u32,
}

impl Span {
    fn leaves(self) -> u32 {
        self.hi - self.lo
    }
}

impl Layout {
    fn new(vtree: &Vtree) -> Self {
        let n = vtree.num_vars() as usize;
        let mut pos = vec![0u32; n];
        let mut order = vec![0u32; n];
        let mut lo = vec![0u32; vtree.num_nodes()];
        let mut hi = vec![0u32; vtree.num_nodes()];
        let mut cursor = 0u32;
        let mut stack = vec![(vtree.root(), false)];
        while let Some((t, done)) = stack.pop() {
            match *vtree.node(t) {
                VtreeNode::Leaf { var, .. } => {
                    pos[var.idx()] = cursor;
                    order[cursor as usize] = var.idx() as u32;
                    lo[t.idx()] = cursor;
                    hi[t.idx()] = cursor + 1;
                    cursor += 1;
                }
                VtreeNode::Internal { left, right, .. } if done => {
                    lo[t.idx()] = lo[left.idx()];
                    hi[t.idx()] = hi[right.idx()];
                }
                VtreeNode::Internal { left, right, .. } => {
                    stack.extend([(t, true), (right, false), (left, false)]);
                }
            }
        }
        let mut nodes = Vec::new();
        let mut ids = Vec::new();
        for t in vtree.bottomup() {
            if let VtreeNode::Internal { right, .. } = *vtree.node(t) {
                nodes.push(Span {
                    lo: lo[t.idx()],
                    mid: lo[right.idx()],
                    hi: hi[t.idx()],
                });
                ids.push(t);
            }
        }
        Layout {
            pos,
            order,
            lo,
            hi,
            nodes,
            ids,
        }
    }

    /// Leaf positions `lo..=hi` a clause spans.
    fn span_of(&self, lits: impl Iterator<Item = usize>) -> (u32, u32) {
        lits.fold((u32::MAX, 0), |(lo, hi), v| {
            let p = self.pos[v];
            (lo.min(p), hi.max(p))
        })
    }

    /// The lowest common ancestor of a clause's variables, and its leaves
    /// `lo..hi`.
    fn lca(&self, vtree: &Vtree, lits: &[i32]) -> (VtreeIdx, u32, u32) {
        let (a, b) = self.span_of(lits.iter().map(|l| l.unsigned_abs() as usize - 1));
        let leaf = |p: u32| vtree.leaf_of(VarId::from_idx(self.order[p as usize] as usize));
        let t = vtree.lca(leaf(a), leaf(b));
        (t, self.lo[t.idx()], self.hi[t.idx()])
    }
}

/// Per variable, the smallest and largest leaf position over the clauses that
/// hold it.
struct Reach {
    lo: Vec<u32>,
    hi: Vec<u32>,
}

impl Reach {
    fn new(formula: &CnfFormula, layout: &Layout) -> Self {
        let mut lo = layout.pos.clone();
        let mut hi = layout.pos.clone();
        for clause in formula.clauses() {
            let (a, b) = layout.span_of(clause.literals.iter().map(|l| l.var.idx()));
            for lit in &clause.literals {
                let v = lit.var.idx();
                lo[v] = lo[v].min(a);
                hi[v] = hi[v].max(b);
            }
        }
        Reach { lo, hi }
    }

    /// The variable at leaf position `p`, when it is open at node `s`.
    fn open_at(&self, layout: &Layout, s: Span, p: u32) -> Option<u32> {
        let v = layout.order[p as usize] as usize;
        (self.lo[v] < s.lo || self.hi[v] >= s.hi).then_some(v as u32)
    }
}

/// The open variables of one internal node's two children, in leaf order.
struct OpenPart {
    node: usize,
    left: Vec<u32>,
    right: Vec<u32>,
}

/// Every internal node where both children have an open variable.
fn open_parts(layout: &Layout, reach: &Reach) -> Vec<OpenPart> {
    let mut parts = Vec::new();
    for (node, &s) in layout.nodes.iter().enumerate() {
        let left: Vec<u32> = (s.lo..s.mid)
            .filter_map(|p| reach.open_at(layout, s, p))
            .collect();
        let right: Vec<u32> = (s.mid..s.hi)
            .filter_map(|p| reach.open_at(layout, s, p))
            .collect();
        if !left.is_empty() && !right.is_empty() {
            parts.push(OpenPart { node, left, right });
        }
    }
    parts
}

/// The triple step's nodes, fewest leaves first, each with its open variables
/// in index order.
fn small_nodes(layout: &Layout, reach: &Reach) -> Vec<(usize, Vec<u32>)> {
    let mut out = Vec::new();
    for (node, &s) in layout.nodes.iter().enumerate() {
        if s.leaves() > TRIPLE_MAX_LEAVES {
            continue;
        }
        let mut open: Vec<u32> = (s.lo..s.hi)
            .filter_map(|p| reach.open_at(layout, s, p))
            .collect();
        if (1..=TRIPLE_MAX_OPEN).contains(&open.len()) {
            open.sort_unstable();
            out.push((node, open));
        }
    }
    out.sort_by_key(|(node, _)| layout.nodes[*node].leaves());
    out
}

// ── Candidates ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cell {
    Open,
    /// Unknown at a cap below the last: retried in the next round.
    Deferred,
    /// Unknown when the allowance ran out.
    Short,
    /// Unknown at the last cap.
    Unknown,
    Model,
    SearchModel,
    Unit,
    InCnf,
    /// Holds a proven binary clause.
    NotPrime,
    ByPropagation,
    BySearch,
}

impl Cell {
    fn open(self) -> bool {
        matches!(self, Cell::Open | Cell::Deferred)
    }
    fn proven(self) -> bool {
        matches!(self, Cell::ByPropagation | Cell::BySearch)
    }
    /// Known to be implied, whether proven here or not.
    fn implied(self) -> bool {
        self.proven() || matches!(self, Cell::InCnf | Cell::Unit)
    }
}

/// Candidate clauses of one length, stored flat.
struct Cells {
    r: usize,
    lits: Vec<i32>,
    leaves: Vec<u32>,
    span: Vec<Span>,
    /// The home node, an index into [`Layout::nodes`].
    node: Vec<u32>,
    status: Vec<Cell>,
    /// Microseconds since the pass began, and work charged, at each proof.
    proven_at: Vec<(u64, u64)>,
    /// The same, per proof in proof order.
    proofs: Vec<(u64, u64)>,
    /// Whether a proof here could be emitted, and so ends a drought.
    emits: bool,
}

impl Cells {
    fn empty(r: usize, emits: bool) -> Self {
        Cells {
            r,
            emits,
            proofs: Vec::new(),
            lits: Vec::new(),
            leaves: Vec::new(),
            span: Vec::new(),
            node: Vec::new(),
            status: Vec::new(),
            proven_at: Vec::new(),
        }
    }

    fn push(&mut self, lits: &[i32], layout: &Layout, node: usize) {
        let s = layout.nodes[node];
        self.lits.extend_from_slice(lits);
        self.leaves.push(s.leaves());
        self.span.push(s);
        self.node.push(node as u32);
    }

    fn seal(&mut self) {
        self.status = vec![Cell::Open; self.leaves.len()];
        self.proven_at = vec![(0, 0); self.leaves.len()];
    }

    /// The pair step's candidates: per node, the four sign patterns of
    /// open-left times open-right, left-major.
    fn pairs(parts: &[OpenPart], layout: &Layout, log: &mut PassLog) -> Self {
        let mut cells = Cells::empty(2, true);
        for p in parts {
            if p.left.len() * p.right.len() > MAX_NODE_PAIRS {
                log.nodes_skipped += 1;
                continue;
            }
            for (sx, sy) in [(1, 1), (1, -1), (-1, 1), (-1, -1)] {
                for &a in &p.left {
                    for &b in &p.right {
                        cells.push(&[sx * (a as i32 + 1), sy * (b as i32 + 1)], layout, p.node);
                    }
                }
            }
        }
        cells.seal();
        cells
    }

    /// Every `r`-set of a node's open variables not taken by a smaller node,
    /// with each of its sign patterns. `seen` holds the sets already taken.
    fn sets(
        nodes: &[(usize, Vec<u32>)],
        layout: &Layout,
        r: usize,
        seen: &mut HashSet<[u32; 3]>,
    ) -> Self {
        let mut cells = Cells::empty(r, r == 3);
        let mut lits = vec![0i32; r];
        for (node, open) in nodes {
            for_each_subset(open.len(), r, |ix| {
                let mut key = [u32::MAX; 3];
                for (k, &j) in ix.iter().enumerate() {
                    key[k] = open[j];
                }
                if !seen.insert(key) {
                    return;
                }
                for signs in 0..1u32 << r {
                    for (k, &j) in ix.iter().enumerate() {
                        let v = open[j] as i32 + 1;
                        lits[k] = if (signs >> (r - 1 - k)) & 1 == 0 {
                            v
                        } else {
                            -v
                        };
                    }
                    cells.push(&lits, layout, *node);
                }
            });
        }
        cells.seal();
        cells
    }

    fn len(&self) -> usize {
        self.leaves.len()
    }

    fn lits(&self, i: usize) -> &[i32] {
        &self.lits[i * self.r..(i + 1) * self.r]
    }

    fn count(&self, which: Cell) -> usize {
        self.status.iter().filter(|&&s| s == which).count()
    }

    /// Mark `as_` every open cell some model of `batch` falsifies. Returns the
    /// literals read.
    fn refute(&mut self, batch: &ModelBatch, as_: Cell) -> u64 {
        if batch.count == 0 {
            return 0;
        }
        let full = if batch.count == 64 {
            u64::MAX
        } else {
            (1u64 << batch.count) - 1
        };
        let mut read = 0u64;
        for i in 0..self.len() {
            if !self.status[i].open() {
                continue;
            }
            let mut f = full;
            for &l in self.lits(i) {
                let w = batch.words[l.unsigned_abs() as usize - 1];
                f &= if l > 0 { !w & full } else { w };
            }
            read += self.r as u64;
            if f != 0 {
                self.status[i] = as_;
            }
        }
        read
    }

    /// Mark `Unit` every open cell that holds one of `units` (sorted).
    fn drop_units(&mut self, units: &[i32]) {
        if units.is_empty() {
            return;
        }
        for i in 0..self.len() {
            if self.status[i] == Cell::Open
                && self.lits(i).iter().any(|l| units.binary_search(l).is_ok())
            {
                self.status[i] = Cell::Unit;
            }
        }
    }

    /// Open cells, bottom-up by the size of their node.
    fn order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.len())
            .filter(|&i| self.status[i] == Cell::Open)
            .collect();
        order.sort_by_key(|&i| self.leaves[i]);
        order
    }

    /// Deferred cells become unknown when the last round finished, else short.
    fn close(&mut self, last_done: bool) {
        for s in &mut self.status {
            if *s == Cell::Deferred {
                *s = if last_done {
                    Cell::Unknown
                } else {
                    Cell::Short
                };
            }
        }
    }

    fn counts(&self) -> Counts {
        Counts {
            in_cnf: self.count(Cell::InCnf),
            refuted_by_models: self.count(Cell::Model),
            refuted_by_search: self.count(Cell::SearchModel),
            unit_dropped: self.count(Cell::Unit),
            not_prime: self.count(Cell::NotPrime),
            proven_by_propagation: self.count(Cell::ByPropagation),
            proven_by_search: self.count(Cell::BySearch),
            unknown: self.count(Cell::Unknown),
            short: self.count(Cell::Short),
            unchecked: self.count(Cell::Open),
        }
    }
}

/// Call `f` with each `r`-subset of `0..k` as increasing indices, in
/// lexicographic order.
fn for_each_subset(k: usize, r: usize, mut f: impl FnMut(&[usize])) {
    if r > k {
        return;
    }
    let mut ix: Vec<usize> = (0..r).collect();
    loop {
        f(&ix);
        let Some(j) = (0..r).rev().find(|&j| ix[j] < k - r + j) else {
            return;
        };
        ix[j] += 1;
        for m in j + 1..r {
            ix[m] = ix[m - 1] + 1;
        }
    }
}

/// Mark `NotPrime` every ternary cell in a state `which` accepts that holds a
/// binary clause the pair cells know to be implied. Returns how many.
fn drop_not_prime(pairs: &Cells, triples: &mut Cells, which: impl Fn(Cell) -> bool) -> usize {
    let implied: HashSet<(i32, i32)> = (0..pairs.len())
        .filter(|&i| pairs.status[i].implied())
        .map(|i| {
            let l = pairs.lits(i);
            (l[0], l[1])
        })
        .collect();
    if implied.is_empty() {
        return 0;
    }
    let mut dropped = 0;
    for i in 0..triples.len() {
        if !which(triples.status[i]) {
            continue;
        }
        // Both kinds of cell list their variables in index order.
        let t = triples.lits(i);
        if [(0, 1), (0, 2), (1, 2)]
            .iter()
            .any(|&(a, b)| implied.contains(&(t[a], t[b])))
        {
            triples.status[i] = Cell::NotPrime;
            dropped += 1;
        }
    }
    dropped
}

/// Up to 64 models, one bit per model in each variable's word.
struct ModelBatch {
    words: Vec<u64>,
    count: usize,
}

impl ModelBatch {
    fn new(vars: usize) -> Self {
        ModelBatch {
            words: vec![0; vars],
            count: 0,
        }
    }

    fn push(&mut self, model: &[i32]) {
        let bit = 1u64 << self.count;
        for (w, &v) in self.words.iter_mut().zip(model) {
            if v > 0 {
                *w |= bit;
            }
        }
        self.count += 1;
    }
}

// ── Spending ────────────────────────────────────────────────────────────────

/// Stops a call at the wall deadline, when there is one, or when the
/// drought ends the proof steps.
struct Stop {
    at: Option<Instant>,
    drought_at: Rc<Shared<Option<Instant>>>,
}

impl Terminator for Stop {
    fn terminated(&mut self) -> bool {
        let now = Instant::now();
        self.at.is_some_and(|d| now >= d) || self.drought_at.get().is_some_and(|d| now >= d)
    }
}

/// The allowance and what has been charged against it.
struct Spend {
    meter: PreprocessMeter,
    mark: PhaseMark,
    /// Where the current stage ends, in the clock's currency since `mark`.
    limit: Duration,
    begun: Instant,
    step_started: Instant,
    props: u64,
    scanned: u64,
    step_props: u64,
    step_scanned: u64,
    /// When the last clause that could be emitted was proven, in the clock's
    /// currency since `mark`; the drought counts from here.
    last_proof: Duration,
    /// While the drought is counted under the wall clock, the instant it
    /// ends the proof steps, shared with each [`Stop`].
    drought_at: Rc<Shared<Option<Instant>>>,
    /// When the drought ended the proof steps.
    dried: Option<Duration>,
}

impl Spend {
    fn used(&self) -> Duration {
        self.meter.elapsed(self.mark)
    }

    /// Whether the current stage is over: its limit reached, or, in the proof
    /// steps, the drought.
    fn out(&mut self) -> bool {
        let used = self.used();
        if self.armed() && self.dried.is_none() && used.saturating_sub(self.last_proof) >= DROUGHT {
            self.dried = Some(used);
        }
        used >= self.limit || (self.armed() && self.dried.is_some())
    }

    fn armed(&self) -> bool {
        self.last_proof != Duration::MAX
    }

    /// Count the drought, from the last proof.
    fn arm(&mut self) {
        if !self.meter.deterministic() {
            let silent = self.used().saturating_sub(self.last_proof);
            self.drought_at
                .set(Some(Instant::now() + DROUGHT.saturating_sub(silent)));
        }
    }

    /// Stop counting the drought: the proof steps are over.
    fn disarm(&mut self) {
        self.last_proof = Duration::MAX;
        self.drought_at.set(None);
    }

    /// A clause that could be emitted was proven: the drought restarts.
    fn proved(&mut self) {
        if self.armed() {
            self.last_proof = self.used();
            self.arm();
        }
    }

    /// The wall instant at which the current stage ends; none under the
    /// deterministic clock, where only charged work stops the pass and each
    /// call is bounded by its conflict cap.
    fn stop(&self) -> Stop {
        Stop {
            at: (!self.meter.deterministic())
                .then(|| Instant::now() + self.limit.saturating_sub(self.used())),
            drought_at: Rc::clone(&self.drought_at),
        }
    }

    /// Run one solver call, charging its search propagations.
    fn call(
        &mut self,
        solver: &mut CaDiCal,
        f: impl FnOnce(&mut CaDiCal) -> Status,
    ) -> (Status, i64) {
        let before = solver.search_stats();
        let status = f(solver);
        let delta = solver.search_stats().since(before);
        let props = delta.propagations.max(0) as u64;
        self.props += props;
        self.meter.charge(props);
        (status, delta.conflicts)
    }

    fn scan(&mut self, literals: u64) {
        self.scanned += literals;
        self.meter.charge(literals);
    }

    fn now(&self) -> (u64, u64) {
        (
            self.begun.elapsed().as_micros() as u64,
            self.props + self.scanned,
        )
    }

    fn end_step(&mut self, step: &'static str, steps: &mut Vec<StepLog>) {
        steps.push(StepLog {
            step,
            us: self.step_started.elapsed().as_micros() as u64,
            propagations: self.props - self.step_props,
            scanned: self.scanned - self.step_scanned,
        });
        self.step_started = Instant::now();
        self.step_props = self.props;
        self.step_scanned = self.scanned;
    }
}

// ── Proving ─────────────────────────────────────────────────────────────────

fn add_clause(solver: &mut CaDiCal, lits: &[i32]) {
    for &l in lits {
        solver.add(l);
    }
    solver.add(0);
}

/// Mark candidates already in the formula.
fn mark_in_cnf(formula: &CnfFormula, cells: &mut Cells) {
    let key = |mut c: Vec<i32>| {
        c.sort_unstable();
        c
    };
    let have: HashSet<Vec<i32>> = formula
        .clauses()
        .iter()
        .filter(|c| c.literals.len() == cells.r)
        .map(|c| key(c.literals.iter().map(|l| l.to_dimacs()).collect()))
        .collect();
    if have.is_empty() {
        return;
    }
    for i in 0..cells.len() {
        if have.contains(&key(cells.lits(i).to_vec())) {
            cells.status[i] = Cell::InCnf;
        }
    }
}

/// The seed model and up to `MODELS - 1` more with random phases; none when
/// the formula is unsatisfiable or no model is found.
fn draw_models(
    solver: &mut CaDiCal,
    n: usize,
    stop: Stop,
    spend: &mut Spend,
    log: &mut PassLog,
) -> Option<ModelBatch> {
    let mut bounded = Bounded::new(solver, stop);
    let mut batch = ModelBatch::new(n);
    let mut rng = SmallRng::seed_from_u64(PHASE_SEED);
    let last_cap = PAIR_CAPS[PAIR_CAPS.len() - 1];
    for k in 0..MODELS {
        if k > 0 && (batch.count == 0 || spend.out()) {
            break;
        }
        if k > 0 {
            use rand::RngExt;
            for v in 1..=n as i32 {
                bounded.phase(if rng.random::<bool>() { v } else { -v });
            }
        }
        bounded.limit(c"conflicts", last_cap);
        match spend.call(&mut bounded, CaDiCal::solve).0 {
            Status::Satisfiable => batch.push(&read_model(&mut bounded, n)),
            Status::Unsatisfiable => {
                log.unsat = true;
                break;
            }
            Status::Unknown => {}
        }
    }
    for v in 1..=n as i32 {
        bounded.unphase(v);
    }
    log.models = batch.count;
    spend.end_step("models", &mut log.steps);
    (batch.count > 0).then_some(batch)
}

/// Literals the formula implies on its own among those of the open cells:
/// candidates holding one are dropped, not proven. Returns them sorted.
fn find_units(solver: &mut CaDiCal, cells: &[&Cells], spend: &mut Spend) -> Vec<i32> {
    let mut lits: Vec<i32> = cells
        .iter()
        .flat_map(|c| {
            (0..c.len())
                .filter(|&i| c.status[i] == Cell::Open)
                .flat_map(|i| c.lits(i).to_vec())
        })
        .collect();
    lits.sort_unstable();
    lits.dedup();
    let mut units = Vec::new();
    for l in lits {
        if spend.out() {
            break;
        }
        solver.assume(-l);
        if spend.call(solver, CaDiCal::propagate).0 == Status::Unsatisfiable {
            units.push(l);
        }
    }
    units
}

/// The pair step: models, units, then the proof rounds; every proven clause
/// is added to the solver.
fn prove_pairs(
    formula: &CnfFormula,
    solver: &mut CaDiCal,
    cells: &mut Cells,
    batch: &ModelBatch,
    spend: &mut Spend,
    log: &mut PassLog,
) {
    let n = formula.num_vars() as usize;
    mark_in_cnf(formula, cells);
    let read = cells.refute(batch, Cell::Model);
    spend.scan(read);
    spend.end_step("candidates", &mut log.steps);

    let units = find_units(solver, &[cells], spend);
    cells.drop_units(&units);
    log.units = units;
    spend.end_step("units", &mut log.steps);

    let order = cells.order();
    let mut done = true;
    let mut last_done = false;
    for (round, &cap) in PAIR_CAPS.iter().enumerate() {
        if !done {
            break;
        }
        let before = cells.count(Cell::BySearch);
        let mut rlog = RoundLog {
            cap,
            ..RoundLog::default()
        };
        done = search_round(
            solver,
            cells,
            &order,
            cap,
            n,
            spend,
            &mut rlog,
            &mut log.calls,
            &mut |_: &[i32], _: &mut Spend| {},
        );
        if done && cells.count(Cell::BySearch) > before {
            let (proven, finished) = propagation_pass(solver, cells, &order, spend);
            rlog.propagation_after = proven;
            done = finished;
        }
        rlog.open_after = order.iter().filter(|&&i| cells.status[i].open()).count();
        log.rounds.push(rlog);
        last_done = done && round == PAIR_CAPS.len() - 1;
    }
    cells.close(last_done);
    spend.end_step("prove", &mut log.steps);
    log.counts = cells.counts();
    log.budget_hit = spend.out() || log.counts.short > 0;
}

/// The triple step's proofs: per round, the binary candidates, then the
/// ternary ones that hold no proven binary clause.
fn prove_triples(
    formula: &CnfFormula,
    solver: &mut CaDiCal,
    pairs: &mut Cells,
    triples: &mut Cells,
    batch: &ModelBatch,
    spend: &mut Spend,
    log: &mut TripleLog,
) {
    let n = formula.num_vars() as usize;
    for cells in [&mut *pairs, &mut *triples] {
        mark_in_cnf(formula, cells);
        let read = cells.refute(batch, Cell::Model);
        spend.scan(read);
    }
    spend.end_step("candidates", &mut log.steps);

    let units = find_units(solver, &[&*pairs, &*triples], spend);
    pairs.drop_units(&units);
    triples.drop_units(&units);
    log.units = units;
    spend.end_step("units", &mut log.steps);

    let porder = pairs.order();
    let torder = triples.order();
    // Models the binary calls return reach the ternary cells 64 at a time,
    // before each ternary round.
    let mut queued: Vec<Vec<i32>> = Vec::new();
    let mut done = true;
    let mut last_done = false;
    for (round, &cap) in TRIPLE_CAPS.iter().enumerate() {
        if !done {
            break;
        }
        let mut row = TripleRound {
            cap,
            pairs: RoundLog {
                cap,
                ..RoundLog::default()
            },
            not_prime: 0,
            triples: RoundLog {
                cap,
                ..RoundLog::default()
            },
        };
        let before = pairs.count(Cell::BySearch);
        done = search_round(
            solver,
            pairs,
            &porder,
            cap,
            n,
            spend,
            &mut row.pairs,
            &mut log.pair_calls,
            &mut |model: &[i32], _: &mut Spend| queued.push(model.to_vec()),
        );
        if done && pairs.count(Cell::BySearch) > before {
            let (proven, finished) = propagation_pass(solver, pairs, &porder, spend);
            row.pairs.propagation_after = proven;
            done = finished;
        }
        row.pairs.open_after = porder.iter().filter(|&&i| pairs.status[i].open()).count();
        row.not_prime = drop_not_prime(pairs, triples, Cell::open);
        for chunk in queued.chunks(64) {
            let mut b = ModelBatch::new(n);
            for m in chunk {
                b.push(m);
            }
            let read = triples.refute(&b, Cell::SearchModel);
            spend.scan(read);
        }
        queued.clear();
        if done {
            let before = triples.count(Cell::BySearch);
            done = search_round(
                solver,
                triples,
                &torder,
                cap,
                n,
                spend,
                &mut row.triples,
                &mut log.triple_calls,
                &mut |model: &[i32], spend: &mut Spend| {
                    let mut one = ModelBatch::new(n);
                    one.push(model);
                    let read = pairs.refute(&one, Cell::SearchModel);
                    spend.scan(read);
                },
            );
            if done && triples.count(Cell::BySearch) > before {
                let (proven, finished) = propagation_pass(solver, triples, &torder, spend);
                row.triples.propagation_after = proven;
                done = finished;
            }
        }
        row.triples.open_after = torder.iter().filter(|&&i| triples.status[i].open()).count();
        log.rounds.push(row);
        last_done = done && round == TRIPLE_CAPS.len() - 1;
    }
    log.not_prime_late = drop_not_prime(pairs, triples, Cell::proven);
    pairs.close(last_done);
    triples.close(last_done);
    spend.end_step("prove", &mut log.steps);
    log.pairs = pairs.counts();
    log.triples = triples.counts();
    log.budget_hit = spend.out() || log.pairs.short + log.triples.short > 0;
}

fn mark_proven(solver: &mut CaDiCal, cells: &mut Cells, i: usize, how: Cell, spend: &mut Spend) {
    cells.status[i] = how;
    cells.proven_at[i] = spend.now();
    cells.proofs.push(cells.proven_at[i]);
    if cells.emits {
        spend.proved();
    }
    add_clause(solver, cells.lits(i));
}

/// Whether propagation under the negation of cell `i` refutes it.
fn propagation_refutes(solver: &mut CaDiCal, cells: &Cells, i: usize, spend: &mut Spend) -> Status {
    for &l in cells.lits(i) {
        solver.assume(-l);
    }
    spend.call(solver, CaDiCal::propagate).0
}

/// One round at conflict cap `cap` over the open cells of `order`. Every
/// model an answer gives refutes the open cells and is passed to `also`.
/// Returns whether it finished inside the allowance.
#[allow(clippy::too_many_arguments)]
fn search_round(
    solver: &mut CaDiCal,
    cells: &mut Cells,
    order: &[usize],
    cap: i32,
    n: usize,
    spend: &mut Spend,
    rlog: &mut RoundLog,
    calls: &mut Vec<Call>,
    also: &mut OnModel<'_>,
) -> bool {
    for &i in order {
        if !cells.status[i].open() {
            continue;
        }
        if spend.out() {
            return false;
        }
        match propagation_refutes(solver, cells, i, spend) {
            Status::Unsatisfiable => {
                mark_proven(solver, cells, i, Cell::ByPropagation, spend);
                rlog.by_propagation += 1;
                continue;
            }
            Status::Satisfiable => {
                refute_by_answer(solver, cells, n, spend, also);
                continue;
            }
            Status::Unknown => {}
        }
        rlog.calls += 1;
        for &l in cells.lits(i) {
            solver.assume(-l);
        }
        solver.limit(c"conflicts", cap);
        let started = Instant::now();
        let (status, conflicts) = spend.call(solver, CaDiCal::solve);
        let us = started.elapsed().as_micros() as u64;
        rlog.max_over_cap = rlog.max_over_cap.max(conflicts - i64::from(cap));
        let answer = match status {
            Status::Unsatisfiable => {
                mark_proven(solver, cells, i, Cell::BySearch, spend);
                rlog.by_search += 1;
                "unsat"
            }
            Status::Satisfiable => {
                refute_by_answer(solver, cells, n, spend, also);
                rlog.refuted_by_search += 1;
                "sat"
            }
            Status::Unknown => "unknown",
        };
        calls.push((cap, cells.leaves[i], answer, conflicts, us));
        if status == Status::Unknown {
            if spend.out() {
                cells.status[i] = Cell::Short;
                return false;
            }
            cells.status[i] = Cell::Deferred;
            rlog.deferred += 1;
        }
    }
    true
}

fn refute_by_answer(
    solver: &mut CaDiCal,
    cells: &mut Cells,
    n: usize,
    spend: &mut Spend,
    also: &mut OnModel<'_>,
) {
    let model = read_model(solver, n);
    let mut one = ModelBatch::new(n);
    one.push(&model);
    let read = cells.refute(&one, Cell::SearchModel);
    spend.scan(read);
    also(&model, spend);
}

/// One pass of propagation alone over the open cells of `order`. Returns the
/// clauses it proved and whether it finished inside the allowance.
fn propagation_pass(
    solver: &mut CaDiCal,
    cells: &mut Cells,
    order: &[usize],
    spend: &mut Spend,
) -> (usize, bool) {
    let mut proven = 0;
    for &i in order {
        if !cells.status[i].open() {
            continue;
        }
        if spend.out() {
            return (proven, false);
        }
        if propagation_refutes(solver, cells, i, spend) == Status::Unsatisfiable {
            mark_proven(solver, cells, i, Cell::ByPropagation, spend);
            proven += 1;
        }
    }
    (proven, true)
}

// ── The local check ─────────────────────────────────────────────────────────

/// The proven ternary cells that the clauses inside their home node, the
/// formula's and the kept binary ones, do not imply: those a capped search
/// over that part alone finds a model against. One solver holds every clause
/// guarded by a selector of its lowest common ancestor, and a node's selector
/// switches on its children's, so assuming a node's selector switches on
/// exactly the clauses inside it.
fn local_check(
    formula: &CnfFormula,
    vtree: &Vtree,
    layout: &Layout,
    kept_pairs: &[Vec<i32>],
    triples: &Cells,
    spend: &mut Spend,
    log: &mut TripleLog,
) -> Vec<usize> {
    let proven: Vec<usize> = (0..triples.len())
        .filter(|&i| triples.status[i].proven())
        .collect();
    let mut emitted = Vec::new();
    if proven.is_empty() {
        spend.end_step("local", &mut log.steps);
        return emitted;
    }
    let Some(mut solver) = CaDiCal::new() else {
        log.local.unchecked = proven.len();
        return emitted;
    };
    let base = formula.num_vars() as i32 + 1;
    let select = |t: VtreeIdx| base + t.0 as i32;
    for t in 0..vtree.num_nodes() {
        let t = VtreeIdx(t as u32);
        if let Some(p) = vtree.node(t).parent() {
            add_clause(&mut solver, &[-select(p), select(t)]);
        }
    }
    let mut guarded = Vec::new();
    let clauses = formula
        .clauses()
        .iter()
        .map(|c| {
            c.literals
                .iter()
                .map(|l| l.to_dimacs())
                .collect::<Vec<i32>>()
        })
        .chain(kept_pairs.iter().cloned());
    for c in clauses {
        let (t, _, _) = layout.lca(vtree, &c);
        guarded.clear();
        guarded.extend_from_slice(&c);
        guarded.push(-select(t));
        add_clause(&mut solver, &guarded);
    }
    let cap = TRIPLE_CAPS[TRIPLE_CAPS.len() - 1];
    let stop = spend.stop();
    let mut solver = Bounded::new(&mut solver, stop);
    for (k, &i) in proven.iter().enumerate() {
        if spend.out() {
            log.local.unchecked = proven.len() - k;
            break;
        }
        let home = select(layout.ids[triples.node[i] as usize]);
        let assume = |s: &mut CaDiCal| {
            s.assume(home);
            for &l in triples.lits(i) {
                s.assume(-l);
            }
        };
        assume(&mut solver);
        if spend.call(&mut solver, CaDiCal::propagate).0 == Status::Unsatisfiable {
            log.local.by_propagation += 1;
            continue;
        }
        log.local.calls += 1;
        assume(&mut solver);
        solver.limit(c"conflicts", cap);
        let (status, conflicts) = spend.call(&mut solver, CaDiCal::solve);
        log.local.max_over_cap = log.local.max_over_cap.max(conflicts - i64::from(cap));
        match status {
            Status::Unsatisfiable => log.local.by_search += 1,
            Status::Satisfiable => emitted.push(i),
            Status::Unknown if spend.out() => {
                log.local.unchecked = proven.len() - k;
                break;
            }
            Status::Unknown => log.local.unknown += 1,
        }
    }
    spend.end_step("local", &mut log.steps);
    emitted
}

// ── Minimizing ──────────────────────────────────────────────────────────────

/// Of `cands`, taken in the given order, the ones that unit propagation
/// inside their window, over the formula, the `fixed` clauses and the other
/// candidates not yet dropped, does not already prove. A window is the
/// leaves `lo..hi` of the clause's lowest common ancestor. Not bounded by the
/// allowance; its work is charged.
fn minimize(
    formula: &CnfFormula,
    layout: &Layout,
    fixed: &[Vec<i32>],
    cands: &[(&[i32], u32, u32)],
    spend: &mut Spend,
) -> Vec<bool> {
    let mut units = ScopedUnits::new(formula.num_vars() as usize);
    let var = |l: &i32| l.unsigned_abs() as usize - 1;
    for clause in formula.clauses() {
        let lits: Vec<i32> = clause.literals.iter().map(|l| l.to_dimacs()).collect();
        let (lo, hi) = layout.span_of(lits.iter().map(var));
        units.push(&lits, lo, hi);
    }
    for c in fixed {
        let (lo, hi) = layout.span_of(c.iter().map(var));
        units.push(c, lo, hi);
    }
    let ids: Vec<usize> = cands
        .iter()
        .map(|(lits, _, _)| {
            let (lo, hi) = layout.span_of(lits.iter().map(var));
            units.push(lits, lo, hi)
        })
        .collect();
    let mut keep = vec![true; cands.len()];
    let mut negated = Vec::new();
    for (k, &(lits, lo, hi)) in cands.iter().enumerate() {
        units.set_active(ids[k], false);
        negated.clear();
        negated.extend(lits.iter().map(|l| -l));
        let (refuted, read) = units.refutes(&negated, lo, hi);
        spend.scan(read);
        if refuted {
            keep[k] = false;
        } else {
            units.set_active(ids[k], true);
        }
    }
    keep
}

/// The kept proven pair cells: largest nodes first, a node's cells in
/// candidate order.
fn minimize_pairs(
    formula: &CnfFormula,
    layout: &Layout,
    cells: &Cells,
    spend: &mut Spend,
    steps: &mut Vec<StepLog>,
) -> Vec<usize> {
    let mut proven: Vec<usize> = (0..cells.len())
        .filter(|&i| cells.status[i].proven())
        .collect();
    proven.sort_by_key(|&i| (std::cmp::Reverse(cells.leaves[i]), i));
    let cands: Vec<(&[i32], u32, u32)> = proven
        .iter()
        .map(|&i| (cells.lits(i), cells.span[i].lo, cells.span[i].hi))
        .collect();
    let keep = minimize(formula, layout, &[], &cands, spend);
    spend.end_step("minimize", steps);
    proven
        .into_iter()
        .zip(keep)
        .filter_map(|(i, k)| k.then_some(i))
        .collect()
}

/// The kept emitted triple cells, over the formula and the kept binary
/// clauses: largest lowest common ancestor first, then by literals.
#[allow(clippy::too_many_arguments)]
fn minimize_triples(
    formula: &CnfFormula,
    vtree: &Vtree,
    layout: &Layout,
    kept_pairs: &[Vec<i32>],
    cells: &Cells,
    emitted: &[usize],
    spend: &mut Spend,
    steps: &mut Vec<StepLog>,
) -> Vec<usize> {
    let mut order: Vec<(usize, u32, u32)> = emitted
        .iter()
        .map(|&i| {
            let (_, lo, hi) = layout.lca(vtree, cells.lits(i));
            (i, lo, hi)
        })
        .collect();
    order.sort_by(|a, b| {
        (b.2 - b.1)
            .cmp(&(a.2 - a.1))
            .then_with(|| cells.lits(a.0).cmp(cells.lits(b.0)))
    });
    let cands: Vec<(&[i32], u32, u32)> = order
        .iter()
        .map(|&(i, lo, hi)| (cells.lits(i), lo, hi))
        .collect();
    let keep = minimize(formula, layout, kept_pairs, &cands, spend);
    spend.end_step("minimize", steps);
    order
        .iter()
        .zip(keep)
        .filter_map(|(o, k)| k.then_some(o.0))
        .collect()
}

fn add_formula(solver: &mut CaDiCal, formula: &CnfFormula) {
    for clause in formula.clauses() {
        for lit in &clause.literals {
            solver.add(lit.to_dimacs());
        }
        solver.add(0);
    }
}
