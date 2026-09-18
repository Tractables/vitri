//! Reading a ranker: the file format, and the one parse per process per file.
//!
//! A ranker is data, and this is where it stops being data: the JSON a fitting
//! run exported is checked column by column against what [`super`] can
//! evaluate, and a file naming a quantity this crate has no definition for is
//! refused here rather than scored against a zero. What comes out is an
//! [`AggModel`] the ranking code evaluates without looking at a file again.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::error::VitriError;

use super::super::tables::{FEATURE_NAMES, Feature};
use super::super::{COST_TERM_NAMES, Ranker};
use super::{AGGREGATE_NAMES, AggModel, AggTerm, Aggregate, Input, Node, Scorer};

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

/// The name the model file spells `feature` as, for a message about it.
pub(super) fn feature_name(feature: Feature) -> &'static str {
    FEATURE_NAMES
        .iter()
        .find(|(_, known)| *known == feature)
        .map(|&(name, _)| name)
        .expect("every feature is in the name table")
}

/// The variable that chooses the ranker. Unset — the default — the portfolio
/// selects on [`DEFAULT_MODEL`]; [`COST_ONLY`] selects on [`super::vtree_cost`]
/// alone, and nothing else in this module runs; a path names another file.
pub(crate) const AGG_VAR: &str = "VITRI_SCORE_AGG";

/// The value of [`AGG_VAR`] that turns the ranker off.
pub(crate) const COST_ONLY: &str = "cost";

/// What the variable's value has to be, quoted in the message a bad one gets.
const AGG_EXPECTED: &str = "`cost`, or the path of an exported whole-tree aggregate ranker in JSON";

/// The ranker the portfolio selects on when [`AGG_VAR`] is unset: a pairwise
/// boosted model over the eleven cost addends and the five aggregates of every
/// column, fitted on the portfolio's own candidates over
/// the model-counting competition benchmarks, each pair labelled by which of
/// the two compiled to the larger diagram.
pub(super) const DEFAULT_MODEL: &str = include_str!("pair_boost.json");

/// What [`AGG_VAR`] names, as the choice a build carries.
///
/// The file it names is parsed here rather than when a build first needs it, so
/// a ranker this crate cannot evaluate is refused where the variable is read.
/// The parse is cached per process, so the build pays nothing for the check.
///
/// # Errors
///
/// [`VitriError::Env`] when the file the variable names cannot be read or is
/// not a ranker this crate can evaluate.
pub(crate) fn ranker_from_env() -> Result<Ranker, VitriError> {
    let Some(raw) = crate::env::env_raw(AGG_VAR, AGG_EXPECTED)? else {
        return Ok(Ranker::Shipped);
    };
    if crate::env::is_form(&raw, COST_ONLY) {
        return Ok(Ranker::Off);
    }
    let path = PathBuf::from(raw.trim());
    load_cached(&path)
        .map_err(|reason| VitriError::env(AGG_VAR, format!("must be {AGG_EXPECTED}; {reason}")))?;
    Ok(Ranker::File(path))
}

/// The ranker a build selects on, parsed once per process per file.
///
/// # Errors
///
/// [`VitriError::Config`] when [`Ranker::File`] names a file this crate cannot
/// read or evaluate. A ranker that was asked for and could not be loaded is
/// never quietly dropped.
pub(crate) fn load(ranker: &Ranker) -> Result<Option<Arc<AggModel>>, VitriError> {
    match ranker {
        Ranker::Off => Ok(None),
        Ranker::Shipped => Ok(Some(default_model())),
        Ranker::File(path) => load_cached(path).map(Some).map_err(|reason| {
            VitriError::config(format!(
                "the aggregate ranker at {}: {reason}",
                path.display()
            ))
        }),
    }
}

/// [`DEFAULT_MODEL`], parsed once per process.
fn default_model() -> Arc<AggModel> {
    static SHIPPED: OnceLock<Arc<AggModel>> = OnceLock::new();
    Arc::clone(SHIPPED.get_or_init(|| {
        Arc::new(
            AggModel::from_json(Path::new("pair_boost.json"), DEFAULT_MODEL)
                .expect("the shipped ranker is a file this crate evaluates"),
        )
    }))
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

/// Reading a ranker file into the model that evaluates it.
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
}
