//! The search over readings: build one tree decomposition every way the
//! caller left open, score each with [`vtree_cost`], and return the cheapest.
//!
//! A conversion is a search. [`Reading`] says which of its three dimensions the
//! caller has named — a named dimension is fixed, an unnamed one is walked — so
//! naming all three is a search of exactly one reading and naming none is the
//! whole of it. There is no second entry point that converts "once": a single
//! conversion is this search with nothing left open.
//!
//! The order is fixed, so truncation is predictable. Every candidate root is
//! screened under one reading first — the first row of [`PLACES`] and of
//! [`BINARIZATIONS`] — and the best few screened roots then get every remaining
//! (place, binarize) pair, places in [`PLACES`] order and binarizations in
//! [`BINARIZATIONS`] order within each. The deadline is tested BETWEEN readings and only once one has
//! been adopted, so a bounded conversion always returns a vtree.
//!
//! The [`TdConversionMeta`] handed back carries the WINNING reading's bag
//! metadata: each reading builds its own bag assignment, and only that one
//! describes the returned tree.

use std::sync::Arc;
use std::time::Instant;

use crate::diagnostics::diag;
use crate::score::{BUILT_FROM_THIS_FORMULA, vtree_cost};
use crate::vtree::Vtree;

use super::super::TreeDecomposition;
use super::super::best::BestBy;
use super::algo::{ConversionInput, convert_one, root_bags};
use super::meta::BagMetadata;
use super::reading::{BINARIZATIONS, Binarization, FixedReading, PLACES, Reading, Root, RootPick};

/// How many candidate roots the search enumerates at most. The screen is one
/// O(n) build and one O(1) score per root, so the cap is about the tail of a
/// decomposition with thousands of leaf bags, not about the first few.
const ROOT_CAP: usize = 20;

/// The binarization a conversion with no CNF runs at. Every other one reads
/// clauses, so without one they all build what this one builds — naming it is
/// what keeps the reading a formula-less conversion reports the reading it ran.
const UNSCORED_BINARIZATION: Binarization = Binarization::Balanced;

/// How many screened roots are carried into the remaining (place, binarize) pairs.
const SCREENED_ROOTS: usize = 3;

/// What a TD → vtree conversion produced BESIDE the tree, returned alongside it
/// so the two can never be paired up wrongly.
///
/// Consumers must still validate `meta.num_vars()` against the formula they
/// intend to use it with: a nested conversion over a sub-decomposition describes
/// a DIFFERENT variable space. Metadata only steers clause ORDER, never the
/// compiled function, so a mismatch costs scheduling quality, not correctness.
#[derive(Clone, Default)]
pub(crate) struct TdConversionMeta {
    /// Bag metadata of the WINNING reading — the one that produced the returned
    /// vtree, never a runner-up's. `None` when the vtree did not come from a TD
    /// conversion at all.
    pub meta: Option<Arc<BagMetadata>>,
}

/// Everything a conversion is asked for beyond the decomposition itself: which
/// dimensions are already named, what it may spend, and how it reports.
///
/// One value rather than four arguments, because every family passes all four
/// and a family that dropped one would be reading its decomposition under a
/// different rule than the rest.
#[derive(Clone, Copy)]
pub(crate) struct ConversionRequest<'a> {
    /// The `--vtree` spec naming the construction this conversion serves, for
    /// the one line it reports. `None` for a conversion nested inside another
    /// construction, which reports nothing: its caller reports for it.
    pub spec: Option<&'a str>,
    /// Which of the three dimensions the caller named. The rest are searched.
    pub reading: Reading,
    /// Effort multiplier for the one binarization that spends a scalable budget.
    pub effort_scale: f64,
    /// Absolute wall-clock deadline. Truncates the search between readings,
    /// never before the first has completed.
    pub deadline: Option<Instant>,
    /// Report every reading, not just the winner (`VITRI_CONVERSION_TRACE`).
    pub trace: bool,
}

impl<'a> ConversionRequest<'a> {
    /// A conversion asked for from outside any construction: at baseline
    /// effort, reporting nothing, under whatever the caller named and whatever
    /// bound it gave. The public entry point is one such caller.
    pub(crate) fn open(reading: Reading, deadline: Option<Instant>) -> ConversionRequest<'static> {
        ConversionRequest {
            spec: None,
            reading,
            effort_scale: 1.0,
            deadline,
            trace: false,
        }
    }

    /// A conversion nested inside another construction: it reports nothing, and
    /// reads the decomposition the way the construction around it was asked to.
    pub(crate) fn nested(&self) -> ConversionRequest<'a> {
        ConversionRequest {
            spec: None,
            trace: false,
            ..*self
        }
    }
}

/// What one finished conversion has to say about itself: the reading it kept,
/// what that reading scored, and how much of the search it got through.
struct ConversionReport {
    winner: FixedReading,
    cost: Option<u64>,
    done: usize,
    planned: usize,
}

impl ConversionReport {
    /// THE wording of the conversion's own report line, so a caller cannot
    /// spell one differently from another.
    fn emit(&self, spec: &str) {
        diag!(
            "[conversion] {spec}: {} cost={} readings={}/{}",
            self.winner,
            self.cost.map_or_else(|| "-".to_string(), |c| c.to_string()),
            self.done,
            self.planned,
        );
    }
}

/// THE tree-decomposition → vtree conversion: search every reading `request`
/// leaves open and return the cheapest tree, with the bag metadata of the
/// reading that produced it.
///
/// Without a formula there is nothing to score a reading against, so the search
/// is one reading long whatever was left open — the same rule as a caller who
/// named all three.
pub(crate) fn convert(
    input: ConversionInput<'_>,
    request: ConversionRequest<'_>,
) -> (Vtree, TdConversionMeta) {
    // Precondition: a non-empty tree decomposition. A 0-variable formula has no
    // vtree to build — callers must short-circuit it before reaching here.
    assert!(
        !input.td.adj.is_empty(),
        "convert: empty tree decomposition (num_vars={}); callers must short-circuit \
         0-variable formulas before vtree construction",
        input.num_vars,
    );
    // What one reading costs, in the construction meter's graph-element unit:
    // realizing a vtree from the decomposition is linear in its bags, and
    // scoring the result is linear in the formula it is scored against. Summing
    // the clause lengths is itself a pass over the formula, so it is done once
    // here rather than once per reading, and not at all when nothing is
    // metering.
    let reading_units: u64 = if crate::decompose::meter::metering() {
        input.num_vars as u64
            + input.td.adj.len() as u64
            + input.formula.map_or(0, |f| {
                f.clauses
                    .iter()
                    .map(|c| c.literals.len() as u64)
                    .sum::<u64>()
            })
    } else {
        0
    };

    let scored = input.formula.is_some();
    let roots = candidate_roots(input.td, request.reading.root, scored);
    let places = axis(request.reading.place, PLACES, scored, PLACES[0].1);
    let binarizations = axis(
        request.reading.binarize,
        BINARIZATIONS,
        scored,
        UNSCORED_BINARIZATION,
    );

    // The plan, fixed before the first build so a truncated search reports what
    // it set out to do rather than what it managed.
    let planned =
        roots.len() + (places.len() * binarizations.len() - 1) * roots.len().min(SCREENED_ROOTS);

    let mut search = Search {
        input,
        request,
        best: BestBy::new(),
        winner: FixedReading {
            root: roots[0],
            place: places[0],
            binarize: binarizations[0],
        },
        best_score: None,
        done: 0,
        reading_units,
    };

    // The screen: every candidate root under the first (place, binarize) pair. Its
    // scores are what ranks the roots for everything below.
    let mut screened: Vec<(RootPick, u64)> = Vec::with_capacity(roots.len());
    for &root in &roots {
        let Some(score) = search.offer(FixedReading {
            root,
            place: places[0],
            binarize: binarizations[0],
        }) else {
            break;
        };
        screened.push((root, score));
    }
    // Lower is better; the sort is stable, so equal-scoring roots keep the
    // order they were enumerated in.
    screened.sort_by_key(|&(_, score)| score);
    screened.truncate(SCREENED_ROOTS);

    // Every remaining (place, binarize) pair over the roots the screen liked.
    'pairs: for &place in &places {
        for &binarize in &binarizations {
            if (place, binarize) == (places[0], binarizations[0]) {
                continue;
            }
            for &(root, _) in &screened {
                if search
                    .offer(FixedReading {
                        root,
                        place,
                        binarize,
                    })
                    .is_none()
                {
                    break 'pairs;
                }
            }
        }
    }

    let report = ConversionReport {
        winner: search.winner,
        cost: search.best_score.filter(|_| scored),
        done: search.done,
        planned,
    };
    if let Some(spec) = request.spec {
        report.emit(spec);
    }

    let (vtree, meta) = search.best.into_best().expect("at least one reading").0;
    // A malformed TD (phantom vertices, inconsistent adjacency) can leak extra
    // leaves into the vtree.
    assert_eq!(
        vtree.num_leaves(),
        input.num_vars,
        "the conversion produced a malformed vtree: {} leaves for a {}-variable formula",
        vtree.num_leaves(),
        input.num_vars,
    );
    (
        vtree,
        TdConversionMeta {
            meta: Some(Arc::new(meta)),
        },
    )
}

/// The running state of one search: what has been offered, what is winning, and
/// how far the deadline let it get.
struct Search<'a, 'b> {
    input: ConversionInput<'a>,
    request: ConversionRequest<'b>,
    best: BestBy<(Vtree, BagMetadata), u64>,
    /// The reading behind whatever `best` is holding.
    winner: FixedReading,
    /// Its score. `None` until the first reading is adopted.
    best_score: Option<u64>,
    done: usize,
    /// What one reading costs the construction meter, charged in
    /// [`Search::offer`].
    reading_units: u64,
}

impl Search<'_, '_> {
    /// Build and score one reading, keeping it if it is the cheapest so far.
    /// `None` means the deadline stopped the search — which it can only do once
    /// a reading has been adopted, so the caller always has a tree.
    fn offer(&mut self, reading: FixedReading) -> Option<u64> {
        if self.best.has_candidate() && crate::budget::expired(self.request.deadline) {
            return None;
        }
        // Every reading the search builds passes through here, which is what
        // makes this the one place it pays for what it considers — and so what
        // makes the deadline test above a bound on the search's own work.
        crate::decompose::meter::charge(self.reading_units);
        let started = Instant::now();
        let built = convert_one(self.input, reading);
        // Without a formula every reading is unscorable and the first is kept.
        let score = self
            .input
            .formula
            .map(|f| vtree_cost(&built.0, f).expect(BUILT_FROM_THIS_FORMULA))
            .unwrap_or(0);
        if self.request.trace
            && let Some(spec) = self.request.spec
        {
            diag!(
                "[conversion] reading {spec} {reading} cost={score} ms={}",
                started.elapsed().as_millis(),
            );
        }
        // `BestBy` keeps the first of equally-scoring candidates, so the
        // reading recorded here must move on exactly the same condition.
        if self.best_score.is_none_or(|best| score < best) {
            self.winner = reading;
            self.best_score = Some(score);
        }
        self.best.offer(built, score);
        self.done += 1;
        Some(score)
    }
}

/// The roots the search enumerates: every bag the caller's [`Root`] admits,
/// capped at [`ROOT_CAP`].
///
/// `root=leaf` admits a set rather than one bag, so naming it still leaves a
/// search; naming nothing admits the first bag, the centroid and the leaf bags
/// together. A leaf bag that is already a component's first or centroid root is
/// left out of that combined list, and so is the centroid when it roots every
/// component exactly where the first bag does — two spellings of one reading
/// would cost a build to discover they score the same.
///
/// Never empty: a decomposition with no leaf bag at all still has a first bag,
/// and `root=leaf` on one falls back to it rather than converting nothing.
fn candidate_roots(td: &TreeDecomposition, named: Option<Root>, scored: bool) -> Vec<RootPick> {
    let mut roots: Vec<RootPick> = Vec::new();
    let mut taken: Vec<usize> = Vec::new();
    if named != Some(Root::Leaf) {
        roots.push(RootPick::First);
        taken.extend(root_bags(td, RootPick::First));
    }
    if named.is_none() {
        let centroid = root_bags(td, RootPick::Centroid);
        if centroid != taken {
            roots.push(RootPick::Centroid);
            taken.extend(centroid);
        }
    }
    if matches!(named, None | Some(Root::Leaf)) {
        for bag in 0..td.adj.len() {
            if roots.len() >= ROOT_CAP {
                break;
            }
            if td.adj[bag].len() == 1 && !taken.contains(&bag) {
                roots.push(RootPick::Leaf(bag));
            }
        }
    }
    if named == Some(Root::Centroid) {
        roots.push(RootPick::Centroid);
    }
    if roots.is_empty() {
        roots.push(RootPick::First);
    }
    if !scored {
        roots.truncate(1);
    }
    roots
}

/// The values one axis is searched over: the one the caller named, or every
/// value in table order. With no formula there is nothing to rank readings by,
/// so the axis collapses to `unscored` and the search is one reading long.
fn axis<T: Copy>(
    named: Option<T>,
    table: &[(&'static str, T)],
    scored: bool,
    unscored: T,
) -> Vec<T> {
    match named {
        Some(v) => vec![v],
        None if !scored => vec![unscored],
        None => table.iter().map(|(_, v)| *v).collect(),
    }
}
