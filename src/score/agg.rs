//! A whole-tree ranker over aggregates of the per-node quantities: linear, or
//! a boosted pairwise model over the same inputs.
//!
//! Unset, `VITRI_SCORE_AGG` leaves every call site below inert and the
//! portfolio selects on [`super::vtree_cost`].
//!
//! The quantities are the 38 columns [`super::tables`] computes at each
//! internal node. Each column is reduced over ALL internal nodes of the tree by
//! one of five aggregates, standardized, and summed with the eleven addends of
//! the structural cost ([`super::unified_cost_terms`]). Lower is better, and
//! the portfolio takes the argmin within a component.
//!
//! The model is data, not code: a JSON file the caller names, exported by the
//! fit that produced the weights. A file naming a column, an aggregate or a
//! cost term this crate has no definition for is refused at load.
//!
//! Two kinds of file. `agg-linear` scores each candidate on its own: the
//! intercept, plus each cost addend at its weight, plus each standardized
//! aggregate at its weight. `agg-pair-boost` scores candidates against each
//! other: a gradient-boosted ensemble reads the DIFFERENCE of two candidates'
//! raw input vectors and predicts the probability that the first is the larger
//! compile, and a candidate's score is its mean probability of being larger
//! than each sibling. Both kinds: lower is better, argmin within a component.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::cnf::CnfFormula;
use crate::error::VitriError;
use crate::vtree::Vtree;

use super::tables::{FEATURE_NAMES, Feature, Tables};
use super::{COST_TERM_NAMES, unified_cost_terms};

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
            Aggregate::Lse => lse2(values),
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

/// `log2 Σ 2^v` over the strictly positive entries, max-shifted; 0 when none is
/// positive. The convention the offline tables were built with.
fn lse2(values: &[f64]) -> f64 {
    let peak = values.iter().copied().filter(|&v| v > 0.0).reduce(f64::max);
    let Some(peak) = peak else {
        return 0.0;
    };
    peak + values
        .iter()
        .copied()
        .filter(|&v| v > 0.0)
        .map(|v| 2f64.powf(v - peak))
        .sum::<f64>()
        .log2()
}

// ---------------------------------------------------------------------------
// The model file
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct RawModel {
    kind: String,
    #[serde(default)]
    intercept: f64,
    /// Ordered, so a file with more than one bad term is refused for the same
    /// one on every run.
    #[serde(default)]
    terms: BTreeMap<String, f64>,
    #[serde(default)]
    features: Vec<RawFeature>,
    // The boosted kind's fields.
    #[serde(default)]
    baseline: f64,
    #[serde(default)]
    inputs: Vec<RawInput>,
    #[serde(default)]
    trees: Vec<RawTree>,
}

/// One entry of the boosted kind's input vector: a cost addend by name, or a
/// column reduced by an aggregate, raw.
#[derive(serde::Deserialize)]
struct RawInput {
    term: Option<String>,
    column: Option<String>,
    agg: Option<String>,
}

#[derive(serde::Deserialize)]
struct RawTree {
    nodes: Vec<RawNode>,
}

/// A leaf carries `value`; a split carries the other four. The file's
/// `missing_left` is read and ignored: every input here is finite.
#[derive(serde::Deserialize)]
struct RawNode {
    value: Option<f64>,
    feature: Option<usize>,
    threshold: Option<f64>,
    left: Option<usize>,
    right: Option<usize>,
}

#[derive(serde::Deserialize)]
struct RawFeature {
    column: String,
    agg: String,
    mean: f64,
    sd: f64,
    weight: f64,
}

/// The two `kind`s this crate evaluates. A file carrying any other is refused
/// rather than read as one of these.
const LINEAR_KIND: &str = "agg-linear";
const BOOST_KIND: &str = "agg-pair-boost";

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
    /// Read a ranker from the JSON a fitting run exported.
    ///
    /// # Errors
    ///
    /// A sentence naming the file and the field that is wrong: bad JSON, a
    /// `kind` this crate does not evaluate, a cost term or a column or an
    /// aggregate it has no definition for, a standard deviation that is not
    /// above zero, a weight that is not finite.
    pub(crate) fn from_json(source: &Path, text: &str) -> Result<AggModel, String> {
        let raw: RawModel = serde_json::from_str(text)
            .map_err(|e| format!("{}: not an aggregate ranker: {e}", source.display()))?;
        let bad = |what: String| format!("{}: {what}", source.display());
        let boosted = match raw.kind.as_str() {
            LINEAR_KIND => false,
            BOOST_KIND => true,
            other => {
                return Err(bad(format!(
                    "kind {other:?} is not one this crate evaluates; it reads {LINEAR_KIND:?} \
                     and {BOOST_KIND:?}",
                )));
            }
        };
        if boosted && (!raw.features.is_empty() || !raw.terms.is_empty()) {
            return Err(bad(format!(
                "a {BOOST_KIND} file lists its inputs under \"inputs\"; \"features\" and \
                 \"terms\" belong to {LINEAR_KIND}",
            )));
        }
        if !raw.intercept.is_finite() {
            return Err(bad(format!("intercept {} is not finite", raw.intercept)));
        }
        let mut terms = [0f64; 11];
        for (name, weight) in &raw.terms {
            let Some(at) = COST_TERM_NAMES
                .iter()
                .position(|known| *known == name.as_str())
            else {
                return Err(bad(format!(
                    "terms names {name:?}, which is not one of the cost's addends: {}",
                    COST_TERM_NAMES.join(", "),
                )));
            };
            if !weight.is_finite() {
                return Err(bad(format!("terms {name:?} weight {weight} is not finite")));
            }
            terms[at] = *weight;
        }
        // The boosted kind's column inputs enter the same aggregate table, raw:
        // mean 0, sd 1, weight unused.
        let listed: Vec<(String, RawFeature)> = if boosted {
            let mut out = Vec::new();
            for (at, input) in raw.inputs.iter().enumerate() {
                if let (Some(column), Some(agg)) = (&input.column, &input.agg) {
                    if input.term.is_some() {
                        return Err(bad(format!(
                            "inputs[{at}] names both a term and a column; one or the other"
                        )));
                    }
                    out.push((
                        format!("inputs[{at}]"),
                        RawFeature {
                            column: column.clone(),
                            agg: agg.clone(),
                            mean: 0.0,
                            sd: 1.0,
                            weight: 0.0,
                        },
                    ));
                }
            }
            out
        } else {
            raw.features
                .into_iter()
                .enumerate()
                .map(|(at, f)| (format!("features[{at}]"), f))
                .collect()
        };
        let mut columns: Vec<Feature> = Vec::new();
        let mut aggregates = Vec::with_capacity(listed.len());
        for (where_, feature) in &listed {
            let named = |what: &str| {
                bad(format!(
                    "{where_} ({:?} {:?}): {what}",
                    feature.column, feature.agg,
                ))
            };
            let column = Feature::from_name(&feature.column)
                .ok_or_else(|| named("column is not a quantity this crate computes"))?;
            let agg = Aggregate::from_name(&feature.agg).ok_or_else(|| {
                named(&format!(
                    "agg is not known; this crate reduces by {}",
                    AGGREGATE_NAMES
                        .iter()
                        .map(|(name, _)| *name)
                        .collect::<Vec<_>>()
                        .join(", "),
                ))
            })?;
            if !(feature.sd.is_finite() && feature.sd > 0.0) {
                return Err(named(&format!(
                    "sd is {}; a standard deviation has to be above zero",
                    feature.sd
                )));
            }
            if !feature.mean.is_finite() {
                return Err(named(&format!("mean {} is not finite", feature.mean)));
            }
            if !feature.weight.is_finite() {
                return Err(named(&format!("weight {} is not finite", feature.weight)));
            }
            // One gather per distinct column, however many aggregates read it.
            let column_at = match columns.iter().position(|&c| c == column) {
                Some(at) => at,
                None => {
                    columns.push(column);
                    columns.len() - 1
                }
            };
            aggregates.push(AggTerm {
                column: column_at,
                agg,
                mean: feature.mean,
                sd: feature.sd,
                weight: feature.weight,
            });
        }
        let (inputs, scorer) = if boosted {
            (
                Self::inputs_from(&raw.inputs, &bad)?,
                Scorer::PairBoost {
                    baseline: Self::baseline_from(raw.baseline, &bad)?,
                    trees: Self::trees_from(&raw.trees, raw.inputs.len(), &bad)?,
                },
            )
        } else {
            (Vec::new(), Scorer::Linear)
        };
        Ok(AggModel {
            intercept: raw.intercept,
            terms,
            columns,
            aggregates,
            inputs,
            scorer,
        })
    }

    /// The boosted kind's input vector: each entry a cost addend by name or an
    /// aggregate, in file order, indexed the way the trees index them.
    fn inputs_from(raw: &[RawInput], bad: &dyn Fn(String) -> String) -> Result<Vec<Input>, String> {
        if raw.is_empty() {
            return Err(bad(
                "inputs is empty; the trees have nothing to read".to_string()
            ));
        }
        let mut inputs = Vec::with_capacity(raw.len());
        let mut next_aggregate = 0;
        for (at, input) in raw.iter().enumerate() {
            match (&input.term, &input.column, &input.agg) {
                (Some(term), None, None) => {
                    let Some(position) = COST_TERM_NAMES.iter().position(|known| known == term)
                    else {
                        return Err(bad(format!(
                            "inputs[{at}] names term {term:?}, which is not one of the cost's \
                             addends: {}",
                            COST_TERM_NAMES.join(", "),
                        )));
                    };
                    inputs.push(Input::Term(position));
                }
                (None, Some(_), Some(_)) => {
                    inputs.push(Input::Aggregate(next_aggregate));
                    next_aggregate += 1;
                }
                _ => {
                    return Err(bad(format!(
                        "inputs[{at}] has to be a term, or a column with an agg"
                    )));
                }
            }
        }
        Ok(inputs)
    }

    fn baseline_from(baseline: f64, bad: &dyn Fn(String) -> String) -> Result<f64, String> {
        if baseline.is_finite() {
            Ok(baseline)
        } else {
            Err(bad(format!("baseline {baseline} is not finite")))
        }
    }

    /// The node tables, each index checked against its own tree and each split
    /// against the input vector, so evaluation never indexes out of range.
    fn trees_from(
        raw: &[RawTree],
        n_inputs: usize,
        bad: &dyn Fn(String) -> String,
    ) -> Result<Vec<Vec<Node>>, String> {
        if raw.is_empty() {
            return Err(bad(
                "trees is empty; a boosted ranker has at least one".to_string()
            ));
        }
        let mut trees = Vec::with_capacity(raw.len());
        for (t, tree) in raw.iter().enumerate() {
            let n = tree.nodes.len();
            if n == 0 {
                return Err(bad(format!("trees[{t}] has no nodes")));
            }
            let mut nodes = Vec::with_capacity(n);
            for (i, node) in tree.nodes.iter().enumerate() {
                let named = |what: String| bad(format!("trees[{t}].nodes[{i}]: {what}"));
                let parsed = match (
                    node.value,
                    node.feature,
                    node.threshold,
                    node.left,
                    node.right,
                ) {
                    (Some(value), None, None, None, None) => {
                        if !value.is_finite() {
                            return Err(named(format!("value {value} is not finite")));
                        }
                        Node::Leaf(value)
                    }
                    (None, Some(input), Some(threshold), Some(left), Some(right)) => {
                        if input >= n_inputs {
                            return Err(named(format!(
                                "feature {input} is out of range; the file lists {n_inputs} inputs"
                            )));
                        }
                        if !threshold.is_finite() {
                            return Err(named(format!("threshold {threshold} is not finite")));
                        }
                        if left >= n || right >= n {
                            return Err(named(format!(
                                "children {left} and {right} have to index the tree's {n} nodes"
                            )));
                        }
                        if left <= i || right <= i {
                            return Err(named(
                                "children have to come after their parent".to_string(),
                            ));
                        }
                        Node::Split {
                            input,
                            threshold,
                            left,
                            right,
                        }
                    }
                    _ => {
                        return Err(named(
                            "a node is a leaf with a value, or a split with feature, \
                             threshold, left and right"
                                .to_string(),
                        ));
                    }
                };
                nodes.push(parsed);
            }
            trees.push(nodes);
        }
        Ok(trees)
    }

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

/// The name the model file spells `feature` as, for a message about it.
fn feature_name(feature: Feature) -> &'static str {
    FEATURE_NAMES
        .iter()
        .find(|(_, known)| *known == feature)
        .map(|&(name, _)| name)
        .expect("every feature is in the name table")
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

/// What `model` computes for `vtree` against `formula`. The linear kind: the
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
) -> Result<AggScore, VitriError> {
    let (terms, values) = agg_numbers(vtree, formula, model)?;
    if model.is_pairwise() {
        let inputs = model
            .inputs
            .iter()
            .map(|input| match *input {
                Input::Term(at) => terms[at],
                Input::Aggregate(at) => values[at],
            })
            .collect();
        return Ok(AggScore::Inputs(inputs));
    }
    let mut score = model.intercept;
    for (weight, term) in model.terms.iter().zip(&terms) {
        score += weight * term;
    }
    for (entry, value) in model.aggregates.iter().zip(&values) {
        score += entry.weight * ((value - entry.mean) / entry.sd);
    }
    Ok(AggScore::Scalar(score))
}

/// The boosted kind's scores for one component: each candidate's mean predicted
/// probability of being the larger compile against each sibling, from the
/// input vectors [`agg_score`] produced. A lone candidate scores 0.
///
/// The ensemble is evaluated on every ordered pair, `n(n-1)` walks of a few
/// hundred shallow trees, which is nothing beside building one candidate.
pub(crate) fn round_robin(model: &AggModel, inputs: &[&[f64]]) -> Vec<f64> {
    let n = inputs.len();
    if n < 2 {
        return vec![0.0; n];
    }
    let mut scores = vec![0.0; n];
    let mut diff = vec![0.0; model.inputs.len()];
    for i in 0..n {
        for j in 0..n {
            if i == j {
                continue;
            }
            for (d, (a, b)) in diff.iter_mut().zip(inputs[i].iter().zip(inputs[j])) {
                *d = a - b;
            }
            let raw = model.raw_pair(&diff);
            scores[i] += 1.0 / (1.0 + (-raw).exp());
        }
        scores[i] /= (n - 1) as f64;
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
) -> Result<([f64; 11], Vec<f64>), VitriError> {
    super::covered_by(vtree, formula)?;
    let tables = Tables::build(vtree, formula, model.reads_split(), model.reads_cut());
    let terms = unified_cost_terms(
        vtree,
        formula,
        tables.cost_tables(),
        super::stddev_from_counts(tables.clause_at()),
        super::vtree_depth(vtree),
    );

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
                eprintln!(
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
    Ok((terms, values))
}

// ---------------------------------------------------------------------------
// The switch
// ---------------------------------------------------------------------------

/// The variable that names a ranker file. Unset — the default — leaves
/// selection on [`super::vtree_cost`], and nothing in this module runs.
pub(crate) const AGG_VAR: &str = "VITRI_SCORE_AGG";

/// What the variable's value has to be, quoted in the message a bad one gets.
const AGG_EXPECTED: &str = "the path of an exported whole-tree aggregate ranker in JSON";

/// The variables that decide the same pick this one does. Set together with
/// [`AGG_VAR`], each of them is inert — this ranker takes the pick — so the
/// combination is refused by name instead of one of the two quietly winning.
const CONFLICTING: [&str; 1] = ["VITRI_SCORE_WEIGHTS"];

/// The ranker [`AGG_VAR`] names, or `None` when it is unset.
///
/// Each file is read and parsed once per process; a second call for the same
/// path hands back the same ranker.
///
/// # Errors
///
/// [`VitriError::Env`] when the variable is set beside a variable that decides
/// the same pick, or when the file it names cannot be read or is not a ranker
/// this crate can evaluate. A ranker that was asked for and could not be loaded
/// is never quietly dropped.
pub(crate) fn model_from_env() -> Result<Option<Arc<AggModel>>, VitriError> {
    let Some(raw) = crate::env::env_raw(AGG_VAR, AGG_EXPECTED)? else {
        return Ok(None);
    };
    conflict(|name| std::env::var_os(name).is_some())?;
    let path = PathBuf::from(raw.trim());
    match load_cached(&path) {
        Ok(model) => Ok(Some(model)),
        Err(reason) => Err(VitriError::env(
            AGG_VAR,
            format!("must be {AGG_EXPECTED}; {reason}"),
        )),
    }
}

/// Refuse a combination of pick variables before anything reads their values.
///
/// Checked ahead of the other pick knobs' own checks so that setting two of them
/// is reported as that, rather than as whatever is wrong with the second one's
/// value.
///
/// # Errors
///
/// [`VitriError::Env`] naming both variables, when [`AGG_VAR`] is set beside one
/// of [`CONFLICTING`].
pub(crate) fn check_conflicts() -> Result<(), VitriError> {
    if std::env::var_os(AGG_VAR).is_none() {
        return Ok(());
    }
    conflict(|name| std::env::var_os(name).is_some())
}

/// The pure half of [`check_conflicts`]: `set` says which variables the process
/// has, so the accepted combinations can be checked without mutating the
/// environment.
///
/// # Errors
///
/// [`VitriError::Env`] naming both variables when one of [`CONFLICTING`] is set.
fn conflict(set: impl Fn(&str) -> bool) -> Result<(), VitriError> {
    let Some(other) = CONFLICTING.iter().find(|name| set(name)) else {
        return Ok(());
    };
    Err(VitriError::env(
        AGG_VAR,
        format!(
            "cannot be set together with {other}: both decide which candidate the portfolio \
             takes, and this ranker would decide it, leaving {other} doing nothing. Unset one of \
             the two."
        ),
    ))
}

/// The variable that narrows the field the ranker chooses from: only the
/// candidates whose cost is within this much of the cost pick's cost are
/// eligible. Unset, every candidate is.
pub(crate) const MARGIN_VAR: &str = "VITRI_SCORE_AGG_MARGIN";

/// What the margin's value has to be, quoted in the message a bad one gets.
const MARGIN_EXPECTED: &str = "a cost margin in the cost's own units, zero or more";

/// How far above the cost pick's cost a candidate may sit and still be ranked,
/// or `None` when [`MARGIN_VAR`] is unset.
///
/// # Errors
///
/// [`VitriError::Env`] when the margin is set without a ranker to narrow, or to
/// something that is not a margin.
pub(crate) fn margin_from_env() -> Result<Option<f64>, VitriError> {
    let raw = crate::env::env_raw(MARGIN_VAR, MARGIN_EXPECTED)?;
    margin_from_value(raw.as_deref(), std::env::var_os(AGG_VAR).is_some())
}

/// The pure half of [`margin_from_env`].
///
/// # Errors
///
/// [`VitriError::Env`] naming both variables when `raw` is `Some` and
/// `ranker_set` is false, or naming the margin when it does not read as one.
fn margin_from_value(raw: Option<&str>, ranker_set: bool) -> Result<Option<f64>, VitriError> {
    let Some(raw) = raw else { return Ok(None) };
    if !ranker_set {
        return Err(VitriError::env(
            MARGIN_VAR,
            format!(
                "requires {AGG_VAR}: it narrows the field that ranker chooses from, and with no \
                 ranker the cost picks alone. Set {AGG_VAR} to a ranker, or unset {MARGIN_VAR}."
            ),
        ));
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

/// Whether this process was asked for the ranker at all — whether [`AGG_VAR`]
/// is set. It says nothing about the file behind it loading; that is reported
/// where the file is read.
///
/// Read once: it decides whether the component labels below are maintained, and
/// a build without the ranker does not pay for them.
pub(crate) fn requested() -> bool {
    static REQUESTED: OnceLock<bool> = OnceLock::new();
    *REQUESTED.get_or_init(|| std::env::var_os(AGG_VAR).is_some())
}

/// Read and parse `path`, or hand back what an earlier call parsed.
fn load_cached(path: &Path) -> Result<Arc<AggModel>, String> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<AggModel>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(model) = cache.get(path) {
        return Ok(Arc::clone(model));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let model = Arc::new(AggModel::from_json(path, &text)?);
    cache.insert(path.to_path_buf(), Arc::clone(&model));
    Ok(model)
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
    /// records the number here. Maintained only when [`requested`], and reset
    /// on the whole-formula path.
    static COMPONENT: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
}

/// Record which component the build about to run is on; `None` for a build over
/// a whole formula that was never split. A no-op unless the ranker was asked
/// for.
pub(crate) fn set_component(index: Option<usize>) {
    if requested() {
        COMPONENT.with(|slot| slot.set(index));
    }
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
