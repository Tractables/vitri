//! Per-component export: the independent sub-problems of `reduced.cnf`, each
//! with its own CNF and its own vtree.
//!
//! # Why this file exists
//!
//! The reduced formula splits into independent components, each compiled on
//! its own smaller vtree. The grafted whole-formula vtree carries no record of
//! where the split was — this manifest is that record.
//!
//! # Variable numbering — read this before using any file here
//!
//! Three spaces are in play, and mixing them silently produces a wrong count:
//!
//! 1. **ORIGINAL** — the input CNF's own 1-based DIMACS ids. Nothing in this
//!    module is in that space; compose with `preprocess.json`'s
//!    `reduced_to_original_dimacs` to get there.
//! 2. **REDUCED** — 1-based DIMACS ids of `reduced.cnf`, the formula the split
//!    was computed on. [`ComponentsManifest::free_vars_reduced_dimacs`] and the
//!    values of [`ComponentEntry::local_to_reduced_dimacs`] are in this space.
//! 3. **LOCAL** — each component is renumbered to a dense `1..=num_vars` space of
//!    its own. `components/compNNN.cnf` is written in it, and that component's
//!    vtree is built over it. [`ComponentEntry::local_to_reduced_dimacs`] is the
//!    correspondence: `local_to_reduced_dimacs[i - 1]` is the REDUCED id of
//!    LOCAL variable `i` (1-based on both sides), strictly increasing, so LOCAL
//!    order is REDUCED order — but **the ids are not equal**, and assuming they
//!    are is the failure this file exists to prevent.
//!
//! A component's own vtree file follows the same rule as the whole-formula
//! one: `compNNN.vtree` (standard SDD text) is LOCAL 1-based, numbering the
//! same variables as `compNNN.cnf`.
//!
//! # How the counts compose
//!
//! The components are variable-disjoint and clause-disjoint by construction, so
//! for a plain model count:
//!
//! ```text
//! count(reduced) = 2^|free_vars_reduced_dimacs| * Π_c count(compNNN.cnf)
//! ```
//!
//! and `count(original) = count(reduced) * 2^count_lift_pow2` from
//! `preprocess.json` closes the loop. A free variable occurs in no clause of
//! `reduced.cnf` — it belongs to no component, which is why the factor exists.
//!
//! For a projected count the free-variable factor is over the free variables
//! that are also show variables — a projected-out free variable contributes ×1,
//! not ×2:
//!
//! ```text
//! count_proj(reduced) = 2^|free_vars_reduced_dimacs ∩ show| * Π_c count_proj(compNNN.cnf)
//! ```
//!
//! where each component's own show set is [`ComponentEntry::show_vars_local_dimacs`]
//! (also written as a `c p show` line inside `compNNN.cnf`).

use std::path::PathBuf;

use crate::candidates::CandidateRankMetric;
use crate::cnf::{Local, ShowSet};
use crate::vtree::Vtree;
use serde::{Deserialize, Serialize};

use crate::score::VtreeScores;

mod write;
pub use write::write_components;

/// Manifest file name inside an output bundle directory.
pub const COMPONENTS_JSON_NAME: &str = "components.json";
/// Sub-directory holding the per-component CNF and vtree files.
pub const COMPONENTS_DIR: &str = "components";
/// Sub-directory holding the runner-up vtrees of each component's candidate
/// set — rank 0 is the selected vtree, and its entry points back at the
/// component's own vtree files rather than a byte-identical copy.
pub const CANDIDATES_DIR: &str = "candidates";

/// Format tag written into every [`ComponentsManifest`]; bump when a field is
/// added, removed, or changes meaning, and a consumer should refuse a tag it
/// does not know.
pub const COMPONENTS_FORMAT_TAG: &str = "vitri-components-v1";

/// One entry of a component's ranked candidate set: a vtree the portfolio
/// built and scored on its way to picking a winner.
///
/// Every candidate is a complete, usable vtree over the component's LOCAL
/// space, the same contract as the component's own vtree file — offered
/// because "best" is measured by this crate's own cost model
/// ([`scores`](Self::scores)); a consumer with a different one may prefer
/// another candidate.
///
/// The array's order is the rank: entry 0 is always the selected vtree, the
/// one the portfolio chose and the one the component's own `vtree` contains.
/// The rest are ordered by [`ComponentsManifest::candidate_rank_metric`], best
/// first.
///
/// Entry 0 is pinned rather than sorted into place because selection is not a
/// plain argmin over that metric — several candidates carry adoption rules that
/// also weigh peak width or a cost proxy, so the winner is occasionally not
/// the metric's own minimum.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandidateEntry {
    /// Every catalog spec that produced this exact vtree, in catalog order,
    /// each carrying the parameter it was built at (`hypergraph-bisect:0.40`).
    /// More than one name means those specs converged on a structurally
    /// identical tree, emitted once rather than as near-duplicate files — so
    /// the candidate set's length counts distinct *vtrees*, not specs tried.
    pub built_by: Vec<String>,
    /// SDD text format, LOCAL 1-based. Entry 0 repeats the component's own
    /// `vtree` path; runners-up are files under [`CANDIDATES_DIR`].
    pub vtree: String,
    /// The five structural scores this candidate was ranked on. **All five are
    /// lower-is-better**, computed on the realized vtree against this
    /// component's own CNF, not estimated from the tree decomposition it came
    /// from. See [`VtreeScores`] for what each measures.
    pub scores: VtreeScores,
}

/// Summary of the tree decomposition a component's vtree was converted from.
///
/// A summary, not the decomposition: the conversion's per-variable bag
/// assignment is proportional to the component and has no consumer outside the
/// build that produced it, so only these two numbers are published. Both are
/// measured on the graph projection the winning construction ran on (primal or
/// incidence), which is why a component can carry a larger `max_bag_size` than
/// it has variables.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TreeDecompositionSummary {
    /// Bags in the decomposition.
    pub num_bags: u32,
    /// Vertices in the largest bag — the decomposition's width plus one.
    pub max_bag_size: u32,
}

/// Which construction produced a component's vtree, and what it knew about the
/// decomposition behind it.
///
/// The vtree file says where every variable ended up but not what put it
/// there, and a candidate set is retained only on request — so for most
/// bundles this is the only record of the choice.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionEntry {
    /// The `--vtree` spec that rebuilds `vtree`. Under a `portfolio` spec this
    /// is the candidate that WON, not `portfolio` — which one won is the thing
    /// the vtree cannot say — spelled with the parameter it was built at, so
    /// asking for it back returns the same tree. A component small enough to
    /// skip the portfolio reports `minfill`.
    pub winning_spec: String,
    /// The decomposition `winning_spec` converted. Absent for a construction
    /// that decomposes nothing (`force`, `hypergraph-bisect`, the simple
    /// vtrees), and for one that recombined several decompositions, where no
    /// single bag assignment describes the result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_decomposition: Option<TreeDecompositionSummary>,
}

/// One independent sub-problem of `reduced.cnf`.
///
/// **All ids here are 1-based DIMACS** — see the module docs for which space
/// each field lives in.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComponentEntry {
    /// LOCAL → REDUCED: `local_to_reduced_dimacs[i - 1]` is the REDUCED
    /// 1-based id of LOCAL variable `i`. One entry per variable of this
    /// component's file, strictly increasing, and injective across components
    /// (no reduced variable in two components).
    ///
    /// The only way back out of the local space — a count over `compNNN.cnf`
    /// needs no translation, but a model does.
    pub local_to_reduced_dimacs: Vec<u32>,

    /// This component's share of the projection show set; `null` when the
    /// instance declares no projection. Also written as a `c p show` line inside
    /// `compNNN.cnf` — read it from either.
    #[serde(with = "crate::cnf::show_set::dimacs")]
    pub show_vars_local_dimacs: Option<ShowSet<Local>>,

    /// Paths, relative to the bundle directory, of this component's two files.
    ///
    /// For a single-component formula these point at the top-level
    /// `reduced.cnf` / `vtree.vtree` instead of copies — the component *is*
    /// the whole reduced formula, and duplicating the bytes would let the two
    /// drift. The manifest is emitted either way, so a consumer has one code
    /// path.
    pub cnf: String,
    /// SDD text format, LOCAL 1-based. See `cnf`.
    pub vtree: String,

    /// What selection reported about `vtree`. Every construction this crate
    /// runs reports, so the omitted case is a build the caller assembled by
    /// hand — [`crate::component::VtreeBuild`]'s fields are public — and handed
    /// to [`write_components`] with no selection to report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<SelectionEntry>,

    /// This component's ranked candidate set — the vtrees the portfolio built
    /// and scored before picking `vtree`. Omitted from the JSON when empty
    /// (the default): retained only when the caller asks for one
    /// (`--candidates N`, [`crate::config::RunConfig::candidates`]).
    ///
    /// Also empty for a component the portfolio never ran on — one small
    /// enough to be built directly has exactly one candidate: `vtree`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vtree_candidates: Vec<CandidateEntry>,
}

/// The component split of `reduced.cnf`, written as `components.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComponentsManifest {
    /// Format tag; see [`COMPONENTS_FORMAT_TAG`].
    pub format: String,
    /// Variables of `reduced.cnf` that occur in no clause, as REDUCED 1-based
    /// ids. They belong to no component and carry the `2^k` factor in the
    /// composition rule (see the module docs); the grafted `vtree.vtree` still
    /// has a leaf for each of them.
    ///
    /// Named for its space: `preprocess.json`'s own `free_vars_original_dimacs`
    /// is in the ORIGINAL space and lists different variables.
    pub free_vars_reduced_dimacs: Vec<u32>,
    /// Which of [`VtreeScores`]' fields the candidate sets' entries after the
    /// first are ordered by, ascending (lower is better). Absent when no
    /// candidate set was emitted. The token is the score key itself, so sorting
    /// the entries on it reproduces the order they're already in.
    ///
    /// A property of the whole manifest, not of each candidate set, because
    /// it's decided by the counting mode, not the formula: a plain count ranks
    /// on `clause_load_stddev` (how evenly clauses spread over the vtree), a
    /// projected count on `peak_context_width_show` — the widest cut the
    /// compile has to carry over the show variables — or on
    /// `peak_context_width_all` when the instance declares no show set. Every
    /// candidate set in one manifest is ranked the same way.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "rank_metric")]
    pub candidate_rank_metric: Option<CandidateRankMetric>,
    /// The components, in emission order. Never empty.
    pub components: Vec<ComponentEntry>,
}

/// `#[serde(with = ...)]` for [`ComponentsManifest::candidate_rank_metric`]:
/// the metric is written as the [`VtreeScores`] field name
/// [`CandidateRankMetric::as_str`] gives it, `null` for absent.
mod rank_metric {
    use crate::candidates::CandidateRankMetric;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// Write the metric as its token.
    pub(super) fn serialize<S: Serializer>(
        metric: &Option<CandidateRankMetric>,
        ser: S,
    ) -> Result<S::Ok, S::Error> {
        metric.map(CandidateRankMetric::as_str).serialize(ser)
    }

    /// Read it back through [`CandidateRankMetric::parse`].
    ///
    /// # Errors
    ///
    /// The format's own error, naming the token, when it is not one this
    /// version of the crate ranks by.
    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        de: D,
    ) -> Result<Option<CandidateRankMetric>, D::Error> {
        match Option::<String>::deserialize(de)? {
            Some(token) => CandidateRankMetric::parse(&token)
                .map(Some)
                .ok_or_else(|| D::Error::custom(format!("unknown rank metric {token:?}"))),
            None => Ok(None),
        }
    }
}

/// Paths written by [`write_components`].
#[derive(Debug)]
pub struct ComponentPaths {
    /// The manifest itself.
    pub manifest: PathBuf,
    /// The per-component files, in component order — and, under
    /// [`ComponentWriteOptions::dot`], each vtree's `.dot` beside it. Empty for
    /// a single-component formula, whose entry points at the top-level files
    /// instead.
    pub files: Vec<PathBuf>,
    /// The runner-up vtree files of the emitted candidate sets, if any, plus
    /// their `.dot` siblings under [`ComponentWriteOptions::dot`]. Kept apart
    /// from `files` because they're an optional extra — a caller reporting or
    /// cleaning up "the component split" shouldn't have to re-classify paths by
    /// directory name to tell the two apart.
    pub candidates: Vec<PathBuf>,
}

/// What [`write_components`] emits alongside the files it always writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ComponentWriteOptions {
    /// Write a Graphviz `.dot` beside every `.vtree` this call writes — same
    /// stem, annotated by [`dot::annotate_from_cnf`](crate::dot::annotate_from_cnf) against the CNF that vtree
    /// serves, which for a component vtree is that component's own CNF in its
    /// LOCAL numbering. The paths land in [`ComponentPaths`] next to the
    /// `.vtree` files they picture.
    pub dot: bool,
}

/// Whether a manifest and the grafted whole-formula vtree describe the same
/// variable space: the components' reduced ids together with the free ones are
/// exactly `1..=whole.num_leaves()`, each named once.
///
/// The split PARTITIONS the reduced space, so naming one id twice and another
/// not at all is the failure to catch — a component mapped through the wrong
/// offset does exactly that, and leaves the totals matching. Asserted by
/// [`VitriRun::write_to_dir`](crate::VitriRun::write_to_dir) on the manifest it
/// just wrote against the vtree beside it.
pub(super) fn manifest_matches_vtree(manifest: &ComponentsManifest, whole: &Vtree) -> bool {
    let mut named = vec![false; whole.num_leaves() as usize];
    let ids = manifest
        .components
        .iter()
        .flat_map(|c| c.local_to_reduced_dimacs.iter())
        .chain(manifest.free_vars_reduced_dimacs.iter());
    for &id in ids {
        let Some(slot) = (id as usize).checked_sub(1).and_then(|i| named.get_mut(i)) else {
            return false;
        };
        if std::mem::replace(slot, true) {
            return false;
        }
    }
    named.iter().all(|&n| n)
}

#[cfg(test)]
mod tests;
