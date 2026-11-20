//! Clauses a component's CNF implies, found over the vtree built for it.
//!
//! The pass runs after construction and reads the finished tree, so the tree is
//! the same whether it runs or not. A variable is open at an internal node when
//! some clause holding it reaches outside the node's leaves.
//!
//! It has two steps over one solver session. The pair step's candidates are
//! the binary clauses over a variable open in a node's left child and one open
//! in its right child, all four sign patterns, at every internal node. The
//! triple step's are the ternary clauses over three variables open at one node,
//! each at the smallest such node (its home), with the binary clauses over two
//! such variables alongside: a ternary clause that holds a proven binary one is
//! not prime and is dropped. At a node with more than 20 open variables the
//! triple step takes the 20 that occur in the most clauses.
//!
//! A candidate is dropped when a model of the formula falsifies it: first 32
//! models drawn from the solver with random phases, then every model a search
//! call returns. Of the candidates the first 32 leave that are not clauses of
//! the formula, each step keeps, per clause length, at most 8,192 at a node and
//! 65,536 in the component, those of highest score. The score is how unlikely it
//! is, were the clause's literals independent with their frequencies in the
//! models, that no model falsifies it, plus the log of one plus the number of
//! the formula's clauses (of at most 64 literals) that hold two of its
//! variables, summed over its pairs of variables.
//!
//! In the pair step, up to 16,384 of the kept candidates are first tried by
//! unit propagation alone under the negated clause, on a solver of their own:
//! the half of highest score, best first, then the other half spread evenly
//! over the rest in score order. Then the kept candidates are proven one by
//! one, bottom-up by the size of the node, by unit propagation under the
//! negated clause or else by a search capped at a conflict limit; a candidate
//! the cap stops waits for the next round at a ten times larger cap. In the
//! triple step each round proves the binary candidates first, then the ternary
//! ones. Every proven clause is added to the solver, so later proofs use it. A
//! candidate that holds a literal the formula implies on its own is dropped
//! rather than proven.
//!
//! A proven ternary clause is emitted only when the clauses inside its home,
//! with the emitted binary clauses, do not already imply it. Last, in each step
//! a proven clause is dropped when unit propagation over the formula's clauses
//! and the other kept clauses that lie inside its lowest common ancestor's
//! leaves already refutes its negation.
//!
//! Once a fifth of the run's whole allowance passes with no clause proven that
//! could be emitted (a binary one in the pair step, a ternary one in the
//! triple step), counted from the end of the pair step's propagation tries,
//! both proof steps end and the local check and minimizing run on what was
//! proven: 400 ms of a 2,000 ms allowance. Drawing the models also ends once
//! that long passes; gathering the candidates, the screen for implied
//! literals and the propagation tries are bounded by their stage alone.
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
use std::fmt;
use std::rc::Rc;
use std::time::{Duration, Instant};

use rand::SeedableRng;
use rand::rngs::SmallRng;

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
/// Candidates a step keeps at one node, per clause length.
const NODE_KEEP: usize = 8_192;
/// Candidates a step keeps in one component, per clause length.
const COMPONENT_KEEP: usize = 65_536;
/// The pair step's candidates tried by propagation alone before its search
/// rounds and their drought.
const PROPAGATION_TRIES: usize = 16_384;
/// The share of a step's stage that gathering its candidates may take.
const GATHER_SHARE: f64 = 0.5;
/// The triple step takes at most this many of a node's open variables: those
/// in the most clauses.
const TRIPLE_VARS: usize = 20;
/// Longer clauses of the formula do not count among the clauses two variables
/// share.
const SHARED_MAX_LEN: usize = 64;
/// The share of the triple step's time kept for the local check.
const LOCAL_SHARE: f64 = 0.1;
/// Seed of the random phases.
const PHASE_SEED: u64 = 1;
/// The proof steps end once the run's whole allowance divided by this passes,
/// in the clock's currency, with no clause proven that could be emitted.
const DROUGHT_PARTS: u32 = 5;
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
    /// How long the proof steps go on with no clause proven that could be
    /// emitted: the whole run's allowance over [`DROUGHT_PARTS`].
    pub drought: Duration,
}

impl Allowance {
    /// `nominal` out of a run whose whole allowance is `total`.
    pub(crate) fn of_run(
        clock: PreprocessClock,
        nominal: Duration,
        total: Duration,
        deadline: Option<Instant>,
    ) -> Self {
        Allowance {
            clock,
            nominal,
            deadline,
            drought: total / DROUGHT_PARTS,
        }
    }
}

/// The clauses one call found, and how the call went.
pub(crate) struct Found {
    /// Emitted clauses, binary and ternary, sorted, over the formula's own
    /// variables. Empty when `dropped`.
    pub clauses: Vec<Clause>,
    /// Binary clauses found, counted before any drop.
    pub binary: usize,
    /// Ternary clauses found, counted before any drop.
    pub ternary: usize,
    /// What the pass spent, in the clock's currency.
    pub spent: Duration,
    /// What ended the pass early; none when it ran to the end.
    pub stopped_by: Option<StoppedBy>,
    /// Clauses found whose lowest common ancestor's leaves hold no clause of
    /// the formula.
    pub at_empty_nodes: usize,
    /// Whether the set was dropped for having too many of those
    /// ([`MAX_EMPTY_SHARE`]).
    pub dropped: bool,
}

/// What ended a call of the pass early.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StoppedBy {
    /// The allowance's drought passed with no clause proven that could be
    /// emitted.
    Drought,
    /// The allowance ran out.
    Allowance,
}

impl fmt::Display for StoppedBy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            StoppedBy::Drought => "drought",
            StoppedBy::Allowance => "allowance",
        })
    }
}

/// What a proof round does with each model an answer gives, beyond refuting
/// its own cells.
type OnModel<'a> = dyn FnMut(&[i32], &mut Spend) + 'a;

/// [`find`] over each of `parts` in turn, `total` divided among them, each
/// set then kept or dropped whole by [`drop_if_mostly_at_empty_nodes`]. Each
/// gets the share of what is left that its candidate count, as the caps bound
/// it, is of the candidates left, so what one leaves unspent goes to the later
/// ones. A part with no candidates is skipped (`None`).
pub(crate) fn find_shared(
    parts: &[(&CnfFormula, &Vtree)],
    total: Duration,
    clock: PreprocessClock,
    deadline: Option<Instant>,
) -> Vec<Option<Found>> {
    let cells: Vec<u64> = parts.iter().map(|(f, t)| candidate_cells(f, t)).collect();
    let mut cells_left: u64 = cells.iter().sum();
    let mut spent = Duration::ZERO;
    let mut out = Vec::with_capacity(parts.len());
    for ((f, t), c) in parts.iter().zip(cells) {
        if c == 0 {
            out.push(None);
            continue;
        }
        let left = total.saturating_sub(spent);
        let share = left.mul_f64(c as f64 / cells_left as f64);
        cells_left -= c;
        let found = find(f, t, &Allowance::of_run(clock, share, total, deadline));
        spent += found.spent;
        out.push(Some(drop_if_mostly_at_empty_nodes(found)));
    }
    out
}

/// Static count of the pair step's candidates of `formula` over `vtree`, at
/// most [`NODE_KEEP`] per node and [`COMPONENT_KEEP`] in all: what the run's
/// allowance is divided by.
pub(crate) fn candidate_cells(formula: &CnfFormula, vtree: &Vtree) -> u64 {
    let layout = Layout::new(vtree);
    let reach = Reach::new(formula, &layout);
    let open = OpenCounts::new(vtree, &layout, &reach);
    let cells: u64 = (0..layout.nodes.len())
        .map(|i| (4 * open.left[i] as u64 * open.right[i] as u64).min(NODE_KEEP as u64))
        .sum();
    cells.min(COMPONENT_KEEP as u64)
}

/// `found` with no clauses when more than [`MAX_EMPTY_SHARE`] of them have a
/// lowest common ancestor whose leaves hold no clause of the formula.
fn drop_if_mostly_at_empty_nodes(mut found: Found) -> Found {
    if found.at_empty_nodes as f64 > MAX_EMPTY_SHARE * found.clauses.len() as f64 {
        found.dropped = true;
        found.clauses.clear();
    }
    found
}

/// Binary clauses `formula` implies whose two variables are open at their
/// lowest common ancestor in `vtree`, and ternary clauses it implies over
/// three variables open at one node, found within `allowance`.
pub(crate) fn find(formula: &CnfFormula, vtree: &Vtree, allowance: &Allowance) -> Found {
    let layout = Layout::new(vtree);
    let reach = Reach::new(formula, &layout);
    let open = OpenCounts::new(vtree, &layout, &reach);

    let meter = PreprocessMeter::new(allowance.clock).with_backstop(allowance.deadline);
    let allowed = meter.clamp(allowance.nominal, allowance.deadline);
    let mark = meter.begin(PreprocessPhase::Implicates, allowed);
    let mut spend = Spend::new(meter, mark, allowed / 2, allowance.drought);
    let tree = Tree {
        formula,
        vtree,
        layout: &layout,
        reach: &reach,
        open: &open,
    };
    let (mut clauses, ran_out) = if open.any() {
        spend.arm();
        run_steps(&tree, &mut spend, allowed)
    } else {
        (Vec::new(), false)
    };
    let stopped_by = if spend.dried.is_some() {
        Some(StoppedBy::Drought)
    } else if ran_out {
        Some(StoppedBy::Allowance)
    } else {
        None
    };
    clauses.sort();
    Found {
        binary: clauses.iter().filter(|c| c.len() == 2).count(),
        ternary: clauses.iter().filter(|c| c.len() == 3).count(),
        at_empty_nodes: at_empty_nodes(formula, vtree, &layout, &clauses),
        clauses: clauses
            .into_iter()
            .map(|c| Clause::new(c.into_iter().map(Literal::from).collect()))
            .collect(),
        spent: spend.used(),
        stopped_by,
        dropped: false,
    }
}

/// The component and what the steps read of its tree.
struct Tree<'a> {
    formula: &'a CnfFormula,
    vtree: &'a Vtree,
    layout: &'a Layout,
    reach: &'a Reach,
    open: &'a OpenCounts,
}

/// Both steps within `allowed`, the pair step taking up to half: the kept
/// clauses, unsorted, and whether the allowance ran out in a proof step.
fn run_steps(tree: &Tree<'_>, spend: &mut Spend, allowed: Duration) -> (Vec<Vec<i32>>, bool) {
    let Tree {
        formula,
        vtree,
        layout,
        ..
    } = *tree;
    let Some(mut solver) = CaDiCal::new() else {
        note_solver_unavailable("implicates", "no implied clauses are emitted");
        return (Vec::new(), false);
    };
    let n = formula.num_vars() as usize;
    add_formula(&mut solver, formula);
    let Some(batch) = draw_models(&mut solver, n, spend) else {
        return (Vec::new(), false);
    };
    spend.disarm();
    let mut rank = Rank::new(formula, &batch);
    let mut pairs = pair_cells(tree, &batch, &mut rank, spend);
    let mut ran_out = prove_pairs(
        formula,
        &mut Bounded::new(&mut solver, spend.stop()),
        &mut pairs,
        &mut rank,
        spend,
    );
    let kept_pairs = minimize_pairs(formula, layout, &pairs, spend);
    let used = spend.used();
    spend.limit = used + allowed.saturating_sub(used).mul_f64(1.0 - LOCAL_SHARE);
    if spend.dried.is_some() {
        return (kept_pairs, ran_out);
    }
    let nodes = triple_nodes(tree, spend);
    let mut seen = HashSet::new();
    let mut small_pairs = set_cells(tree, &nodes, 2, &mut seen, &batch, &mut rank, spend);
    let mut triples = set_cells(tree, &nodes, 3, &mut seen, &batch, &mut rank, spend);
    if triples.len() == 0 {
        return (kept_pairs, ran_out);
    }
    ran_out |= prove_triples(
        formula,
        &mut Bounded::new(&mut solver, spend.stop()),
        &mut small_pairs,
        &mut triples,
        spend,
    );
    drop(solver);
    spend.disarm();
    spend.limit = allowed;
    let emitted = local_check(formula, vtree, layout, &kept_pairs, &triples, spend);
    let kept_triples = minimize_triples(
        formula,
        vtree,
        layout,
        &kept_pairs,
        &triples,
        &emitted,
        spend,
    );
    (
        kept_pairs.into_iter().chain(kept_triples).collect(),
        ran_out,
    )
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

/// Per entry of [`Layout::nodes`], how many variables of its left and of its
/// right child are open at it.
struct OpenCounts {
    left: Vec<u32>,
    right: Vec<u32>,
}

impl OpenCounts {
    /// A variable is open at the internal nodes from its leaf up to, but not
    /// including, the lowest node whose leaves hold every clause it is in (its
    /// closing node). So a child's variables open at its parent are its leaves
    /// less those that close inside it or at the parent.
    fn new(vtree: &Vtree, layout: &Layout, reach: &Reach) -> Self {
        let leaf = |p: u32| vtree.leaf_of(VarId::from_idx(layout.order[p as usize] as usize));
        // Per vtree node: the variables that close inside it, and those of its
        // left and right child that close at it.
        let mut inside = vec![0u32; vtree.num_nodes()];
        let mut from_left = vec![0u32; vtree.num_nodes()];
        let mut from_right = vec![0u32; vtree.num_nodes()];
        for v in 0..layout.pos.len() {
            let c = vtree.lca(leaf(reach.lo[v]), leaf(reach.hi[v]));
            inside[c.idx()] += 1;
            if let VtreeNode::Internal { left, .. } = *vtree.node(c) {
                if layout.pos[v] < layout.hi[left.idx()] {
                    from_left[c.idx()] += 1;
                } else {
                    from_right[c.idx()] += 1;
                }
            }
        }
        for t in vtree.bottomup() {
            if let VtreeNode::Internal { left, right, .. } = *vtree.node(t) {
                inside[t.idx()] += inside[left.idx()] + inside[right.idx()];
            }
        }
        let open = |child: VtreeIdx, at_parent: u32| {
            layout.hi[child.idx()] - layout.lo[child.idx()] - inside[child.idx()] - at_parent
        };
        let (mut l, mut r) = (Vec::new(), Vec::new());
        for &t in &layout.ids {
            if let VtreeNode::Internal { left, right, .. } = *vtree.node(t) {
                l.push(open(left, from_left[t.idx()]));
                r.push(open(right, from_right[t.idx()]));
            }
        }
        OpenCounts { left: l, right: r }
    }

    /// Whether either step has a candidate.
    fn any(&self) -> bool {
        self.left
            .iter()
            .zip(&self.right)
            .any(|(&l, &r)| l * r > 0 || l + r >= 3)
    }
}

/// The open variables of node `node`'s two children, in leaf order.
fn open_children(tree: &Tree<'_>, node: usize) -> (Vec<u32>, Vec<u32>) {
    let s = tree.layout.nodes[node];
    let open = |range: std::ops::Range<u32>| -> Vec<u32> {
        range
            .filter_map(|p| tree.reach.open_at(tree.layout, s, p))
            .collect()
    };
    (open(s.lo..s.mid), open(s.mid..s.hi))
}

/// The triple step's nodes, fewest leaves first, each with its open variables
/// in index order, at most [`TRIPLE_VARS`] of them: those in the most clauses.
fn triple_nodes(tree: &Tree<'_>, spend: &mut Spend) -> Vec<(usize, Vec<u32>)> {
    let mut occurs = vec![0u32; tree.formula.num_vars() as usize];
    for clause in tree.formula.clauses() {
        for lit in &clause.literals {
            occurs[lit.var.idx()] += 1;
        }
    }
    let mut out = Vec::new();
    for node in 0..tree.layout.nodes.len() {
        if tree.open.left[node] + tree.open.right[node] < 2 {
            continue;
        }
        if spend.out() {
            break;
        }
        spend.scan(tree.layout.nodes[node].leaves() as u64);
        let (mut open, right) = open_children(tree, node);
        open.extend(right);
        if open.len() > TRIPLE_VARS {
            open.sort_unstable_by_key(|&v| (std::cmp::Reverse(occurs[v as usize]), v));
            open.truncate(TRIPLE_VARS);
        }
        open.sort_unstable();
        out.push((node, open));
    }
    out.sort_by_key(|(node, _)| tree.layout.nodes[*node].leaves());
    out
}

// ── Candidates ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cell {
    Open,
    /// Unknown at the cap of its last round: retried in the next round, if
    /// there is one.
    Deferred,
    /// Falsified by a model.
    Refuted,
    /// Holds a literal the formula implies.
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
    span: Vec<Span>,
    /// The home node, an index into [`Layout::nodes`].
    node: Vec<u32>,
    status: Vec<Cell>,
    /// Whether a proof here could be emitted, and so ends a drought.
    emits: bool,
}

impl Cells {
    fn empty(r: usize, emits: bool) -> Self {
        Cells {
            r,
            emits,
            lits: Vec::new(),
            span: Vec::new(),
            node: Vec::new(),
            status: Vec::new(),
        }
    }

    fn push(&mut self, lits: &[i32], layout: &Layout, node: usize) {
        self.lits.extend_from_slice(lits);
        self.span.push(layout.nodes[node]);
        self.node.push(node as u32);
        self.status.push(Cell::Open);
    }

    fn len(&self) -> usize {
        self.status.len()
    }

    fn lits(&self, i: usize) -> &[i32] {
        &self.lits[i * self.r..(i + 1) * self.r]
    }

    fn count(&self, which: Cell) -> usize {
        self.status.iter().filter(|&&s| s == which).count()
    }

    /// Indices of the cells in state `which`.
    fn with(&self, which: fn(Cell) -> bool) -> impl Iterator<Item = usize> + '_ {
        (0..self.len()).filter(move |&i| which(self.status[i]))
    }

    /// Mark `Refuted` every open cell some model of `batch` falsifies. Returns
    /// the literals read.
    fn refute(&mut self, batch: &ModelBatch) -> u64 {
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
                self.status[i] = Cell::Refuted;
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
        let mut order: Vec<usize> = self.with(|s| s == Cell::Open).collect();
        order.sort_by_key(|&i| self.span[i].leaves());
        order
    }
}

// ── Ranking ─────────────────────────────────────────────────────────────────

/// What scores a candidate: the models' literal frequencies and the clauses
/// the formula's variables share.
struct Rank<'a> {
    formula: &'a CnfFormula,
    models: f64,
    /// Per variable, the share of the models in which it is false.
    false_share: Vec<f64>,
    /// Per variable, each variable it shares a clause of at most
    /// [`SHARED_MAX_LEN`] literals with and in how many, by variable; built
    /// when a cap first binds.
    shared: Option<Vec<Vec<(u32, u32)>>>,
}

impl<'a> Rank<'a> {
    fn new(formula: &'a CnfFormula, batch: &ModelBatch) -> Self {
        let k = batch.count as u32;
        Rank {
            formula,
            models: f64::from(k),
            false_share: batch
                .words
                .iter()
                .map(|w| f64::from(k - w.count_ones()) / f64::from(k))
                .collect(),
            shared: None,
        }
    }

    /// `-k ln(1 - q)`, where `q` is the product over `lits` of the share of
    /// the `k` models falsifying each, plus `ln(1 + s)`, where `s` sums over
    /// the pairs of `lits` the clauses that hold both variables.
    fn score(&mut self, lits: &[i32], spend: &mut Spend) -> f64 {
        let var = |l: i32| l.unsigned_abs() as usize - 1;
        let q: f64 = lits
            .iter()
            .map(|&l| {
                let f = self.false_share[var(l)];
                if l > 0 { f } else { 1.0 - f }
            })
            .product();
        let unlikely = -self.models * (1.0 - q).max(f64::MIN_POSITIVE).ln();
        let formula = self.formula;
        let shared = self
            .shared
            .get_or_insert_with(|| shared_clauses(formula, spend));
        let mut s = 0u64;
        for (k, &a) in lits.iter().enumerate() {
            let row = &shared[var(a)];
            for &b in &lits[k + 1..] {
                if let Ok(j) = row.binary_search_by_key(&(var(b) as u32), |&(w, _)| w) {
                    s += u64::from(row[j].1);
                }
            }
        }
        unlikely + (1.0 + s as f64).ln()
    }
}

/// Per variable of `formula`, each other variable it shares a clause of at
/// most [`SHARED_MAX_LEN`] literals with and in how many, by variable.
fn shared_clauses(formula: &CnfFormula, spend: &mut Spend) -> Vec<Vec<(u32, u32)>> {
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); formula.num_vars() as usize];
    let mut read = 0u64;
    for clause in formula.clauses() {
        let lits = &clause.literals;
        if lits.len() > SHARED_MAX_LEN {
            continue;
        }
        read += (lits.len() * lits.len()) as u64;
        for (k, a) in lits.iter().enumerate() {
            for b in &lits[k + 1..] {
                adj[a.var.idx()].push(b.var.idx() as u32);
                adj[b.var.idx()].push(a.var.idx() as u32);
            }
        }
    }
    spend.scan(read);
    adj.into_iter()
        .map(|mut row| {
            row.sort_unstable();
            let mut out: Vec<(u32, u32)> = Vec::new();
            for v in row {
                match out.last_mut() {
                    Some((w, c)) if *w == v => *c += 1,
                    _ => out.push((v, 1)),
                }
            }
            out
        })
        .collect()
}

/// A candidate while the caps choose: its literals, its home node, its place
/// in candidate order, and its score (NaN until a cap needs it).
#[derive(Clone, Copy)]
struct Ranked {
    lits: [i32; 3],
    node: u32,
    seq: u64,
    score: f64,
}

/// Cut `items` to the `keep` of highest score, ties to the earlier in
/// candidate order, in no particular order. Scores are computed only when
/// there are more than `keep`.
fn keep_best(items: &mut Vec<Ranked>, keep: usize, r: usize, rank: &mut Rank, spend: &mut Spend) {
    if items.len() <= keep {
        return;
    }
    for item in items.iter_mut().filter(|it| it.score.is_nan()) {
        item.score = rank.score(&item.lits[..r], spend);
    }
    items.select_nth_unstable_by(keep - 1, |a, b| {
        b.score.total_cmp(&a.score).then(a.seq.cmp(&b.seq))
    });
    items.truncate(keep);
}

/// One step's candidates of one length, gathered under the caps: those no
/// model of the batch falsifies and that are not clauses of the formula, at
/// most [`NODE_KEEP`] per node and [`COMPONENT_KEEP`] in all.
struct Gather {
    r: usize,
    emits: bool,
    /// The formula's clauses of length `r`, sorted.
    in_formula: HashSet<[i32; 3]>,
    /// Whether candidates in the formula are kept, as known (uncapped).
    keep_known: bool,
    full: u64,
    node: Vec<Ranked>,
    all: Vec<Ranked>,
    known: Vec<Ranked>,
    read: u64,
}

impl Gather {
    fn new(formula: &CnfFormula, r: usize, emits: bool, batch: &ModelBatch) -> Self {
        let in_formula = formula
            .clauses()
            .iter()
            .filter(|c| c.literals.len() == r)
            .map(|c| {
                let mut key = [0i32; 3];
                for (k, l) in c.literals.iter().enumerate() {
                    key[k] = l.to_dimacs();
                }
                key[..r].sort_unstable();
                key
            })
            .collect();
        Gather {
            r,
            emits,
            in_formula,
            keep_known: r == 2,
            full: if batch.count == 64 {
                u64::MAX
            } else {
                (1u64 << batch.count) - 1
            },
            node: Vec::new(),
            all: Vec::new(),
            known: Vec::new(),
            read: 0,
        }
    }

    /// Consider the candidate `lits` of node `node`, `seq`-th in candidate
    /// order.
    fn offer(&mut self, lits: &[i32], node: usize, seq: u64, batch: &ModelBatch) {
        let mut f = self.full;
        for &l in lits {
            let w = batch.words[l.unsigned_abs() as usize - 1];
            f &= if l > 0 { !w & self.full } else { w };
        }
        if f != 0 {
            return;
        }
        let mut item = Ranked {
            lits: [0; 3],
            node: node as u32,
            seq,
            score: f64::NAN,
        };
        item.lits[..self.r].copy_from_slice(lits);
        let mut key = item.lits;
        key[..self.r].sort_unstable();
        if self.in_formula.contains(&key) {
            if self.keep_known {
                self.known.push(item);
            }
            return;
        }
        self.node.push(item);
    }

    /// Charge what was read since the last charge.
    fn charge(&mut self, spend: &mut Spend) {
        spend.scan(std::mem::take(&mut self.read));
    }

    /// Cut the current node's candidates to its cap, when they are twice it.
    fn trim(&mut self, rank: &mut Rank, spend: &mut Spend) {
        if self.node.len() >= 2 * NODE_KEEP {
            keep_best(&mut self.node, NODE_KEEP, self.r, rank, spend);
        }
    }

    /// The current node is done: its best join the component's.
    fn end_node(&mut self, rank: &mut Rank, spend: &mut Spend) {
        self.charge(spend);
        keep_best(&mut self.node, NODE_KEEP, self.r, rank, spend);
        self.all.append(&mut self.node);
        if self.all.len() >= 2 * COMPONENT_KEEP {
            keep_best(&mut self.all, COMPONENT_KEEP, self.r, rank, spend);
        }
    }

    /// The kept candidates as cells, in candidate order.
    fn finish(mut self, layout: &Layout, rank: &mut Rank, spend: &mut Spend) -> Cells {
        self.end_node(rank, spend);
        keep_best(&mut self.all, COMPONENT_KEEP, self.r, rank, spend);
        let mut items: Vec<(Ranked, Cell)> = self
            .all
            .into_iter()
            .map(|it| (it, Cell::Open))
            .chain(self.known.into_iter().map(|it| (it, Cell::InCnf)))
            .collect();
        items.sort_by_key(|(it, _)| it.seq);
        let mut cells = Cells::empty(self.r, self.emits);
        for (it, status) in items {
            cells.push(&it.lits[..self.r], layout, it.node as usize);
            *cells.status.last_mut().expect("just pushed") = status;
        }
        cells
    }
}

/// The pair step's candidates: per node, the four sign patterns of open-left
/// times open-right, left-major, as [`Gather`] keeps them. Each pair of
/// variables read is charged one unit.
fn pair_cells(tree: &Tree<'_>, batch: &ModelBatch, rank: &mut Rank, spend: &mut Spend) -> Cells {
    const SIGNS: [(i32, i32); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];
    let mut gather = Gather::new(tree.formula, 2, true, batch);
    let until = spend.gather_until();
    let mut seq = 0u64;
    'nodes: for node in 0..tree.layout.nodes.len() {
        let (l, r) = (tree.open.left[node] as u64, tree.open.right[node] as u64);
        if l * r == 0 {
            continue;
        }
        spend.scan(tree.layout.nodes[node].leaves() as u64);
        let (left, right) = open_children(tree, node);
        for (ia, &a) in left.iter().enumerate() {
            gather.read += r;
            gather.charge(spend);
            if spend.out() || spend.used() >= until {
                break 'nodes;
            }
            for (ib, &b) in right.iter().enumerate() {
                for (k, &(sx, sy)) in SIGNS.iter().enumerate() {
                    let at = seq + k as u64 * l * r + ia as u64 * r + ib as u64;
                    gather.offer(&[sx * (a as i32 + 1), sy * (b as i32 + 1)], node, at, batch);
                }
            }
            gather.trim(rank, spend);
        }
        gather.end_node(rank, spend);
        seq += 4 * l * r;
    }
    gather.finish(tree.layout, rank, spend)
}

/// Every `r`-set of a node's variables in `nodes` not taken by a smaller node,
/// with each of its sign patterns, as [`Gather`] keeps them. `seen` holds the
/// sets already taken. Each set read is charged one unit.
fn set_cells(
    tree: &Tree<'_>,
    nodes: &[(usize, Vec<u32>)],
    r: usize,
    seen: &mut HashSet<[u32; 3]>,
    batch: &ModelBatch,
    rank: &mut Rank,
    spend: &mut Spend,
) -> Cells {
    let mut gather = Gather::new(tree.formula, r, r == 3, batch);
    let mut lits = vec![0i32; r];
    let until = spend.gather_until();
    let mut seq = 0u64;
    for (node, open) in nodes {
        gather.charge(spend);
        if spend.out() || spend.used() >= until {
            break;
        }
        for_each_subset(open.len(), r, |ix| {
            let mut key = [u32::MAX; 3];
            for (k, &j) in ix.iter().enumerate() {
                key[k] = open[j];
            }
            if !seen.insert(key) {
                return;
            }
            gather.read += 1;
            for signs in 0..1u32 << r {
                for (k, &j) in ix.iter().enumerate() {
                    let v = open[j] as i32 + 1;
                    lits[k] = if (signs >> (r - 1 - k)) & 1 == 0 {
                        v
                    } else {
                        -v
                    };
                }
                gather.offer(&lits, *node, seq, batch);
                seq += 1;
            }
            gather.trim(rank, spend);
        });
        gather.end_node(rank, spend);
    }
    gather.finish(tree.layout, rank, spend)
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
/// binary clause the pair cells know to be implied.
fn drop_not_prime(pairs: &Cells, triples: &mut Cells, which: impl Fn(Cell) -> bool) {
    let implied: HashSet<(i32, i32)> = pairs
        .with(Cell::implied)
        .map(|i| {
            let l = pairs.lits(i);
            (l[0], l[1])
        })
        .collect();
    if implied.is_empty() {
        return;
    }
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
        }
    }
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
    /// When the last clause that could be emitted was proven, in the clock's
    /// currency since `mark`; the drought counts from here. `Duration::MAX`
    /// while no drought is counted.
    last_proof: Duration,
    /// While the drought is counted under the wall clock, the instant it
    /// ends the proof steps, shared with each [`Stop`].
    drought_at: Rc<Shared<Option<Instant>>>,
    /// When the drought ended the proof steps.
    dried: Option<Duration>,
    /// How long the proof steps go on without a proof.
    drought: Duration,
}

impl Spend {
    fn new(meter: PreprocessMeter, mark: PhaseMark, limit: Duration, drought: Duration) -> Self {
        Spend {
            meter,
            mark,
            limit,
            drought,
            last_proof: Duration::ZERO,
            drought_at: Rc::new(Shared::new(None)),
            dried: None,
        }
    }

    fn used(&self) -> Duration {
        self.meter.elapsed(self.mark)
    }

    /// Whether the current stage is over: its limit reached, or, in the proof
    /// steps, the drought.
    fn out(&mut self) -> bool {
        let used = self.used();
        if self.armed() && self.dried.is_none() && used.saturating_sub(self.last_proof) >= self.drought {
            self.dried = Some(used);
        }
        used >= self.limit || (self.armed() && self.dried.is_some())
    }

    /// Whether the current stage's limit is reached, the drought aside.
    fn past_limit(&self) -> bool {
        self.used() >= self.limit
    }

    fn armed(&self) -> bool {
        self.last_proof != Duration::MAX
    }

    /// Count the drought, from the last proof.
    fn arm(&mut self) {
        if !self.meter.deterministic() {
            let silent = self.used().saturating_sub(self.last_proof);
            self.drought_at
                .set(Some(Instant::now() + self.drought.saturating_sub(silent)));
        }
    }

    /// Stop counting the drought.
    fn disarm(&mut self) {
        self.last_proof = Duration::MAX;
        self.drought_at.set(None);
    }

    /// Count the drought from now: the proofs start.
    fn restart_drought(&mut self) {
        self.dried = None;
        self.last_proof = self.used();
        self.arm();
    }

    /// Where gathering candidates ends: [`GATHER_SHARE`] of what is left of
    /// the current stage.
    fn gather_until(&self) -> Duration {
        let used = self.used();
        used + self.limit.saturating_sub(used).mul_f64(GATHER_SHARE)
    }

    fn proved(&mut self) {
        if self.armed() {
            self.last_proof = self.used();
            self.arm();
        }
    }

    /// The wall instant at which the current stage ends; under the
    /// deterministic clock, where charged work stops the pass and each call is
    /// bounded by its conflict cap, only the run's backstop.
    fn stop(&self) -> Stop {
        Stop {
            at: self.meter.wall_stop(
                (!self.meter.deterministic())
                    .then(|| Instant::now() + self.limit.saturating_sub(self.used())),
            ),
            drought_at: Rc::clone(&self.drought_at),
        }
    }

    /// Run one solver call, charging what its search did.
    fn call(&mut self, solver: &mut CaDiCal, f: impl FnOnce(&mut CaDiCal) -> Status) -> Status {
        self.meter.call(PreprocessPhase::Implicates, solver, f)
    }

    fn scan(&mut self, literals: u64) {
        self.meter.charge(literals);
    }
}

// ── Proving ─────────────────────────────────────────────────────────────────

fn add_clause(solver: &mut CaDiCal, lits: &[i32]) {
    for &l in lits {
        solver.add(l);
    }
    solver.add(0);
}

fn add_formula(solver: &mut CaDiCal, formula: &CnfFormula) {
    for clause in formula.clauses() {
        for lit in &clause.literals {
            solver.add(lit.to_dimacs());
        }
        solver.add(0);
    }
}

/// The seed model and up to `MODELS - 1` more with random phases; none when
/// the formula is unsatisfiable or no model is found.
fn draw_models(solver: &mut CaDiCal, n: usize, spend: &mut Spend) -> Option<ModelBatch> {
    let mut bounded = Bounded::new(solver, spend.stop());
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
        match spend.call(&mut bounded, CaDiCal::solve) {
            Status::Satisfiable => batch.push(&read_model(&mut bounded, n)),
            Status::Unsatisfiable => break,
            Status::Unknown => {}
        }
    }
    for v in 1..=n as i32 {
        bounded.unphase(v);
    }
    (batch.count > 0).then_some(batch)
}

/// Mark the open candidates that hold a literal the formula implies on its
/// own.
fn screen(solver: &mut CaDiCal, cells: &mut [&mut Cells], spend: &mut Spend) {
    let units = find_units(solver, cells, spend);
    for c in cells.iter_mut() {
        c.drop_units(&units);
    }
}

/// Literals the formula implies on its own among those of the open cells,
/// within the stage's limit whatever the drought. Returns them sorted.
fn find_units(solver: &mut CaDiCal, cells: &[&mut Cells], spend: &mut Spend) -> Vec<i32> {
    let mut lits: Vec<i32> = cells
        .iter()
        .flat_map(|c| {
            c.with(|s| s == Cell::Open)
                .flat_map(move |i| c.lits(i).to_vec())
        })
        .collect();
    lits.sort_unstable();
    lits.dedup();
    let mut units = Vec::new();
    for l in lits {
        if spend.past_limit() {
            break;
        }
        solver.assume(-l);
        if spend.call(solver, CaDiCal::propagate) == Status::Unsatisfiable {
            units.push(l);
        }
    }
    units
}

/// The pair step: the screen, the propagation tries, then the proof rounds,
/// the drought counted from their start. Returns whether the allowance ran
/// out.
fn prove_pairs(
    formula: &CnfFormula,
    solver: &mut CaDiCal,
    cells: &mut Cells,
    rank: &mut Rank,
    spend: &mut Spend,
) -> bool {
    let n = formula.num_vars() as usize;
    screen(solver, &mut [&mut *cells], spend);
    propagation_tries(formula, solver, cells, rank, spend);
    spend.restart_drought();
    let order = cells.order();
    let finished = PAIR_CAPS
        .iter()
        .all(|&cap| proof_round(solver, cells, &order, cap, n, spend, &mut |_, _| {}));
    spend.out() || (!finished && cells.count(Cell::Deferred) > 0)
}

/// The triple step's proofs: per round, the binary candidates, then the
/// ternary ones that hold no proven binary clause. Returns whether the
/// allowance ran out.
fn prove_triples(
    formula: &CnfFormula,
    solver: &mut CaDiCal,
    pairs: &mut Cells,
    triples: &mut Cells,
    spend: &mut Spend,
) -> bool {
    let n = formula.num_vars() as usize;
    screen(solver, &mut [&mut *pairs, &mut *triples], spend);
    let porder = pairs.order();
    let torder = triples.order();
    // Models the binary calls return reach the ternary cells 64 at a time,
    // before each ternary round.
    let mut queued: Vec<Vec<i32>> = Vec::new();
    let mut finished = true;
    for &cap in &TRIPLE_CAPS {
        let pairs_done = proof_round(solver, pairs, &porder, cap, n, spend, &mut |model, _| {
            queued.push(model.to_vec())
        });
        drop_not_prime(pairs, triples, Cell::open);
        for chunk in queued.chunks(64) {
            let mut b = ModelBatch::new(n);
            for m in chunk {
                b.push(m);
            }
            let read = triples.refute(&b);
            spend.scan(read);
        }
        queued.clear();
        finished = pairs_done
            && proof_round(
                solver,
                triples,
                &torder,
                cap,
                n,
                spend,
                &mut |model, spend| {
                    let mut one = ModelBatch::new(n);
                    one.push(model);
                    let read = pairs.refute(&one);
                    spend.scan(read);
                },
            );
        if !finished {
            break;
        }
    }
    drop_not_prime(pairs, triples, Cell::proven);
    spend.out() || (!finished && pairs.count(Cell::Deferred) + triples.count(Cell::Deferred) > 0)
}

/// One round at conflict cap `cap` over the open cells of `order`, then, when
/// a search proved some, one pass of propagation alone over them, which the
/// clauses proven since can now settle. Every model an answer gives refutes
/// the open cells and is passed to `also`. Returns whether the round finished
/// inside the allowance.
fn proof_round(
    solver: &mut CaDiCal,
    cells: &mut Cells,
    order: &[usize],
    cap: i32,
    n: usize,
    spend: &mut Spend,
    also: &mut OnModel<'_>,
) -> bool {
    let before = cells.count(Cell::BySearch);
    search_round(solver, cells, order, cap, n, spend, also)
        && (cells.count(Cell::BySearch) == before || propagation_pass(solver, cells, order, spend))
}

/// Propagation alone under the negation of [`PROPAGATION_TRIES`] open cells,
/// within the stage's limit whatever the drought: when more are open, the
/// half of highest score, best first, then the other half spread evenly over
/// the rest in score order (ties to the earlier in candidate order); else all,
/// in candidate order. The tries run on a solver of their own that holds the
/// formula alone, so the solver of the search rounds is changed only by the
/// clauses they prove, which it is given.
fn propagation_tries(
    formula: &CnfFormula,
    solver: &mut CaDiCal,
    cells: &mut Cells,
    rank: &mut Rank,
    spend: &mut Spend,
) {
    let mut tries: Vec<(f64, usize)> = cells.with(|s| s == Cell::Open).map(|i| (0.0, i)).collect();
    if tries.len() > PROPAGATION_TRIES {
        for (score, i) in &mut tries {
            *score = rank.score(cells.lits(*i), spend);
        }
        tries.sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let best = PROPAGATION_TRIES / 2;
        let spread = PROPAGATION_TRIES - best;
        let rest = tries.len() - best;
        tries = (0..best)
            .map(|k| tries[k])
            .chain((0..spread).map(|k| tries[best + k * rest / spread]))
            .collect();
    }
    if tries.is_empty() {
        return;
    }
    let Some(mut own) = CaDiCal::new() else {
        return;
    };
    add_formula(&mut own, formula);
    spend.scan(
        formula
            .clauses()
            .iter()
            .map(|c| c.literals.len() as u64)
            .sum(),
    );
    let n = formula.num_vars() as usize;
    for (_, i) in tries {
        if spend.past_limit() {
            break;
        }
        if !cells.status[i].open() {
            continue;
        }
        match propagate_negation(&mut own, cells, i, spend) {
            Status::Unsatisfiable => {
                mark_proven(&mut own, cells, i, Cell::ByPropagation, spend);
                add_clause(solver, cells.lits(i));
            }
            Status::Satisfiable => refute_by_answer(&mut own, cells, n, spend, &mut |_, _| {}),
            Status::Unknown => {}
        }
    }
}

fn mark_proven(solver: &mut CaDiCal, cells: &mut Cells, i: usize, how: Cell, spend: &mut Spend) {
    cells.status[i] = how;
    if cells.emits {
        spend.proved();
    }
    add_clause(solver, cells.lits(i));
}

/// What propagation under the negation of cell `i` gives.
fn propagate_negation(solver: &mut CaDiCal, cells: &Cells, i: usize, spend: &mut Spend) -> Status {
    for &l in cells.lits(i) {
        solver.assume(-l);
    }
    spend.call(solver, CaDiCal::propagate)
}

/// Each open cell of `order` in turn: propagation, then a search capped at
/// `cap`. Returns whether it finished inside the allowance.
fn search_round(
    solver: &mut CaDiCal,
    cells: &mut Cells,
    order: &[usize],
    cap: i32,
    n: usize,
    spend: &mut Spend,
    also: &mut OnModel<'_>,
) -> bool {
    for &i in order {
        if !cells.status[i].open() {
            continue;
        }
        if spend.out() {
            return false;
        }
        match propagate_negation(solver, cells, i, spend) {
            Status::Unsatisfiable => {
                mark_proven(solver, cells, i, Cell::ByPropagation, spend);
                continue;
            }
            Status::Satisfiable => {
                refute_by_answer(solver, cells, n, spend, also);
                continue;
            }
            Status::Unknown => {}
        }
        for &l in cells.lits(i) {
            solver.assume(-l);
        }
        solver.limit(c"conflicts", cap);
        match spend.call(solver, CaDiCal::solve) {
            Status::Unsatisfiable => mark_proven(solver, cells, i, Cell::BySearch, spend),
            Status::Satisfiable => refute_by_answer(solver, cells, n, spend, also),
            Status::Unknown if spend.out() => return false,
            Status::Unknown => cells.status[i] = Cell::Deferred,
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
    let read = cells.refute(&one);
    spend.scan(read);
    also(&model, spend);
}

/// One pass of propagation alone over the open cells of `order`. Returns
/// whether it finished inside the allowance.
fn propagation_pass(
    solver: &mut CaDiCal,
    cells: &mut Cells,
    order: &[usize],
    spend: &mut Spend,
) -> bool {
    for &i in order {
        if !cells.status[i].open() {
            continue;
        }
        if spend.out() {
            return false;
        }
        if propagate_negation(solver, cells, i, spend) == Status::Unsatisfiable {
            mark_proven(solver, cells, i, Cell::ByPropagation, spend);
        }
    }
    true
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
) -> Vec<usize> {
    let proven: Vec<usize> = triples.with(Cell::proven).collect();
    let mut emitted = Vec::new();
    if proven.is_empty() {
        return emitted;
    }
    let Some(mut solver) = CaDiCal::new() else {
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
    let mut solver = Bounded::new(&mut solver, spend.stop());
    for i in proven {
        if spend.out() {
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
        if spend.call(&mut solver, CaDiCal::propagate) == Status::Unsatisfiable {
            continue;
        }
        assume(&mut solver);
        solver.limit(c"conflicts", cap);
        match spend.call(&mut solver, CaDiCal::solve) {
            Status::Satisfiable => emitted.push(i),
            Status::Unknown if spend.out() => break,
            Status::Unsatisfiable | Status::Unknown => {}
        }
    }
    emitted
}

// ── Minimizing ──────────────────────────────────────────────────────────────

/// The clauses of `cands`, taken in the given order, that unit propagation
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
) -> Vec<Vec<i32>> {
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
    let mut kept = Vec::new();
    let mut negated = Vec::new();
    for (&(lits, lo, hi), id) in cands.iter().zip(ids) {
        units.set_active(id, false);
        negated.clear();
        negated.extend(lits.iter().map(|l| -l));
        let (refuted, read) = units.refutes(&negated, lo, hi);
        spend.scan(read);
        if !refuted {
            units.set_active(id, true);
            kept.push(lits.to_vec());
        }
    }
    kept
}

/// The kept proven pair cells' clauses: largest nodes first, a node's cells
/// in candidate order.
fn minimize_pairs(
    formula: &CnfFormula,
    layout: &Layout,
    cells: &Cells,
    spend: &mut Spend,
) -> Vec<Vec<i32>> {
    let mut proven: Vec<usize> = cells.with(Cell::proven).collect();
    proven.sort_by_key(|&i| (std::cmp::Reverse(cells.span[i].leaves()), i));
    let cands: Vec<(&[i32], u32, u32)> = proven
        .iter()
        .map(|&i| (cells.lits(i), cells.span[i].lo, cells.span[i].hi))
        .collect();
    minimize(formula, layout, &[], &cands, spend)
}

/// The kept emitted triple cells' clauses, over the formula and the kept
/// binary clauses: largest lowest common ancestor first, then by literals.
fn minimize_triples(
    formula: &CnfFormula,
    vtree: &Vtree,
    layout: &Layout,
    kept_pairs: &[Vec<i32>],
    cells: &Cells,
    emitted: &[usize],
    spend: &mut Spend,
) -> Vec<Vec<i32>> {
    let mut cands: Vec<(&[i32], u32, u32)> = emitted
        .iter()
        .map(|&i| {
            let (_, lo, hi) = layout.lca(vtree, cells.lits(i));
            (cells.lits(i), lo, hi)
        })
        .collect();
    cands.sort_by(|a, b| (b.2 - b.1).cmp(&(a.2 - a.1)).then_with(|| a.0.cmp(b.0)));
    minimize(formula, layout, kept_pairs, &cands, spend)
}
