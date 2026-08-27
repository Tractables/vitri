//! CNF graph-view adapter for goatd's FlowCutter decomposer.

use std::time::Duration;

use super::td_to_vtree::{ConversionRequest, convert_td};
use super::{GraphKind, TdConversion, TreeDecomposition};
use crate::cnf::CnfFormula;

/// Whether an elapsed-time cap changes FlowCutter's search gates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WallCapMode {
    /// Stop at the cap without shortening preprocessing.
    BoundOnly,
    /// Adapt preprocessing to a short search window.
    Tight,
}

/// Limits for one FlowCutter decomposition.
#[derive(Clone, Copy)]
pub(crate) enum FcBudget {
    /// Stop on elapsed time.
    Timed {
        timeout_ms: i64,
        patience_ms: i64,
        iters: i32,
        steps: i64,
        cap_mode: WallCapMode,
    },
    /// Stop after a fixed amount of search work.
    Steps { steps: i64, iters: i32 },
}

const FC_TIMED_STEPS: i64 = 1_000_000;
pub(crate) const FC_BARE_TIMEOUT_MS: i64 = 200;
pub(crate) const FC_PATIENCE_MS_BARE: i64 = 100;
pub(crate) const FC_PATIENCE_MS_PARAMETRIZED: i64 = 150;
pub(crate) const FC_DEFAULT_ITERS: i32 = 100_000;
pub(crate) const FC_DEFAULT_STEPS_ITERS: i32 = 900;

impl FcBudget {
    pub(crate) const fn timed(timeout_ms: i64, patience_ms: i64, iters: i32) -> Self {
        if timeout_ms > 0 {
            Self::Timed {
                timeout_ms,
                patience_ms,
                iters,
                steps: FC_TIMED_STEPS,
                cap_mode: WallCapMode::Tight,
            }
        } else {
            Self::Steps {
                steps: FC_TIMED_STEPS,
                iters,
            }
        }
    }

    fn into_goatd(self) -> Result<::goatd::flowcutter::Budget, String> {
        let positive = |value: i64, what: &str| {
            u64::try_from(value)
                .ok()
                .filter(|&value| value > 0)
                .ok_or_else(|| format!("FlowCutter {what} must be positive, got {value}"))
        };
        let iterations = |value: i32| {
            u32::try_from(value)
                .ok()
                .filter(|&value| value > 0)
                .ok_or_else(|| format!("FlowCutter iteration count must be positive, got {value}"))
        };

        match self {
            Self::Steps { steps, iters } => Ok(::goatd::flowcutter::Budget::steps(
                positive(steps, "step budget")?,
                iterations(iters)?,
            )),
            Self::Timed {
                timeout_ms,
                patience_ms,
                iters,
                steps,
                cap_mode,
            } => {
                let timeout = Duration::from_millis(positive(timeout_ms, "timeout")?);
                let patience = match patience_ms {
                    0 => None,
                    value => Some(Duration::from_millis(positive(value, "patience")?)),
                };
                let behavior = match cap_mode {
                    WallCapMode::BoundOnly => ::goatd::flowcutter::TimeoutBehavior::StopOnly,
                    WallCapMode::Tight => ::goatd::flowcutter::TimeoutBehavior::AdaptSearch,
                };
                Ok(::goatd::flowcutter::Budget::steps(
                    positive(steps, "step budget")?,
                    iterations(iters)?,
                )
                .with_timeout(timeout, patience, behavior))
            }
        }
    }
}

pub(crate) fn flowcutter_vtree(
    formula: &CnfFormula,
    kind: GraphKind,
    budget: FcBudget,
    request: ConversionRequest<'_>,
) -> Result<TdConversion, String> {
    let td = flowcutter_td(formula, kind, budget)?;
    Ok(convert_td(formula, &td, request))
}

pub(crate) fn flowcutter_td(
    formula: &CnfFormula,
    kind: GraphKind,
    budget: FcBudget,
) -> Result<TreeDecomposition, String> {
    let graph = kind.build(formula).as_goatd();
    ::goatd::flowcutter::decompose(&graph, budget.into_goatd()?).map_err(|error| error.to_string())
}
