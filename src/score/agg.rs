//! A whole-tree ranker over aggregates of the per-node quantities: linear, or
//! a boosted pairwise model over the same inputs.
//!
//! The portfolio selects on the ranker shipped in this crate unless
//! `VITRI_SCORE_AGG` says otherwise: `cost` selects on [`super::vtree_cost`]
//! alone, and a path names another ranker file.
//!
//! The quantities are the 38 columns [`super::tables`] computes at each
//! internal node. Each column is reduced over ALL internal nodes of the tree by
//! one of five aggregates, standardized, and summed with the eleven addends of
//! the structural cost ([`super::unified_cost_terms`]). Lower is better, and
//! the portfolio takes the argmin within a component.
//!
//! The model is data, not code: a JSON file exported by the fit that produced
//! the weights, the shipped one read from [`load::DEFAULT_MODEL`]. A file naming a
//! column, an aggregate or a cost term this crate has no definition for is
//! refused at load.
//!
//! Two kinds of file. `agg-linear` scores each candidate on its own: the
//! intercept, plus each cost addend at its weight, plus each standardized
//! aggregate at its weight. `agg-pair-boost` scores candidates against each
//! other: a gradient-boosted ensemble reads the DIFFERENCE of two candidates'
//! raw input vectors and predicts the probability that the first is the larger
//! compile, and a candidate's score is its mean probability of being larger
//! than each sibling. Both kinds: lower is better, argmin within a component.

mod load;

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::cnf::CnfFormula;
use crate::error::VitriError;
use crate::vtree::Vtree;

use load::feature_name;
pub(crate) use load::{AGG_VAR, COST_ONLY, load, model, ranker_from_env};

use super::VtreeScores;
use super::tables::{Feature, Tables};

// ---------------------------------------------------------------------------
// The aggregates
// ---------------------------------------------------------------------------

/// How one column is reduced over the internal nodes of a tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Aggregate {
    Max,
    Mean,
    /// The 90th percentile, interpolated linearly between the two neighbouring
    /// order statistics.
    P90,
    /// The 99th, the same way.
    P99,
    /// `log2 Σ 2^v` over the strictly positive entries, computed max-shifted.
    Lse,
}

/// Every aggregate under the name the model file uses for it.
const AGGREGATE_NAMES: [(&str, Aggregate); 5] = [
    ("max", Aggregate::Max),
    ("mean", Aggregate::Mean),
    ("p90", Aggregate::P90),
    ("p99", Aggregate::P99),
    ("lse", Aggregate::Lse),
];

impl Aggregate {
    fn from_name(name: &str) -> Option<Aggregate> {
        AGGREGATE_NAMES
            .iter()
            .find(|(known, _)| *known == name)
            .map(|&(_, agg)| agg)
    }

    /// Reduce the values a column took over the internal nodes.
    ///
    /// `values` holds one entry per node the column has a value at, which is
    /// every internal node except that the four cut columns have none at the
    /// root — the cut pass produces no row there, and neither does the table
    /// this is checked against. An aggregate over no entries is 0, for `lse`
    /// and for the other four alike. The reference implementation instead
    /// carries such a column through as NaN and drops the tree from the fit, so
    /// a 0 here is a tree that fit never saw; [`agg_score`] says so on stderr
    /// when it happens.
    ///
    /// `values` is sorted in place by the two percentiles.
    fn of(self, values: &mut [f64]) -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        match self {
            Aggregate::Max => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            Aggregate::Mean => values.iter().sum::<f64>() / values.len() as f64,
            Aggregate::P90 => percentile(values, 90.0),
            Aggregate::P99 => percentile(values, 99.0),
            Aggregate::Lse => super::log2_sum_exp(values),
        }
    }
}

/// The `q`th percentile with linear interpolation, which is what
/// `numpy.percentile` computes by default.
///
/// The position is `q/100` of the way through the order statistics; when it
/// falls between two of them the answer is interpolated between the pair. The
/// two-sided form is numpy's own: it interpolates from whichever end is nearer,
/// so neither endpoint is recovered through a cancelling subtraction.
fn percentile(values: &mut [f64], q: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    let last = values.len() - 1;
    let pos = q / 100.0 * last as f64;
    let below = pos.floor();
    let index = below as usize;
    if index >= last {
        return values[last];
    }
    let (a, b) = (values[index], values[index + 1]);
    let t = pos - below;
    let span = b - a;
    if t <= 0.5 {
        a + span * t
    } else {
        b - span * (1.0 - t)
    }
}

// ---------------------------------------------------------------------------
// The model file
// ---------------------------------------------------------------------------

/// One node of one boosted tree, indices into the tree's own node table.
#[derive(Clone, Debug)]
enum Node {
    Leaf(f64),
    Split {
        /// Index into [`AggModel::inputs`].
        input: usize,
        /// `<=` goes left.
        threshold: f64,
        left: usize,
        right: usize,
    },
}

/// One entry of the boosted kind's input vector.
#[derive(Clone, Copy, Debug)]
enum Input {
    /// A cost addend, by position in [`COST_TERM_NAMES`].
    Term(usize),
    /// An entry of [`AggModel::aggregates`], raw (its mean and sd unused).
    Aggregate(usize),
}

/// How a loaded model turns a candidate's numbers into a score.
enum Scorer {
    Linear,
    PairBoost {
        baseline: f64,
        trees: Vec<Vec<Node>>,
    },
}

/// What the ranker computed for one candidate: the linear kind's score, or the
/// boosted kind's raw input vector, which only becomes a score once the
/// component's candidates are all known ([`round_robin`]).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AggScore {
    Scalar(f64),
    Inputs(Vec<f64>),
}

impl AggScore {
    /// The score, once there is one.
    pub(crate) fn scalar(&self) -> Option<f64> {
        match self {
            AggScore::Scalar(s) => Some(*s),
            AggScore::Inputs(_) => None,
        }
    }
}

/// One standardized aggregate of one column, with the weight it enters at.
struct AggTerm {
    /// Which of [`AggModel::columns`] this reduces.
    column: usize,
    agg: Aggregate,
    mean: f64,
    sd: f64,
    weight: f64,
}

impl AggTerm {
    /// The name the model file spells this reduction as.
    fn agg_name(&self) -> &'static str {
        AGGREGATE_NAMES
            .iter()
            .find(|(_, known)| *known == self.agg)
            .map(|&(name, _)| name)
            .expect("every aggregate is in the name table")
    }
}

/// A fitted whole-tree ranker, ready to score a candidate.
pub(crate) struct AggModel {
    intercept: f64,
    /// One multiplier per addend of the structural cost, in
    /// [`COST_TERM_NAMES`] order. A term the file does not name enters at 0.
    terms: [f64; 11],
    /// The distinct columns the aggregates below read, gathered once per tree.
    columns: Vec<Feature>,
    aggregates: Vec<AggTerm>,
    /// The boosted kind's input vector, in file order; empty for the linear
    /// kind.
    inputs: Vec<Input>,
    scorer: Scorer,
}

impl AggModel {
    /// Whether this model scores candidates against each other, so a
    /// component's candidates have to be gathered before any of them has a
    /// score ([`round_robin`]).
    pub(crate) fn is_pairwise(&self) -> bool {
        matches!(self.scorer, Scorer::PairBoost { .. })
    }

    /// The ensemble's raw output on one difference vector: the baseline plus
    /// one leaf per tree. Children come after their parent, so the walk ends.
    fn raw_pair(&self, diff: &[f64]) -> f64 {
        let Scorer::PairBoost { baseline, trees } = &self.scorer else {
            unreachable!("raw_pair is the boosted kind's");
        };
        let mut sum = *baseline;
        for tree in trees {
            let mut at = 0;
            loop {
                match &tree[at] {
                    Node::Leaf(value) => {
                        sum += value;
                        break;
                    }
                    Node::Split {
                        input,
                        threshold,
                        left,
                        right,
                    } => {
                        at = if diff[*input] <= *threshold {
                            *left
                        } else {
                            *right
                        };
                    }
                }
            }
        }
        sum
    }

    /// Whether any column comes from the split pass, which decides whether the
    /// per-node tables pay for it.
    fn reads_split(&self) -> bool {
        self.columns.iter().any(|c| c.is_from_split())
    }

    /// The same for the cut pass, which is the more expensive of the two.
    fn reads_cut(&self) -> bool {
        self.columns.iter().any(|c| c.is_from_cut())
    }
}

// ---------------------------------------------------------------------------
// Scoring one candidate
// ---------------------------------------------------------------------------

/// What each column took over the internal nodes of the tree, in `columns`
/// order, one pass for all of them.
///
/// A column with no value at a node contributes no entry there, which is the
/// four cut columns at the root and nothing else — the cut pass writes no row
/// for it. Everywhere else every internal node contributes.
fn gather(vtree: &Vtree, tables: &Tables, columns: &[Feature]) -> Vec<Vec<f64>> {
    let mut gathered: Vec<Vec<f64>> = vec![Vec::new(); columns.len()];
    for (node, left, right) in vtree.internal_bottomup() {
        for (column, values) in columns.iter().zip(&mut gathered) {
            if column.is_from_cut() && !tables.has_cut_row(node) {
                continue;
            }
            values.push(tables.value(*column, node, left, right));
        }
    }
    gathered
}

/// Structural statistics and what `model` computes for `vtree` against
/// `formula`, sharing their clause and context tables. The linear kind: the
/// intercept, plus each addend of the structural cost at its weight, plus each
/// standardized aggregate at its weight, lower is better. The boosted kind: the
/// raw input vector, which [`round_robin`] turns into a score once the
/// component's candidates are all known.
///
/// # Errors
///
/// [`VitriError::Mismatch`] if `formula` names a variable `vtree` has no leaf
/// for.
pub(crate) fn agg_score(
    vtree: &Vtree,
    formula: &CnfFormula,
    model: &AggModel,
    show_mask: Option<&crate::cnf::ShowMask>,
) -> Result<(VtreeScores, AggScore), VitriError> {
    let (stats, terms, values) = agg_numbers(vtree, formula, model, show_mask)?;
    if model.is_pairwise() {
        let inputs = model
            .inputs
            .iter()
            .map(|input| match *input {
                Input::Term(at) => terms[at],
                Input::Aggregate(at) => values[at],
            })
            .collect();
        return Ok((stats, AggScore::Inputs(inputs)));
    }
    let mut score = model.intercept;
    for (weight, term) in model.terms.iter().zip(&terms) {
        score += weight * term;
    }
    for (entry, value) in model.aggregates.iter().zip(&values) {
        score += entry.weight * ((value - entry.mean) / entry.sd);
    }
    Ok((stats, AggScore::Scalar(score)))
}

/// The boosted kind's scores for one component: each candidate's mean predicted
/// probability of being the larger compile against each sibling, from the
/// input vectors [`agg_score`] produced. A lone candidate scores 0.
/// `families` gives equal total weight to each represented opponent family;
/// `None` gives equal weight to each opponent.
///
/// The ensemble is evaluated on every ordered pair, `n(n-1)` walks of a few
/// hundred shallow trees, which is nothing beside building one candidate.
pub(crate) fn round_robin(
    model: &AggModel,
    inputs: &[&[f64]],
    families: Option<&[&str]>,
) -> Vec<f64> {
    let n = inputs.len();
    if n < 2 {
        return vec![0.0; n];
    }
    let mut family_counts = HashMap::new();
    if let Some(families) = families {
        assert_eq!(families.len(), n);
        for family in families {
            *family_counts.entry(*family).or_insert(0usize) += 1;
        }
    }
    let mut scores = vec![0.0; n];
    let mut diff = vec![0.0; model.inputs.len()];
    for i in 0..n {
        let mut weight_sum = 0.0;
        for j in 0..n {
            if i == j {
                continue;
            }
            for (d, (a, b)) in diff.iter_mut().zip(inputs[i].iter().zip(inputs[j])) {
                *d = a - b;
            }
            let raw = model.raw_pair(&diff);
            let weight = families.map_or(1.0, |families| {
                let opponents =
                    family_counts[families[j]] - usize::from(families[i] == families[j]);
                1.0 / opponents as f64
            });
            scores[i] += weight / (1.0 + (-raw).exp());
            weight_sum += weight;
        }
        scores[i] /= weight_sum;
    }
    scores
}

/// The eleven cost addends and each aggregate's value over `vtree`, in
/// [`AggModel::aggregates`] order: the numbers both kinds read.
///
/// # Errors
///
/// [`VitriError::Mismatch`] if `formula` names a variable `vtree` has no leaf
/// for.
fn agg_numbers(
    vtree: &Vtree,
    formula: &CnfFormula,
    model: &AggModel,
    show_mask: Option<&crate::cnf::ShowMask>,
) -> Result<(VtreeScores, [f64; 11], Vec<f64>), VitriError> {
    super::covered_by(vtree, formula)?;
    let tables = Tables::build(vtree, formula, model.reads_split(), model.reads_cut());
    let peak_show = show_mask.map(|mask| {
        super::context_width_from_high_lca(vtree, tables.high_lca(), Some(mask))
            .into_iter()
            .max()
            .unwrap_or(0)
    });
    let (stats, terms) = VtreeScores::from_tables(vtree, formula, tables.cost_tables(), peak_show);

    let mut gathered = gather(vtree, &tables, &model.columns);

    let mut values = Vec::with_capacity(model.aggregates.len());
    for entry in &model.aggregates {
        let column = &mut gathered[entry.column];
        if column.is_empty() {
            // The fit's table carries this column as NaN over such a tree and
            // drops the row; scoring it against a 0 is the one place this
            // ranker can say something the fit never learned. Said once per
            // process, so a run that hits it is readable and one that hits it
            // on every component is still readable.
            static SAID: OnceLock<()> = OnceLock::new();
            SAID.get_or_init(|| {
                crate::diagnostics::diag!(
                    "[agg-pick] no {} to take the {} of over the {} internal node(s) of this \
                     tree; scoring it as 0 (said once)",
                    feature_name(model.columns[entry.column]),
                    entry.agg_name(),
                    vtree.internal_bottomup().count(),
                );
            });
        }
        // A non-finite aggregate enters as 0 before standardising, which is
        // what the fit did with one.
        let value = entry.agg.of(column);
        values.push(if value.is_finite() { value } else { 0.0 });
    }
    Ok((stats, terms, values))
}

// ---------------------------------------------------------------------------
// The switch
// ---------------------------------------------------------------------------

/// The variable that narrows the field the ranker chooses from: only the
/// candidates whose cost is within this much of the cost pick's cost are
/// eligible. Unset, the margin is [`DEFAULT_MARGIN`]; [`NO_MARGIN`] makes
/// every candidate eligible.
pub(crate) const MARGIN_VAR: &str = "VITRI_SCORE_AGG_MARGIN";

/// The margin a build under a ranker gets unless its caller or
/// `VITRI_SCORE_AGG_MARGIN` says otherwise. It leaves the ranker its picks on
/// most components and pins the rest to the cost pick, which is where the
/// ranker has traded a solve for speed.
pub const DEFAULT_MARGIN: f64 = 10.0;

/// The value of [`MARGIN_VAR`] that ranks every candidate.
pub(crate) const NO_MARGIN: &str = "none";

/// What the margin's value has to be, quoted in the message a bad one gets.
const MARGIN_EXPECTED: &str =
    "a cost margin in the cost's own units, zero or more, or `none` for every candidate";

/// How far above the cost pick's cost a candidate may sit and still be ranked:
/// [`DEFAULT_MARGIN`] when [`MARGIN_VAR`] is unset, `None` when it is
/// [`NO_MARGIN`] or there is no ranker. `ranker_on` is what [`model`] answered:
/// whether this process selects on a ranker at all.
///
/// # Errors
///
/// [`VitriError::Env`] when the margin is set under [`COST_ONLY`], where there
/// is no ranker to narrow, or to something that is not a margin.
pub(crate) fn margin_from_env(ranker_on: bool) -> Result<Option<f64>, VitriError> {
    let raw = crate::env::env_raw(MARGIN_VAR, MARGIN_EXPECTED)?;
    margin_from_value(raw.as_deref(), ranker_on)
}

/// [`margin_from_env`] filling in over a margin a caller already chose: the
/// variable wins when it is set, the caller's value stands when it is not, and
/// a build with no ranker has no field to narrow whichever said what.
///
/// # Errors
///
/// As [`margin_from_env`].
pub(crate) fn margin_over(
    current: Option<f64>,
    ranker_on: bool,
) -> Result<Option<f64>, VitriError> {
    match crate::env::env_raw(MARGIN_VAR, MARGIN_EXPECTED)? {
        Some(raw) => margin_from_value(Some(&raw), ranker_on),
        None if ranker_on => Ok(current),
        None => Ok(None),
    }
}

/// The pure half of [`margin_from_env`].
///
/// # Errors
///
/// [`VitriError::Env`] naming both variables when `raw` is `Some` and
/// `ranker_on` is false, or naming the margin when it does not read as one.
fn margin_from_value(raw: Option<&str>, ranker_on: bool) -> Result<Option<f64>, VitriError> {
    let Some(raw) = raw else {
        return Ok(ranker_on.then_some(DEFAULT_MARGIN));
    };
    if !ranker_on {
        return Err(VitriError::env(
            MARGIN_VAR,
            format!(
                "requires a ranker: it narrows the field the ranker chooses from, and under \
                 {AGG_VAR}={COST_ONLY} the cost picks alone. Unset {MARGIN_VAR}, or set {AGG_VAR} \
                 to a ranker."
            ),
        ));
    }
    if raw.trim() == NO_MARGIN {
        return Ok(None);
    }
    let margin: f64 = crate::env::parse_value(MARGIN_VAR, Some(raw), 0.0, MARGIN_EXPECTED)?;
    if !margin.is_finite() || margin < 0.0 {
        return Err(VitriError::env(
            MARGIN_VAR,
            format!("must be {MARGIN_EXPECTED}; got {raw:?}"),
        ));
    }
    Ok(Some(margin))
}

// ---------------------------------------------------------------------------
// Which component the pick line is about
// ---------------------------------------------------------------------------

thread_local! {
    /// Which independent component the build is on, as
    /// [`crate::component::build_vtree_split`] numbers them — which is the same
    /// numbering the written `components/compNNN` files carry. The library has
    /// no component identity of its own, and the pick line has to be joinable
    /// to an offline table by component, so the loop that splits the formula
    /// records the number here; the whole-formula path resets it.
    static COMPONENT: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
}

/// Record which component the build about to run is on; `None` for a build over
/// a whole formula that was never split.
pub(crate) fn set_component(index: Option<usize>) {
    COMPONENT.with(|slot| slot.set(index));
}

/// What the pick line calls the component it is about.
pub(crate) fn component_label() -> String {
    COMPONENT.with(|slot| match slot.get() {
        Some(index) => format!("comp{index:03}"),
        None => "whole".to_string(),
    })
}

#[cfg(test)]
mod tests;
