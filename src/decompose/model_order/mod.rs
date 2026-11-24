//! A vtree learned from models of the formula alone.
//!
//! Two draws of models (one to build from, one held out to score with) give
//! weighted variable sets ([`weights`]) from two sources: a blend of mutual
//! information on the building draw and clause co-occurrence, which needs no
//! solver call, and minimal repairs of some of the building draw's models.
//! Each source gives four candidate leaf orders ([`orders`]). Each order is cut
//! into blocks, every run of consecutive blocks is read on the building draw,
//! and the join DP brackets the blocks from those readings ([`bracket`]). The
//! blend's candidates are built first. Where the build's share is at least
//! [`REPAIR_SHARE`], repairs then run while the allowance leaves room for
//! their candidates, and theirs are built from the repairs found. The
//! held-out join score picks among all the trees: the blend's all-sets
//! Fiedler tree stays unless another is lower by more than
//! [`MARGIN`] bits. The building draw is doubled and the trees built again
//! while a node of the picked tree reads at the draw's cap. Each draw's
//! readings are kept by variable set ([`Readings`]), so a set read for one
//! order, score or figure is not read again for another.
//!
//! [`pick`] then sets the learned tree against the portfolio's on two figures
//! ([`figures`]): it takes the learned tree when its `s` is lower by at least
//! [`S_MARGIN`] and its Omega is less than [`OMEGA_MARGIN`] above the
//! portfolio tree's.
//!
//! The build and the figures are bounded by an allowance on the preprocessing
//! clock, solver work and readings alike, and under charged work also stop at
//! a wall time ([`models::wall_stop`]). When it runs out before a tree is
//! built and scored there is no learned tree, and the building draw ends as
//! soon as its calls so far project the two draws past the allowance
//! ([`models::draw`]). The draws are smaller where the portfolio tree leaves
//! little to gain for the build's cost ([`allowance`]).
//! The build stops before the held-out draw when the blend's first tree is far
//! behind the portfolio tree on the building draw ([`EARLY_STOP`]), and after
//! the blend's candidates when the blend tree's `s` is more than [`GAP_STOP`]
//! above the portfolio tree's, or, without repairs, when it is not
//! [`S_MARGIN`] below. A blend tree whose Omega already fails
//! [`OMEGA_MARGIN`] goes on as without repairs ([`after_gap`]).

use std::fmt;
use std::time::{Duration, Instant};

use rand::rngs::SmallRng;
use rand::{RngExt, SeedableRng};

use super::reading::{ClauseList, ModelSample, Readings, interval_matrix};
use crate::cnf::CnfFormula;
use crate::config::PreprocessClock;
use crate::preprocess::implicates::Allowance;
use crate::preprocess::models::{self, ModelMatrix, Repair};
use crate::vtree::Vtree;
use bracket::{Tree, join_split};
use orders::{CANDIDATES, Candidate};
use weights::Sets;

mod bracket;
mod figures;
mod orders;
mod spectral;
#[cfg(test)]
mod tests;
mod weights;

/// The most models a draw may have, and the least of that most.
const DRAW: usize = 1000;
const MIN_DRAW: usize = DRAW / 4;
/// The most is [`DRAW`] when what the portfolio tree leaves to gain is this
/// many times the build's expected cost or more, and shrinks in proportion
/// below it, to [`MIN_DRAW`] ([`allowance`]).
const FULL_DRAW_RATIO: f64 = 64.0;
/// Models of the building draw before any doubling, and of the held-out
/// draw; each at most the most a draw may have.
const BUILD_DRAW: usize = 250;
const HELD_DRAW: usize = DRAW;
/// Models of the building draw that get repairs, and variables repaired in
/// each.
const REPAIR_MODELS: usize = 50;
const REPAIR_VARS: usize = 200;
/// The conflict cap of each repair call.
const REPAIR_CONFLICTS: i32 = 20_000;
const BUILD_SEED: u64 = 1;
const HELD_SEED: u64 = 2;
const REPAIR_SEED: u64 = 57;
/// Repairs stop where what is left of the allowance falls to this many times
/// what the blend's candidates cost, so the repairs' candidates can still be
/// built and scored.
const REPAIR_RESERVE: f64 = 2.0;
/// Another order replaces the base order only when its held-out join score
/// is lower by more than this many bits.
const MARGIN: f64 = 0.1;
/// Charged work units per millisecond of reading, under a deterministic
/// clock: the median rate measured on the build's readings. A reading is
/// charged its clause occurrences plus one unit per clause and word of models.
const READ_UNITS_PER_MS: u64 = 61_000;
/// What the portfolio tree leaves to gain, in ms, is `PAYOFF_MS *
/// 2^(PAYOFF_PER_BIT * s_bound)` ([`allowance`]). Fitted, with the build's
/// figures below, on the model counts of half of a benchmark tier at a
/// 120 s limit: least time spent on builds whose tree is not taken while
/// three quarters of the trees taken are still built.
const PAYOFF_MS: f64 = 178.0;
const PAYOFF_PER_BIT: f64 = 0.2;
/// The learned tree's expected build, in ms, is `BUILD_MS * n^BUILD_PER_VAR *
/// m^BUILD_PER_CLAUSE` over `n` variables and `m` clauses ([`allowance`]).
const BUILD_MS: f64 = 0.536;
const BUILD_PER_VAR: f64 = 0.72;
const BUILD_PER_CLAUSE: f64 = 0.739;
/// The build starts only when this many times its expected cost fits.
const BUILD_SLACK: f64 = 2.0;
/// The build starts only when the portfolio tree's `s_bound` is at least
/// this: below it the learned tree was not taken on a benchmark tier at a
/// 120 s limit, nor on the formulas it was checked on.
const MIN_S_BOUND: f64 = 20.0;
/// The pick considers formulas of more than this many variables.
pub(crate) const MIN_VARS: u32 = 64;
/// The learned tree is passed over when its Omega is this much or more above
/// the portfolio tree's.
const OMEGA_MARGIN: usize = 5;
/// The learned tree is taken only when its `s` is at least this many bits
/// below the portfolio tree's.
const S_MARGIN: f64 = 0.5;
/// After the blend's candidates, the build stops when the blend tree's `s` is
/// more than this many bits above the portfolio tree's. The repairs would
/// then have to lower `s` by more than this plus [`S_MARGIN`] for a take,
/// which they rarely do.
const GAP_STOP: f64 = 0.5;
/// Before the held-out draw, the build stops when the blend's first tree has
/// an `s` on the building draw more than this many bits above the portfolio
/// tree's on the same draw. Read on the models it was built from, the learned
/// tree's `s` is low, so a tree this far behind there stays behind.
const EARLY_STOP: f64 = 1.5;
/// The repairs run only where the build's share of the run's budget is at
/// least this. Below it the blend tree alone is set against the portfolio
/// tree's, so the build stops after the blend's candidates unless the blend
/// tree's `s` is at least [`S_MARGIN`] below.
const REPAIR_SHARE: Duration = Duration::from_secs(60);

/// What the build may spend, and has spent.
struct Spend {
    allowance: Allowance,
    started: Instant,
    limit: Duration,
    /// Where the build stops in wall time ([`models::wall_stop`]).
    wall: Option<Instant>,
    /// Solver work charged by the draws and repairs.
    solver: Duration,
    units: u64,
    stages: Stages,
}

/// Where a build's time and work went, stage by stage.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Stages {
    /// Wall time of the building draw, then the held-out draw.
    pub draws: [Duration; 2],
    pub repairs: Duration,
    /// The candidate orders from the repair sets.
    pub orders: Duration,
    /// Every candidate's block runs read on the building draw.
    pub readings: Duration,
    /// The join DPs.
    pub brackets: Duration,
    /// The candidates' held-out join scores.
    pub held: Duration,
    /// `s` of the learned tree, then of the portfolio's.
    pub figures: [Duration; 2],
    /// Solver work of the two draws and the repairs, in the clock's currency.
    pub solver: [Duration; 3],
    /// Reads of the readings, the held-out scores and the figures.
    pub reads: [u64; 3],
    /// Repair pairs handled, and the minimal repairs among them.
    pub pairs: usize,
    pub minimal: usize,
    /// Models asked of the last building draw.
    pub models: usize,
    /// Solver calls of the repairs.
    pub calls: u64,
    /// The weights' cheap signals (mutual information, clause sets, the
    /// choice of pairs to repair).
    pub cheap: Duration,
    /// The blend's first tree's `s` less the portfolio tree's, both on the
    /// building draw ([`EARLY_STOP`]), and what the build had spent then.
    pub early: Option<(f64, Duration)>,
    /// Calls of the draws stopped by the conflict cap.
    pub capped: usize,
    /// A draw ended where its projected cost passed the allowance.
    pub projected: bool,
    /// The blend tree's `s` less the portfolio tree's.
    pub gap: Option<f64>,
    /// The blend tree's Omega and the portfolio tree's, at the gap.
    pub omega: Option<(usize, usize)>,
    /// The gap stopped the build.
    pub stopped: bool,
    /// Where the picked tree's weighted sets came from.
    pub source: Option<Source>,
    /// The last pick failed the take test and the blend's pick at the gap,
    /// which passed it, was taken.
    pub from_gap: bool,
}

impl fmt::Display for Stages {
    /// `ms` then nine wall times in milliseconds (the two draws, repairs,
    /// orders, readings, DPs, held-out scores, the two figures), `solver`
    /// then three solver charges in milliseconds, `reads` then three read
    /// counts, `repairs` then minimal repairs over pairs, `draw` then the
    /// models asked of the last building draw, `gap` then the gap when there
    /// is a portfolio tree, `omega` then the two Omegas at the gap, `early`
    /// then the early gap and the spend at it, `capped` then the draws' calls stopped by the conflict cap,
    /// `projected` when a draw's projected cost ended it, and `taken at the
    /// gap` when the blend's pick at the gap was taken.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let walls = [
            self.draws[0],
            self.draws[1],
            self.repairs,
            self.orders,
            self.readings,
            self.brackets,
            self.held,
            self.figures[0],
            self.figures[1],
        ];
        write!(f, "ms")?;
        for d in walls {
            write!(f, " {:.0}", ms(d))?;
        }
        write!(f, " solver")?;
        for d in self.solver {
            write!(f, " {:.0}", ms(d))?;
        }
        write!(
            f,
            " reads {} {} {}",
            self.reads[0], self.reads[1], self.reads[2]
        )?;
        write!(f, " repairs {}/{}", self.minimal, self.pairs)?;
        write!(f, " draw {}", self.models)?;
        if let Some(gap) = self.gap {
            write!(f, " gap {gap:.2}")?;
        }
        if let Some((mine, theirs)) = self.omega {
            write!(f, " omega {mine}/{theirs}")?;
        }
        if let Some((gap, spent)) = self.early {
            write!(f, " early {gap:.2} at {:.0}", ms(spent))?;
        }
        if self.capped > 0 {
            write!(f, " capped {}", self.capped)?;
        }
        if self.projected {
            write!(f, " projected")?;
        }
        if self.from_gap {
            write!(f, " taken at the gap")?;
        }
        match self.source {
            None => Ok(()),
            Some(Source::Blend) => write!(f, " from blend"),
            Some(Source::Repairs) => write!(f, " from repairs"),
        }
    }
}

impl Spend {
    fn new(allowance: &Allowance) -> Self {
        let started = Instant::now();
        Spend {
            allowance: *allowance,
            started,
            limit: models::limit(allowance),
            wall: models::wall_stop(allowance, started),
            solver: Duration::ZERO,
            units: 0,
            stages: Stages::default(),
        }
    }

    fn used(&self) -> Duration {
        match self.allowance.clock {
            PreprocessClock::WallClock => self.started.elapsed(),
            PreprocessClock::Deterministic { .. } => {
                self.solver + Duration::from_millis(self.units / READ_UNITS_PER_MS)
            }
        }
    }

    /// Whether the allowance is spent, the build's wall stop has passed, or
    /// the wall backstop of a metered construction has.
    fn out(&self) -> bool {
        self.used() >= self.limit
            || self.wall.is_some_and(|w| Instant::now() >= w)
            || crate::decompose::meter::past_backstop()
    }

    /// What is left above `reserve`, for a solver phase, which stops at the
    /// build's wall stop at the latest.
    fn rest(&self, reserve: Duration) -> Allowance {
        Allowance {
            nominal: self
                .limit
                .saturating_sub(reserve)
                .saturating_sub(self.used()),
            deadline: crate::budget::earliest(self.wall, self.allowance.deadline),
            ..self.allowance
        }
    }

    fn charge_reads(&mut self, reads: u64, clauses: &ClauseList, sample: &ModelSample) {
        let per = (clauses.occurrences() + clauses.len() * sample.words()) as u64;
        self.units = self.units.saturating_add(reads * per);
    }
}

/// One draw of `count` models, or none when the formula has no model or the
/// allowance runs out, or would with `then` more models drawn after it;
/// `which` is its place in [`Stages::draws`].
fn draw(
    formula: &CnfFormula,
    seed: u64,
    count: usize,
    then: usize,
    spend: &mut Spend,
    which: usize,
) -> Option<ModelMatrix> {
    let t = Instant::now();
    let d = models::draw(formula, count, then, seed, &spend.rest(Duration::ZERO));
    spend.solver += d.spent;
    spend.stages.draws[which] += t.elapsed();
    spend.stages.solver[which] += d.spent;
    spend.stages.capped += d.capped;
    spend.stages.projected |= d.projected;
    (!(d.unsatisfiable || d.ran_out || d.models.is_empty())).then_some(d.models)
}

/// The repairs of `pairs` in `session` while what is left of the allowance
/// stays above `reserve`, and whether they stopped there first.
fn repair(
    session: &mut models::RepairSession<'_>,
    pairs: &[(usize, u32)],
    spend: &mut Spend,
    reserve: Duration,
) -> (Vec<Repair>, bool) {
    let t = Instant::now();
    let got = session.run(pairs, &spend.rest(reserve));
    spend.solver += got.spent;
    spend.stages.repairs += t.elapsed();
    spend.stages.solver[2] += got.spent;
    spend.stages.pairs += got.found.len();
    spend.stages.calls += got.calls;
    (got.found, got.ran_out)
}

/// The minimal repairs among `found`, those of `pairs`, each as the flipped
/// variable and then the others, with scale 1.
fn repair_sets(pairs: &[(usize, u32)], found: &[Repair]) -> Sets {
    pairs
        .iter()
        .zip(found)
        .filter_map(|(&(_, u), r)| match r {
            Repair::Minimal(d) => {
                Some((std::iter::once(u).chain(d.iter().copied()).collect(), 1.0))
            }
            _ => None,
        })
        .collect()
}

/// The blend's weighted sets ([`weights::blend`]); none when the allowance
/// runs out. Mutual information reads every pair of variables over the draw,
/// so it is charged one read unit per pair and word of models before it runs.
fn blend_sets(formula: &CnfFormula, train: &ModelMatrix, spend: &mut Spend) -> Option<Sets> {
    let n = formula.num_vars() as usize;
    let pairs = (n * n.saturating_sub(1) / 2) as u64;
    spend.units = spend
        .units
        .saturating_add(pairs.saturating_mul(train.len().div_ceil(64) as u64));
    if spend.out() {
        return None;
    }
    let t = Instant::now();
    let mi = weights::mutual(train);
    let mutual = weights::mutual_sets(&mi, n);
    let clauses = weights::clause_sets(&dimacs(formula));
    spend.stages.cheap += t.elapsed();
    Some(weights::blend(n, mutual, clauses))
}

/// The `(model, variable)` pairs to repair: up to [`REPAIR_MODELS`] distinct
/// models of the draw in a seeded random order, and in each up to
/// [`REPAIR_VARS`] variables drawn without replacement.
fn repair_pairs(models: &ModelMatrix) -> Vec<(usize, u32)> {
    let n = models.vars();
    let mut seen = rustc_hash::FxHashSet::default();
    let mut rows: Vec<usize> = (0..models.len())
        .filter(|&r| seen.insert(models.row(r)))
        .collect();
    let mut rng = SmallRng::seed_from_u64(REPAIR_SEED);
    let picked = REPAIR_MODELS.min(rows.len());
    partial_shuffle(&mut rows, picked, &mut rng);
    let mut vars: Vec<u32> = (0..n as u32).collect();
    let mut pairs = Vec::new();
    for &row in &rows[..picked] {
        let k = REPAIR_VARS.min(n);
        partial_shuffle(&mut vars, k, &mut rng);
        pairs.extend(vars[..k].iter().map(|&u| (row, u)));
    }
    pairs
}

/// Put a uniform random choice of `k` of the items, in random order, first.
fn partial_shuffle<T>(items: &mut [T], k: usize, rng: &mut SmallRng) {
    for i in 0..k {
        let j = rng.random_range(i..items.len());
        items.swap(i, j);
    }
}

/// Where a candidate order's weighted sets came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Blend,
    Repairs,
}

/// One candidate leaf order.
#[derive(Clone, Debug)]
struct Order {
    source: Source,
    candidate: Candidate,
    order: Vec<usize>,
}

/// One candidate's tree and held-out score.
struct Built {
    pub source: Source,
    pub tree: Tree,
    pub score: f64,
    /// A node of the tree other than the root reads at the building draw's
    /// cap.
    pub at_cap: bool,
}

/// Every candidate built, and which was picked.
struct Choice {
    pub built: Vec<Built>,
    pub picked: usize,
}

/// The trees of `orders` from the building draw's readings `train`, scored on
/// the held-out `held`, and the pick among them; none when the allowance runs
/// out. The first order is the base: another replaces it only when its score
/// is lower by more than [`MARGIN`].
fn choose(
    clauses: &ClauseList,
    train: &mut Readings<'_>,
    held: &mut Readings<'_>,
    orders: &[Order],
    spend: &mut Spend,
) -> Option<Choice> {
    let mut built = Vec::with_capacity(orders.len());
    for o in orders {
        let (tree, at_cap) = tree_of(clauses, train, &o.order, spend)?;
        let t = Instant::now();
        let before = held.reads();
        let score = tree.score(held);
        let new = held.reads() - before;
        spend.charge_reads(new, clauses, held.sample());
        spend.stages.reads[1] += new;
        spend.stages.held += t.elapsed();
        if spend.out() {
            return None;
        }
        built.push(Built {
            source: o.source,
            tree,
            score,
            at_cap,
        });
    }
    let mut best: Option<usize> = None;
    for (i, b) in built.iter().enumerate().skip(1) {
        if best.is_none_or(|j| b.score < built[j].score) {
            best = Some(i);
        }
    }
    let base = built.first()?.score;
    let picked = best
        .filter(|&j| built[j].score < base - MARGIN)
        .unwrap_or(0);
    Some(Choice { built, picked })
}

/// The tree of `order` from the building draw's readings `train`, and whether
/// a node of it reads at the draw's cap; none when the allowance runs out.
fn tree_of(
    clauses: &ClauseList,
    train: &mut Readings<'_>,
    order: &[usize],
    spend: &mut Spend,
) -> Option<(Tree, bool)> {
    let sample = train.sample();
    let t = Instant::now();
    let found = interval_matrix(train, order, || {
        spend.charge_reads(1, clauses, sample);
        spend.stages.reads[0] += 1;
        !spend.out()
    });
    spend.stages.readings += t.elapsed();
    let (bounds, r) = found?;
    let t = Instant::now();
    let g = bounds.len() - 1;
    let tree = Tree::new(order.to_vec(), &bounds, &join_split(&r, g, &bounds));
    let at_cap = reads_at_cap(&tree, &bounds, &r, sample.cap());
    spend.stages.brackets += t.elapsed();
    Some((tree, at_cap))
}

/// Whether a node of `tree` other than the root, a run of whole blocks of
/// `bounds`, reads at `cap` in the interval matrix `r`.
fn reads_at_cap(tree: &Tree, bounds: &[usize], r: &[f64], cap: f64) -> bool {
    let g = bounds.len() - 1;
    let block = |p: u32| bounds.binary_search(&(p as usize)).ok();
    tree.run[..tree.run.len() - 1]
        .iter()
        .any(|&(lo, hi)| match (block(lo), block(hi)) {
            (Some(s), Some(t)) if t > s => r[s * g + t - 1] >= cap,
            _ => false,
        })
}

/// The formula's clauses as DIMACS literals.
fn dimacs(formula: &CnfFormula) -> Vec<Vec<i32>> {
    formula
        .clauses()
        .iter()
        .map(|c| c.literals.iter().map(|l| l.to_dimacs()).collect())
        .collect()
}

/// The orders of `candidates` from `source`'s weighted sets, unless the
/// allowance runs out. A recursive candidate after the flat one it refines
/// starts from that one's order.
fn candidate_orders(
    n: usize,
    source: Source,
    sets: &[(Vec<u32>, f64)],
    candidates: &[Candidate],
    spend: &mut Spend,
) -> Option<Vec<Order>> {
    let t = Instant::now();
    let mut out: Vec<Order> = Vec::with_capacity(candidates.len());
    for &c in candidates {
        if spend.out() {
            break;
        }
        let first = out
            .iter()
            .find(|o| c.refines(o.candidate))
            .map(|o| o.order.clone());
        out.push(Order {
            source,
            candidate: c,
            order: orders::order(c, n, sets, false, first).0,
        });
    }
    spend.stages.orders += t.elapsed();
    (out.len() == candidates.len()).then_some(out)
}

/// The learned tree, with its figures and the portfolio tree's when there is
/// a portfolio tree.
struct Learned {
    tree: Tree,
    figures: Option<(Figures, Figures)>,
    /// The blend's pick at the gap and its figures, when the build went on
    /// past the gap.
    at_gap: Option<(Tree, Figures)>,
}

/// The repair sets of the building draw, repaired while what is left of the
/// allowance stays above `reserve`. The repairs found before the allowance
/// runs out are kept.
fn repairs_within(
    formula: &CnfFormula,
    train_draw: &ModelMatrix,
    reserve: Duration,
    spend: &mut Spend,
) -> Sets {
    let pairs = repair_pairs(train_draw);
    let Some(mut session) =
        models::RepairSession::new(formula, train_draw, REPAIR_CONFLICTS, REPAIR_SEED)
    else {
        return Sets::new();
    };
    let (found, _) = repair(&mut session, &pairs, spend, reserve);
    repair_sets(&pairs, &found)
}

/// The learned tree of `formula`, of at least two variables, from draws of at
/// most `most` models; none when the formula has no model, the allowance runs
/// out before a tree is built and scored, or the blend tree's `s` is more
/// than [`GAP_STOP`] above `portfolio`'s. Without `repairs`, or when the
/// blend tree's Omega fails [`OMEGA_MARGIN`] ([`after_gap`]), the build stops
/// there unless the blend tree's `s` is at least [`S_MARGIN`] below, and
/// does not repair.
fn learn(
    formula: &CnfFormula,
    most: usize,
    repairs: bool,
    portfolio: Option<&Tree>,
    spend: &mut Spend,
) -> Option<Learned> {
    let n = formula.num_vars() as usize;
    let mut count = BUILD_DRAW.min(most);
    spend.stages.models = count;
    let held_count = HELD_DRAW.min(most);
    let train_draw = draw(formula, BUILD_SEED, count, held_count, spend, 0)?;
    let clauses = ClauseList::new(&dimacs(formula));
    let train = ModelSample::new(n, &clauses, train_draw.to_rows());
    let mut train_readings = Readings::new(&clauses, &train);
    let before_blend = spend.used();
    let blend = blend_sets(formula, &train_draw, spend)?;
    // The other candidates' orders wait for the early stop, which reads only
    // the first's.
    let mut orders = candidate_orders(n, Source::Blend, &blend, &CANDIDATES[..1], spend)?;
    if let Some(portfolio) = portfolio {
        let (tree, _) = tree_of(&clauses, &mut train_readings, &orders[0].order, spend)?;
        let s = figures::static_score(&tree, &clauses, &mut train_readings, spend)?;
        let theirs = figures::static_score(portfolio, &clauses, &mut train_readings, spend)?;
        spend.stages.early = Some((s - theirs, spend.used()));
        if s - theirs > EARLY_STOP {
            spend.stages.stopped = true;
            return None;
        }
    }
    orders.extend(candidate_orders(
        n,
        Source::Blend,
        &blend,
        &CANDIDATES[1..],
        spend,
    )?);
    let before_held = spend.used();
    let held_draw = draw(formula, HELD_SEED, held_count, 0, spend, 1)?;
    let held_cost = spend.used().saturating_sub(before_held);
    let held = ModelSample::new(n, &clauses, held_draw.to_rows());
    let mut held_readings = Readings::new(&clauses, &held);
    let mut choice = choose(
        &clauses,
        &mut train_readings,
        &mut held_readings,
        &orders,
        spend,
    )?;
    let reserve = spend
        .used()
        .saturating_sub(before_blend)
        .saturating_sub(held_cost)
        .mul_f64(REPAIR_RESERVE);
    let mut repairs = repairs;
    let mut at_gap = None;
    if let Some(portfolio) = portfolio {
        let tree = &choice.built[choice.picked].tree;
        let s = figures::static_score(tree, &clauses, &mut held_readings, spend)?;
        let theirs = figures::static_score(portfolio, &clauses, &mut held_readings, spend)?;
        spend.stages.gap = Some(s - theirs);
        let omega = (
            figures::omega(tree, &clauses),
            figures::omega(portfolio, &clauses),
        );
        spend.stages.omega = Some(omega);
        let (stop, then_repairs) = after_gap(repairs, omega);
        repairs = then_repairs;
        if s - theirs > stop {
            spend.stages.stopped = true;
            return None;
        }
        at_gap = Some((tree.clone(), Figures { s, omega: omega.0 }));
    }
    let sets = if repairs {
        repairs_within(formula, &train_draw, reserve, spend)
    } else {
        Sets::new()
    };
    spend.stages.minimal += sets.len();
    if !sets.is_empty()
        && let Some(more) = candidate_orders(n, Source::Repairs, &sets, &CANDIDATES, spend)
    {
        let mut all = orders.clone();
        all.extend(more);
        if let Some(both) = choose(
            &clauses,
            &mut train_readings,
            &mut held_readings,
            &all,
            spend,
        ) {
            (orders, choice) = (all, both);
        }
    }
    while choice.built[choice.picked].at_cap && 2 * count <= most {
        let Some(rows) = draw(formula, BUILD_SEED, 2 * count, 0, spend, 0) else {
            break;
        };
        let train = ModelSample::new(n, &clauses, rows.to_rows());
        let mut readings = Readings::new(&clauses, &train);
        let Some(more) = choose(&clauses, &mut readings, &mut held_readings, &orders, spend) else {
            break;
        };
        count *= 2;
        spend.stages.models = count;
        choice = more;
    }
    let picked = choice.built.swap_remove(choice.picked);
    spend.stages.source = Some(picked.source);
    let tree = picked.tree;
    let figures = match portfolio {
        None => None,
        Some(portfolio) => {
            let mut figures_of = |tree: &Tree, which: usize| {
                let t = Instant::now();
                let s = figures::static_score(tree, &clauses, &mut held_readings, spend);
                let found = s.map(|s| Figures {
                    s,
                    omega: figures::omega(tree, &clauses),
                });
                spend.stages.figures[which] += t.elapsed();
                found
            };
            let mine = figures_of(&tree, 0)?;
            Some((mine, figures_of(portfolio, 1)?))
        }
    };
    Some(Learned {
        tree,
        figures,
        at_gap,
    })
}

/// The learned tree of `formula`, or none when the formula has no model or
/// the allowance runs out first.
#[cfg(test)]
pub(crate) fn build(formula: &CnfFormula, allowance: &Allowance) -> Option<Vtree> {
    let n = formula.num_vars() as usize;
    if n < 2 {
        return (n == 1).then(|| Vtree::balanced(1));
    }
    learn(formula, DRAW, true, None, &mut Spend::new(allowance)).map(|l| l.tree.to_vtree())
}

/// One tree's figures.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Figures {
    /// `S + W / 8` on the held-out draw.
    pub s: f64,
    /// The most orphans of a node.
    pub omega: usize,
}

/// The portfolio tree's figures that need no models.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Prior {
    /// `s` with every reading at its cap and no draw (`figures::static_bound`).
    pub s_bound: f64,
    /// The largest static width of a node, `W`.
    pub width: usize,
}

/// [`Prior`] of `portfolio`, a tree of `formula`.
pub(crate) fn prior(formula: &CnfFormula, portfolio: &Vtree) -> Prior {
    let clauses = ClauseList::new(&dimacs(formula));
    let tree = Tree::from_vtree(portfolio);
    let (s_bound, width) = figures::static_bound(&tree, &clauses);
    Prior { s_bound, width }
}

/// What the learned tree's build may spend, the models in each draw, and
/// whether it repairs ([`REPAIR_SHARE`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Plan {
    pub nominal: Duration,
    pub draw: usize,
    pub repairs: bool,
}

/// What the learned tree of `formula` may spend out of `share`, and how many
/// models it draws, given the portfolio tree's [`Prior`]; none when the build
/// is not worth starting.
///
/// What the portfolio tree leaves to gain grows as `2^(PAYOFF_PER_BIT *
/// s_bound)` and caps the allowance; the build's expected cost grows with
/// the formula's variables and clauses. The build is skipped when the
/// portfolio tree's `s_bound` is below [`MIN_S_BOUND`], or when the cost
/// times [`BUILD_SLACK`] does not fit the capped allowance, and its draws
/// shrink with the gain over the cost below [`FULL_DRAW_RATIO`]. It repairs
/// when `share` is at least [`REPAIR_SHARE`].
pub(crate) fn allowance(formula: &CnfFormula, prior: &Prior, share: Duration) -> Option<Plan> {
    if prior.s_bound < MIN_S_BOUND {
        return None;
    }
    let payoff = PAYOFF_MS * (PAYOFF_PER_BIT * prior.s_bound).exp2();
    let cap = share.min(millis(payoff));
    let cost = build_cost(formula);
    let fraction = (payoff / (FULL_DRAW_RATIO * cost)).clamp(MIN_DRAW as f64 / DRAW as f64, 1.0);
    (millis(BUILD_SLACK * cost) <= cap).then(|| Plan {
        nominal: cap,
        draw: (DRAW as f64 * fraction).round() as usize,
        repairs: share >= REPAIR_SHARE,
    })
}

/// Whether the learned tree's build could fit `share` whatever the
/// portfolio tree leaves to gain: when it does not, [`allowance`] skips the
/// build for every [`Prior`], so the prior need not be read.
pub(crate) fn may_fit(formula: &CnfFormula, share: Duration) -> bool {
    millis(BUILD_SLACK * build_cost(formula)) <= share
}

/// The learned tree's expected build over `formula`, in ms.
fn build_cost(formula: &CnfFormula) -> f64 {
    let n = f64::from(formula.num_vars());
    let m = formula.clauses().len().max(1) as f64;
    BUILD_MS * n.powf(BUILD_PER_VAR) * m.powf(BUILD_PER_CLAUSE)
}

fn millis(ms: f64) -> Duration {
    Duration::try_from_secs_f64(ms / 1000.0).unwrap_or(Duration::MAX)
}

/// After the blend's candidates, the gap in `s` above which the build stops,
/// and whether it repairs, given whether the plan `repairs` and the blend
/// tree's and the portfolio tree's Omegas `omega`. A blend tree whose Omega
/// fails [`OMEGA_MARGIN`] goes on as without repairs: the repairs' trees have
/// not brought an Omega back under the margin, while a larger building draw
/// has.
fn after_gap(repairs: bool, omega: (usize, usize)) -> (f64, bool) {
    let repairs = repairs && omega_within(omega.0, omega.1);
    (if repairs { GAP_STOP } else { -S_MARGIN }, repairs)
}

/// The take test: whether a learned tree of figures `mine` is taken over a
/// portfolio tree of figures `theirs`, its `s` lower by at least
/// [`S_MARGIN`] and its Omega within [`OMEGA_MARGIN`].
fn takes(mine: Figures, theirs: Figures) -> bool {
    mine.s <= theirs.s - S_MARGIN && omega_within(mine.omega, theirs.omega)
}

/// Of the last pick, of figures `last`, and the blend's pick at the gap, the
/// tree the take test takes over a portfolio tree of figures `theirs`: the
/// last pick when it passes, else the gap's when it passes, else none. With
/// the figures of the tree taken (of the last pick when none is), and whether
/// that was the gap's.
fn taken<T>(
    last: (T, Figures),
    at_gap: Option<(T, Figures)>,
    theirs: Figures,
) -> (Option<T>, Figures, bool) {
    if takes(last.1, theirs) {
        return (Some(last.0), last.1, false);
    }
    match at_gap {
        Some((tree, mine)) if takes(mine, theirs) => (Some(tree), mine, true),
        _ => (None, last.1, false),
    }
}

/// Whether a learned tree of Omega `mine` may be taken over a portfolio tree
/// of Omega `theirs`: less than [`OMEGA_MARGIN`] above it.
fn omega_within(mine: usize, theirs: usize) -> bool {
    mine < theirs + OMEGA_MARGIN
}

/// What [`pick`] found.
pub(crate) struct Verdict {
    /// The learned tree when the pick takes it.
    pub tree: Option<Vtree>,
    /// The learned tree's figures, then the portfolio tree's; none when the
    /// learned tree was not built and scored inside the allowance.
    pub figures: Option<(Figures, Figures)>,
    /// What the build and the figures spent, in the clock's currency.
    pub spent: Duration,
    pub stages: Stages,
}

/// The learned tree of `formula`, from draws of at most `count` models and
/// with repairs when `repairs`, when it beats `portfolio`, the portfolio's
/// tree of the same formula: its `s` lower by at least [`S_MARGIN`] and its
/// Omega less than [`OMEGA_MARGIN`] above ([`takes`]). When the last pick
/// fails that test but the blend's pick at the gap passed it, the gap's tree
/// is taken ([`taken`]). No tree when neither passes, when the formula has no model, when the gap after the blend's
/// candidates stops the build ([`GAP_STOP`], or [`S_MARGIN`] without
/// repairs), or when the allowance runs out before both trees are scored.
pub(crate) fn pick(
    formula: &CnfFormula,
    portfolio: &Vtree,
    count: usize,
    repairs: bool,
    allowance: &Allowance,
) -> Verdict {
    let mut spend = Spend::new(allowance);
    let other = Tree::from_vtree(portfolio);
    let learned = learn(formula, count, repairs, Some(&other), &mut spend);
    let spent = spend.used();
    let mut stages = spend.stages;
    let Some(Learned {
        tree,
        figures: Some((mine, theirs)),
        at_gap,
    }) = learned
    else {
        return Verdict {
            tree: None,
            figures: None,
            spent,
            stages,
        };
    };
    let (tree, mine, from_gap) = taken((tree, mine), at_gap, theirs);
    stages.from_gap = from_gap;
    Verdict {
        tree: tree.map(|t| t.to_vtree()),
        figures: Some((mine, theirs)),
        spent,
        stages,
    }
}
