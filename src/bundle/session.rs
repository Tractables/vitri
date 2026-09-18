//! The frontend session: one anchored budget shared by preprocessing and
//! vtree construction, and the retry path over a kept simplify checkpoint.

use super::*;

/// The single anchored preprocessing result, optionally carrying the owned
/// count-stage checkpoint a frontend session may reuse for a later attempt.
pub(super) struct PreprocessOutcome {
    pub(super) bundle: PreprocessBundle,
    pub(super) count_stage1: Option<count_chain::CountStage1>,
}

/// The wall assigned by a caller to one preprocessing retry.
///
/// Vitri owns what the retry does; the caller owns how much of its larger
/// workflow the retry may spend. The absolute deadline bounds preprocessing,
/// vtree construction, and the caller's later use of the returned run.
/// `arjun_budget` is the exact part of that window Arjun may consume, clamped by
/// `deadline`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryBudget {
    pub(super) deadline: std::time::Instant,
    pub(super) arjun_budget: std::time::Duration,
}

/// The preprocessing and vtree policy for one checkpointed frontend retry.
///
/// Each `None` field inherits the primary run's setting. The two choices are
/// independent: a caller may change Arjun's bounded-variable-addition policy,
/// vtree construction, both, or neither. `vtree_spec` uses the same public
/// language as [`RunConfig::vtree_spec`](crate::config::RunConfig::vtree_spec),
/// so `portfolio` requests scored portfolio selection while a concrete spec
/// requests that construction directly.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrontendRetryConfig {
    /// Override Arjun's bounded-variable-addition policy for this retry.
    pub arjun_sbva: Option<crate::preprocess::ArjunSbva>,
    /// Override the vtree specification for this retry.
    pub vtree_spec: Option<String>,
}

impl RetryBudget {
    /// Create a non-empty retry budget ending at `deadline`.
    ///
    /// # Errors
    ///
    /// [`VitriError::Config`] when `arjun_budget` is zero. An already-expired
    /// deadline is not a malformed request; the session simply declines the
    /// retry when it is attempted.
    pub fn new(
        deadline: std::time::Instant,
        arjun_budget: std::time::Duration,
    ) -> Result<Self, VitriError> {
        if arjun_budget.is_zero() {
            return Err(VitriError::config(
                "a frontend retry needs a non-zero Arjun budget",
            ));
        }
        Ok(Self {
            deadline,
            arjun_budget,
        })
    }

    /// The absolute deadline shared by the returned vtree and its caller's
    /// compile attempt.
    pub fn deadline(self) -> std::time::Instant {
        self.deadline
    }

    /// The exact Arjun allowance inside [`Self::deadline`].
    pub fn arjun_budget(self) -> std::time::Duration {
        self.arjun_budget
    }
}

/// A validated, anchored full-pipeline run that has not prepared its primary
/// attempt yet.
///
/// The session borrows the raw formula and its metadata, and owns the cloned
/// [`RunConfig`], construction context, and raw
/// [`StructureProfile`](crate::score::StructureProfile) that every attempt over
/// that input must share. Its deadline is anchored when the session is created,
/// so time between [`frontend`] and [`Self::prepare`] remains part of the run
/// budget.
///
/// A session prepares one primary attempt. Calling [`Self::prepare`] again is
/// refused explicitly. A caller may then use [`Self::retry`] to apply another
/// Arjun and vtree policy to the exact simplify checkpoint retained by the
/// primary run; retries neither repeat nor clone simplification.
pub struct FrontendSession<'a> {
    pub(super) formula: &'a CnfFormula,
    pub(super) meta: &'a CnfMeta,
    pub(super) config: RunConfig,
    pub(super) selection: crate::decompose::SelectionCtx,
    pub(super) source_profile: crate::score::StructureProfile,
    pub(super) count_stage1: Option<count_chain::CountStage1>,
    pub(super) prepared: bool,
}

impl std::fmt::Debug for FrontendSession<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrontendSession")
            .field("formula", self.formula)
            .field("meta", self.meta)
            .field("config", &self.config)
            .field("selection", &self.selection)
            .field("source_profile", &self.source_profile)
            .field("has_count_stage1", &self.count_stage1.is_some())
            .field("prepared", &self.prepared)
            .finish()
    }
}

impl FrontendSession<'_> {
    pub(super) fn build_run(
        &self,
        preprocessed: PreprocessBundle,
        config: &RunConfig,
    ) -> Result<VitriRun, VitriError> {
        // Both ways preprocessing can settle the instance by itself, before
        // anything is spent on selection or construction. A refutation is
        // checked first because its exported formula is not empty: the
        // contradiction is written over the original variable count, so
        // `num_vars` says nothing about whether the answer is already known.
        if preprocessed.record.unsat {
            return Ok(VitriRun {
                source_profile: self.source_profile,
                preprocessed,
                vtree: RunVtree::Refuted,
            });
        }
        if preprocessed.reduced.num_vars() == 0 {
            return Ok(VitriRun {
                source_profile: self.source_profile,
                preprocessed,
                vtree: RunVtree::FullyResolved,
            });
        }
        let selection = run_selection(
            &self.selection,
            self.source_profile,
            preprocessed.record.show_vars_reduced_dimacs.as_ref(),
            preprocessed.reduced.num_vars(),
        );
        let built =
            crate::component::build_vtree_anchored(&preprocessed.reduced, config, &selection)?;
        Ok(VitriRun {
            source_profile: self.source_profile,
            preprocessed,
            vtree: RunVtree::Built(built),
        })
    }

    /// Apply a preprocessing and vtree policy to the primary run's retained
    /// simplify checkpoint.
    ///
    /// The caller selects the independently optional policy overrides through
    /// [`FrontendRetryConfig`] and may issue more than one retry with distinct
    /// configurations and budgets. Each retry begins from the same immutable
    /// checkpoint; it never preprocesses the raw formula or repeats simplify.
    ///
    /// Returns `Ok(None)` when the primary run did not retain a count-preserving
    /// Arjun checkpoint, the budget has expired, or Arjun kept no reduction.
    ///
    /// # Errors
    ///
    /// [`VitriError::Config`] when called before [`Self::prepare`].
    /// [`VitriError::Spec`] when an overridden vtree spec is invalid. Other
    /// errors come from Arjun or vtree construction.
    pub fn retry(
        &self,
        budget: RetryBudget,
        retry: &FrontendRetryConfig,
    ) -> Result<Option<VitriRun>, VitriError> {
        if !self.prepared {
            return Err(VitriError::config(
                "FrontendSession::retry requires a completed primary prepare",
            ));
        }
        if let Some(vtree_spec) = retry.vtree_spec.as_deref() {
            crate::spec::validate_vtree_spec(vtree_spec)?;
        }
        let Some(stage1) = self.count_stage1.as_ref() else {
            return Ok(None);
        };
        let now = std::time::Instant::now();
        let deadline = self
            .config
            .deadline
            .map_or(budget.deadline, |run_deadline| {
                run_deadline.min(budget.deadline)
            });
        if deadline <= now {
            return Ok(None);
        }

        let mut retry_config = self.config.clone();
        retry_config.deadline = Some(deadline);
        retry_config.arjun_budget = crate::config::ArjunBudget::Exact(budget.arjun_budget);
        if let Some(vtree_spec) = retry.vtree_spec.as_deref() {
            retry_config.vtree_spec = vtree_spec.to_owned();
        }
        if let Some(sbva) = retry.arjun_sbva {
            retry_config.arjun.sbva = sbva;
        }
        let mut preprocessed = count_chain::finish_count_preserving_attempt(stage1, &retry_config)?;
        finish_bundle(&mut preprocessed, &retry_config, now, stage1.elapsed_ms());
        // A retry that discarded its reduction produced the formula the primary
        // attempt already built a vtree over, so there is nothing new to build.
        if preprocessed.stages.arjun.as_ref() != Some(&StageOutcome::Ran) {
            return Ok(None);
        }
        self.build_run(preprocessed, &retry_config).map(Some)
    }

    /// Preprocess the borrowed input and build the vtree over what remains.
    ///
    /// # Errors
    ///
    /// Whatever [`preprocess`] and
    /// [`component::build_vtree`](crate::component::build_vtree) return. A
    /// second call returns [`VitriError::Config`] instead of repeating work.
    pub fn prepare(&mut self) -> Result<VitriRun, VitriError> {
        if self.prepared {
            return Err(VitriError::config(
                "FrontendSession::prepare may be called at most once",
            ));
        }
        // An attempted preparation consumes the session even when a phase
        // fails: silently replaying preprocessing after an error would be a
        // second attempt with no policy saying that is what the caller wanted.
        self.prepared = true;

        let outcome = preprocess_anchored_with_checkpoint(self.formula, self.meta, &self.config)?;
        let preprocessed = outcome.bundle;
        // The checkpoint is worth keeping for one shape of retry: plain `mc`,
        // where Arjun ran and a second attempt can start after simplify.
        if preprocessed.record.mode == Mode::Mc
            && preprocessed.stages.arjun.as_ref() == Some(&StageOutcome::Ran)
        {
            self.count_stage1 = outcome.count_stage1;
        }
        self.build_run(preprocessed, &self.config)
    }
}

/// Create a full-pipeline session over one raw input.
///
/// Configuration is validated, the raw structural profile is measured, and a
/// relative [`RunConfig::budget_ms`] is anchored to an absolute deadline before
/// this function returns. [`FrontendSession::prepare`] therefore spends the
/// budget that remains from session creation rather than starting a new one.
///
/// # Errors
///
/// [`VitriError::Config`] for an invalid configuration.
pub fn frontend<'a>(
    formula: &'a CnfFormula,
    meta: &'a CnfMeta,
    config: &RunConfig,
    selection: &crate::decompose::SelectionCtx,
) -> Result<FrontendSession<'a>, VitriError> {
    frontend_at(formula, meta, config, selection, std::time::Instant::now())
}

/// The deterministic clock seam beneath [`frontend`].
pub(super) fn frontend_at<'a>(
    formula: &'a CnfFormula,
    meta: &'a CnfMeta,
    config: &RunConfig,
    selection: &crate::decompose::SelectionCtx,
    now: std::time::Instant,
) -> Result<FrontendSession<'a>, VitriError> {
    config.validate()?;
    selection.goatd.validate()?;
    Ok(FrontendSession {
        formula,
        meta,
        config: config.anchored(now),
        selection: selection.clone(),
        source_profile: crate::score::StructureProfile::measure(formula),
        count_stage1: None,
        prepared: false,
    })
}

/// Preprocess `formula` and build the vtree over what preprocessing left — the
/// whole pipeline, in the one order it runs in.
///
/// [`preprocess`] then [`component::build_vtree`](crate::component::build_vtree),
/// with the step between them that a caller would otherwise have to know about:
/// the vtree is built over the REDUCED formula, and selection is made show-aware
/// from the record's show set, which is already in that formula's space.
///
/// `selection` carries the caller's construction knobs; its objective is filled
/// in from the instance, since only the record can say what the reduced show set
/// is. Its `source_profile` field is ignored: this full-pipeline entry measures
/// the raw input exactly once, reports that measurement on [`VitriRun`], and
/// unconditionally supplies the same value to vtree selection.
///
/// The budget is anchored once by [`frontend`], and both halves stop at that
/// instant: what preprocessing spends, construction does not get. `run` creates
/// the session and immediately prepares it.
///
/// # Errors
///
/// Whatever [`preprocess`] and
/// [`component::build_vtree`](crate::component::build_vtree) return.
pub fn run(
    formula: &CnfFormula,
    meta: &CnfMeta,
    config: &RunConfig,
    selection: &crate::decompose::SelectionCtx,
) -> Result<VitriRun, VitriError> {
    frontend(formula, meta, config, selection)?.prepare()
}

/// Resolve the construction context owned by a full [`run`]. Kept as one
/// production path so the internal regression test can prove the value handed
/// to selection is the value the run reports.
pub(super) fn run_selection(
    selection: &crate::decompose::SelectionCtx,
    source_profile: crate::score::StructureProfile,
    show: Option<&crate::cnf::ShowSet<crate::cnf::Reduced>>,
    num_vars: u32,
) -> crate::decompose::SelectionCtx {
    let mut selection = selection.clone().with_show(show, num_vars);
    selection.source_profile = Some(source_profile);
    selection
}
