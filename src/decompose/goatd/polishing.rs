//! What the goatd construction does with its winning decomposition once the
//! schedule has produced it: the policies a caller names through
//! [`GoatdPolishing`], projection-and-lift through [`GoatdLift`], and the loop
//! each runs.
//!
//! The adaptive policy is the one place this crate scores mid-construction. It
//! converts a refinement proposal, scores that tree against the formula, and
//! keeps the incumbent unless the proposal is cheaper, so refinement effort is
//! spent only where it buys a better tree.

use std::time::{Duration, Instant};

use ::goatd::decomposition::{
    FlowCutterConfig, FlowCutterSession,
    polishing::{Advance, Budget},
    vertex_rebuild,
};

use crate::cnf::CnfFormula;
use crate::decompose::{
    TdConversion,
    td_to_vtree::{ConversionRequest, convert_td},
};
use crate::diagnostics::diag;
use crate::score::{BUILT_FROM_THIS_FORMULA, vtree_cost};

/// Final decomposition refinement policy for a goatd construction.
///
/// `legacy` independently controls the two existing stages. `adaptive` offers
/// proposals to Vitri's vtree scorer, preserving an already converted incumbent
/// before spending refinement effort. This score is a construction heuristic;
/// it does not execute a downstream compiler.
///
/// The default is adaptive refinement, at the effort its [`Default`] names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct GoatdPolishing {
    mode: Mode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Legacy {
        reinsertion: bool,
        separator: bool,
    },
    Adaptive {
        reinsertion_steps: u64,
        separator_steps: u64,
        wall_ms: Option<u64>,
        separator: FlowCutterConfig,
    },
}

impl Default for GoatdPolishing {
    fn default() -> Self {
        Self {
            mode: Mode::Adaptive {
                reinsertion_steps: 8,
                separator_steps: 128,
                wall_ms: Some(100),
                separator: FlowCutterConfig::default(),
            },
        }
    }
}

impl GoatdPolishing {
    /// No final refinement of the winner. The standard candidate generators and
    /// the initial triangulation refinement still run, and the time this would
    /// have spent stays with them.
    pub const fn off() -> Self {
        Self::legacy(false, false)
    }

    /// Independently enable the existing final reinsertion and separator passes.
    /// Neither is [`GoatdPolishing::off`].
    pub const fn legacy(reinsertion: bool, separator: bool) -> Self {
        Self {
            mode: Mode::Legacy {
                reinsertion,
                separator,
            },
        }
    }

    /// Allocate scheduling operations to reinsertion, then separator refinement.
    /// Zero skips that kernel. Counts include setup and differ between kernels;
    /// they are not milliseconds or interchangeable amounts of graph work.
    /// Accepted proposals must strictly improve the converted vtree score.
    pub fn adaptive(reinsertion_steps: u64, separator_steps: u64) -> Self {
        Self {
            mode: Mode::Adaptive {
                reinsertion_steps,
                separator_steps,
                wall_ms: None,
                separator: FlowCutterConfig::default(),
            },
        }
    }

    /// Also cap the complete adaptive stage by elapsed milliseconds,
    /// including validation, proposal assembly, conversion and acceptance.
    /// The limit is cooperative; a conversion always completes its first tree.
    /// Deterministic construction measures this limit on its work clock and
    /// retains the kernel scheduling limits; other modes use real elapsed time.
    /// Returns an error for a legacy policy, which has no adaptive allocation.
    pub fn with_wall_limit(mut self, milliseconds: u64) -> Result<Self, crate::error::VitriError> {
        let Mode::Adaptive {
            ref mut wall_ms, ..
        } = self.mode
        else {
            return Err(crate::error::VitriError::config(
                "goatd polishing wall limit requires adaptive polishing",
            ));
        };
        *wall_ms = Some(milliseconds);
        Ok(self)
    }

    /// Set separator effort and recursion settings for adaptive refinement.
    /// Returns an error for a legacy policy.
    pub fn with_separator(
        mut self,
        config: FlowCutterConfig,
    ) -> Result<Self, crate::error::VitriError> {
        let Mode::Adaptive {
            ref mut separator, ..
        } = self.mode
        else {
            return Err(crate::error::VitriError::config(
                "goatd separator settings require adaptive polishing",
            ));
        };
        *separator = config;
        Ok(self)
    }

    pub(super) fn validate(self) -> Result<(), crate::error::VitriError> {
        if let Mode::Adaptive { separator, .. } = self.mode {
            separator
                .validate()
                .map_err(|e| crate::error::VitriError::config(format!("goatd.polishing: {e}")))?;
        }
        Ok(())
    }

    /// Whether this policy refines the winner at all.
    pub(super) fn is_off(self) -> bool {
        matches!(
            self.mode,
            Mode::Legacy {
                reinsertion: false,
                separator: false,
            }
        )
    }

    pub(super) fn reinsertion(self) -> bool {
        matches!(
            self.mode,
            Mode::Legacy {
                reinsertion: true,
                ..
            }
        )
    }
    pub(super) fn separator(self) -> bool {
        matches!(
            self.mode,
            Mode::Legacy {
                separator: true,
                ..
            }
        )
    }
    pub(super) fn is_adaptive(self) -> bool {
        matches!(self.mode, Mode::Adaptive { .. })
    }

    pub(super) fn refine(
        self,
        graph: &::goatd::Graph,
        tree: ::goatd::TreeDecomposition,
        mut built: TdConversion,
        formula: &CnfFormula,
        request: ConversionRequest<'_>,
        trace: bool,
    ) -> Result<TdConversion, String> {
        let Mode::Adaptive {
            reinsertion_steps,
            separator_steps,
            wall_ms,
            separator,
        } = self.mode
        else {
            return Ok(built);
        };
        let started = Instant::now();
        let stage_end = wall_ms
            .map(|ms| {
                crate::decompose::meter::now()
                    .checked_add(Duration::from_millis(ms))
                    .ok_or("goatd polishing wall limit is too large")
            })
            .transpose()?;
        let real_end = stage_end.filter(|_| !crate::decompose::meter::is_armed());
        let expired = || {
            stage_end.is_some_and(|end| crate::decompose::meter::now() >= end)
                || request
                    .deadline
                    .is_some_and(|end| crate::decompose::meter::now() >= end)
        };
        let budget = |steps| {
            real_end.map_or(Budget::new(steps), |end| {
                Budget::new(steps).with_deadline(end)
            })
        };
        let mut cost = vtree_cost(&built.vtree, formula).expect(BUILT_FROM_THIS_FORMULA);
        let mut proposed = 0u64;
        let mut accepted = 0u64;
        let mut score = |proposal: ::goatd::decomposition::polishing::Proposal<'_>| {
            if expired() {
                return;
            }
            let nested = request.nested();
            let candidate = convert_td(
                formula,
                proposal.candidate(),
                ConversionRequest {
                    deadline: nested.deadline.into_iter().chain(stage_end).min(),
                    real_deadline: match (nested.real_deadline, real_end) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    },
                    ..nested
                },
            );
            let next = vtree_cost(&candidate.vtree, formula).expect(BUILT_FROM_THIS_FORMULA);
            proposed += 1;
            if next < cost {
                proposal.accept();
                built = candidate;
                cost = next;
                accepted += 1;
            }
        };
        let mut tree = tree;
        if reinsertion_steps > 0 && !expired() {
            let mut session =
                vertex_rebuild::Session::new(graph, tree).map_err(|e| e.to_string())?;
            while session.progress().steps < reinsertion_steps && !expired() {
                let left = reinsertion_steps - session.progress().steps;
                match session.advance(budget(left)) {
                    Advance::Proposal(proposal) => score(proposal),
                    _ => break,
                }
            }
            tree = session.into_tree();
        }
        if separator_steps > 0 && !expired() {
            let mut session =
                FlowCutterSession::new(graph, tree, separator).map_err(|e| e.to_string())?;
            while session.progress().steps < separator_steps && !expired() {
                let left = separator_steps - session.progress().steps;
                match session.advance(budget(left)) {
                    Advance::Proposal(proposal) => score(proposal),
                    _ => break,
                }
            }
        }
        if trace {
            diag!(
                "[goatd-polishing] proposed={proposed} accepted={accepted} total_ms={} cost={cost}",
                started.elapsed().as_millis()
            );
        }
        Ok(built)
    }
}

/// Incidence projection-and-lift limits passed to goatd's portfolio.
#[derive(Clone, Copy, Debug, PartialEq)]
#[must_use]
pub struct GoatdLift {
    edge_factor: f64,
    edges_per_ms: f64,
}

impl GoatdLift {
    /// Allow a projection within `edge_factor` times the input edges and price
    /// its work at `edges_per_ms`. Both values must be finite and positive.
    pub fn new(edge_factor: f64, edges_per_ms: f64) -> Result<Self, crate::error::VitriError> {
        if !edge_factor.is_finite()
            || edge_factor <= 0.0
            || !edges_per_ms.is_finite()
            || edges_per_ms <= 0.0
        {
            return Err(crate::error::VitriError::config(
                "goatd lift edge factor and edges per millisecond must be positive",
            ));
        }
        Ok(Self {
            edge_factor,
            edges_per_ms,
        })
    }

    pub(super) fn apply(
        self,
        config: ::goatd::portfolio::PortfolioConfig,
    ) -> ::goatd::portfolio::PortfolioConfig {
        config
            .with_bipartite_lift(self.edge_factor)
            .with_bipartite_lift_rate(self.edges_per_ms)
    }
}
