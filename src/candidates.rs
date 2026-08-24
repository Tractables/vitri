//! The ranked candidate set: the vtrees the portfolio built and scored on the
//! way to picking one, kept instead of dropped.
//!
//! # Why this exists
//!
//! `portfolio` builds several vtrees for the same formula, scores each with the
//! purely structural metrics in [`VtreeScores`], and returns the best one by
//! this crate's own cost model. The losers are already built and already scored, so
//! surfacing them is nearly free — and a consumer whose compiler has a different
//! memory model, a different apply strategy, or different hardware may well
//! prefer a different member of the candidate set than our selector picks.
//!
//! This module is the *retention and ranking* half of that. It does not select:
//! the winner appears here only as the candidate set's rank-0 entry
//! ([`VtreeCandidate::selected`]).
//!
//! # Cost
//!
//! Retention is OFF unless [`RunConfig::candidates`](crate::config::RunConfig::candidates)
//! is set above 1. With it off, no candidate is cloned, no vtree is kept alive
//! past its scoring, no ranking runs, and nothing is written — a build is
//! byte-identical to one from a crate with no candidate set at all.

use std::sync::Arc;

use crate::vtree::Vtree;

use crate::score::VtreeScores;

/// Ceiling on how many candidates may be retained.
///
/// Eight is deliberately just above the production catalog (five entries), so
/// asking for the maximum retains everything the portfolio can produce. It is a
/// MEMORY bound, not a taste one: each retained candidate holds a live vtree over the formula
/// being built (≈ `2 × num_vars` nodes), so on a large component the retained
/// set is the peak-memory term this cap exists to keep predictable.
///
/// Exceeding it is an error naming the cap, never a silent truncation.
pub const MAX_CANDIDATES: usize = 8;

/// Whether a run keeping `keep` vtrees retains a candidate SET.
///
/// Selection always produces a winner; the set is the extra output beside it,
/// so one vtree is not a set and anything above one is. Every site that asks
/// whether this run has a set — the validator refusing an inert request, the
/// retention itself, the summary line — asks here, so the threshold is a single
/// fact rather than a comparison repeated in four places, and the floor sits
/// beside the ceiling [`MAX_CANDIDATES`] it is bounded by.
pub const fn retains_set(keep: usize) -> bool {
    keep > 1
}

/// Which score the candidate set's ranks 1.. are ORDERED by — always the same
/// metric the selector minimized on this run, so the candidate set's order is
/// the selector's own preference order rather than a second opinion invented for
/// the export.
///
/// Every variant is LOWER-IS-BETTER, like every field of [`VtreeScores`], and
/// every variant NAMES one of those fields: the token in the manifest is the
/// score key a consumer would sort on themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateRankMetric {
    /// Plain model counting: [`VtreeScores::clause_load_stddev`], the standard
    /// deviation of per-node clause load. Balance of clause placement across the
    /// tree.
    ClauseLoadStddev,
    /// Projected counting with a show mask: [`VtreeScores::peak_context_width_show`],
    /// context width counted over show variables only — the part of the frontier
    /// a projected compile keeps paying for, hidden variables being ∃-forgotten.
    PeakContextWidthShow,
    /// Projected counting without a show mask, where the show peak is not
    /// defined: [`VtreeScores::peak_context_width_all`], the same peak over every
    /// variable.
    PeakContextWidthAll,
}

impl CandidateRankMetric {
    /// Parses a manifest token: `"clause_load_stddev"`,
    /// `"peak_context_width_show"`, `"peak_context_width_all"`. `None` for
    /// anything else. The exact inverse of [`CandidateRankMetric::as_str`], so a
    /// consumer reading `candidate_rank_metric` back out of the emitted manifest
    /// recovers the metric rather than re-deriving it from the token.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "clause_load_stddev" => Some(CandidateRankMetric::ClauseLoadStddev),
            "peak_context_width_show" => Some(CandidateRankMetric::PeakContextWidthShow),
            "peak_context_width_all" => Some(CandidateRankMetric::PeakContextWidthAll),
            _ => None,
        }
    }
    /// Stable token for the emitted manifest — the name of the
    /// [`VtreeScores`] field this metric reads, and the exact inverse of
    /// [`CandidateRankMetric::parse`].
    pub fn as_str(self) -> &'static str {
        match self {
            CandidateRankMetric::ClauseLoadStddev => "clause_load_stddev",
            CandidateRankMetric::PeakContextWidthShow => "peak_context_width_show",
            CandidateRankMetric::PeakContextWidthAll => "peak_context_width_all",
        }
    }

    /// This metric's value for one candidate's scores.
    ///
    /// Crate-visible because a selection that defers its pick reads the same
    /// number the exported candidate set is ordered by, rather than spelling
    /// the metric out a second time.
    ///
    /// [`PeakContextWidthShow`](CandidateRankMetric::PeakContextWidthShow) reads
    /// the all-variable peak when there is no show score. The two are different
    /// quantities and the wider one always loses, so a sort that mixed them
    /// would rank on the mask rather than on the vtree — but
    /// [`VtreeScores::peak_context_width_show`] is present for a whole run or
    /// for none of it, which makes the fallback uniform across any one set.
    pub(crate) fn value(self, s: &VtreeScores) -> f64 {
        match self {
            CandidateRankMetric::ClauseLoadStddev => s.clause_load_stddev,
            CandidateRankMetric::PeakContextWidthShow => {
                s.peak_context_width_show
                    .unwrap_or(s.peak_context_width_all) as f64
            }
            CandidateRankMetric::PeakContextWidthAll => s.peak_context_width_all as f64,
        }
    }
}

/// One retained vtree, with the scores it was ranked on and the construction(s)
/// that produced it.
#[derive(Clone)]
pub struct VtreeCandidate {
    /// Every portfolio construction that produced *this exact vtree*, in catalog
    /// order, each spelled as the `--vtree` spec that rebuilds it — parameter
    /// included, so `hypergraph-bisect:imbalance=0.40` rather than the bare family.
    ///
    /// Normally one name. TWO OR MORE means those constructions converged on a
    /// structurally identical tree — they are deduplicated into this single
    /// entry rather than emitted as near-duplicate files, so a consumer counting
    /// entries is counting genuinely distinct vtrees.
    pub built_by: Vec<String>,
    /// The vtree itself. Leaves are in the variable space of the formula this
    /// candidate set was built for — for a per-component candidate set, that component's LOCAL
    /// space.
    pub vtree: Arc<Vtree>,
    /// The five structural scores, computed against that same formula. All five
    /// are lower-is-better; see [`VtreeScores`] for what each estimates.
    pub scores: VtreeScores,
    /// True for the candidate the selector actually chose — the vtree this tool
    /// emits as *the* vtree. Exactly one entry per candidate set carries it, and it is
    /// always rank 0.
    pub selected: bool,
}

/// A whole retained candidate set: the metric it is ordered by, and the candidates.
///
/// Rank is position: `candidates[0]` is always the SELECTED vtree, and
/// `candidates[1..]` are the remaining distinct candidates in ascending
/// [`metric`](Self::metric) order. Rank 0 is pinned to the selection rather than
/// to the metric because the plain-MC selector's adoption rules are not a pure
/// argmin (some candidates must also not regress cost or peak width to be
/// adopted), so the winner is not always the metric's minimum — and a candidate set whose
/// first entry were not the emitted vtree would be a trap.
#[derive(Clone, Debug)]
pub struct CandidateSet {
    /// The score `candidates[1..]` are sorted ascending on.
    pub metric: CandidateRankMetric,
    /// Distinct candidates, selected first. Empty when no candidate set was requested.
    pub candidates: Vec<VtreeCandidate>,
}

impl Default for CandidateSet {
    /// The empty candidate set — what every call that did not ask for one produces.
    fn default() -> Self {
        CandidateSet {
            metric: CandidateRankMetric::ClauseLoadStddev,
            candidates: Vec::new(),
        }
    }
}

impl CandidateSet {
    /// True when nothing was retained — the ordinary case, since only a portfolio
    /// spec asked for more than the winner.
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }
}

// Hand-written rather than derived: the candidate's vtree would dump a whole
// tree into every `CandidateSet` printout, so the summary leaves it out.
impl std::fmt::Debug for VtreeCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VtreeCandidate")
            .field("built_by", &self.built_by)
            .field("selected", &self.selected)
            .field("scores", &self.scores)
            .finish_non_exhaustive()
    }
}

/// One built vtree offered for retention, before the deduplication that turns
/// it into a [`VtreeCandidate`]: the construction that produced it, the tree,
/// and the scores the run already computed for it.
pub(crate) struct ScoredVtree {
    /// The `--vtree` spec that rebuilds this tree, parameter included — one
    /// construction, so one name, where the retained entry may end up naming
    /// several.
    pub built_by: String,
    /// The tree itself.
    pub vtree: Arc<Vtree>,
    /// Its scores against the formula this run is building for.
    pub scores: VtreeScores,
}

/// Deduplicate, rank and truncate a scored candidate set into a [`CandidateSet`].
///
/// `selected` must be structurally present in `scored` — matched by *structure*,
/// not by pointer, so a winner that deduplicates into an earlier identical
/// candidate is still found. `keep <= 1` yields an empty candidate set
/// (retention off).
///
/// **Deduplication** is by the serialized vtree text: two candidates
/// that produced the same tree collapse into one entry whose
/// [`built_by`](VtreeCandidate::built_by) names both. Their scores are equal by
/// construction (every score is a pure function of `(vtree, formula)`), so no
/// information is lost by keeping the first.
///
/// The text is a sound identity here — rather than [`Vtree::same_tree`], which
/// is the identity in general — because every candidate arrives freshly built
/// from this run's own portfolio and none has been rotated, so each is numbered
/// by one bottom-up pass over its own shape and equal text is equal tree. It
/// also keys the linear scan directly, where the tree comparison would walk a
/// pair of trees per probe.
pub(crate) fn from_scored(
    scored: Vec<ScoredVtree>,
    selected: &Arc<Vtree>,
    metric: CandidateRankMetric,
    keep: usize,
) -> CandidateSet {
    if !retains_set(keep) || scored.is_empty() {
        return CandidateSet {
            metric,
            candidates: Vec::new(),
        };
    }

    // Sorted below on one metric, whose show-width reading falls back to the
    // all-variable peak — see `CandidateRankMetric::value` for why one set
    // cannot mix the two.
    debug_assert!(
        scored
            .windows(2)
            .all(|w| w[0].scores.peak_context_width_show.is_some()
                == w[1].scores.peak_context_width_show.is_some()),
        "a candidate set mixes projected and non-projected scores",
    );

    // Preserve first-seen (catalog) order so the candidate set is deterministic
    // for a deterministic build.
    let winner_key = selected.to_vtree_text();
    let mut keys: Vec<String> = Vec::with_capacity(scored.len());
    let mut out: Vec<VtreeCandidate> = Vec::with_capacity(scored.len());
    for ScoredVtree {
        built_by,
        vtree,
        scores,
    } in scored
    {
        let key = vtree.to_vtree_text();
        if let Some(pos) = keys.iter().position(|k| *k == key) {
            out[pos].built_by.push(built_by);
            continue;
        }
        let selected = key == winner_key;
        keys.push(key);
        out.push(VtreeCandidate {
            built_by: vec![built_by],
            vtree,
            scores,
            selected,
        });
    }

    out.sort_by(|a, b| {
        b.selected
            .cmp(&a.selected)
            .then_with(|| cmp_f64(metric.value(&a.scores), metric.value(&b.scores)))
            .then_with(|| cmp_f64(a.scores.clause_load_stddev, b.scores.clause_load_stddev))
            .then_with(|| cmp_f64(a.scores.cost, b.scores.cost))
            .then_with(|| a.built_by[0].cmp(&b.built_by[0]))
    });
    out.truncate(keep);
    CandidateSet {
        metric,
        candidates: out,
    }
}

/// Total order on the score f64s. They are finite by construction (`stddev`
/// returns `0.0` on both degenerate branches, and every width is an integer), so
/// the `Equal` fallback is unreachable rather than a silent mis-sort — it exists
/// only because `f64` has no `Ord`.
fn cmp_f64(a: f64, b: f64) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}
