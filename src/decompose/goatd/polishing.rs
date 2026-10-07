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
    polishing::{Advance, Budget, Pause},
    vertex_rebuild,
};

use crate::budget::earliest;
use crate::cnf::CnfFormula;
use crate::decompose::{
    TdConversion, meter,
    td_to_vtree::{ConversionMemo, ConversionRequest, convert_td},
};
use crate::diagnostics::diag;
use crate::score::BUILT_FROM_THIS_FORMULA;

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

/// The work clock milliseconds the default policy polishes for: the work a
/// hundred milliseconds of real time bought on a typical instance when the
/// stage was bounded by elapsed time, measured on the machine the reading
/// charge was fitted on.
const DEFAULT_WORK_MS: u64 = 110;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Legacy {
        reinsertion: bool,
        separator: bool,
    },
    Adaptive {
        reinsertion_steps: u64,
        separator_steps: u64,
        work_ms: Option<u64>,
        separator: FlowCutterConfig,
    },
}

impl Default for GoatdPolishing {
    fn default() -> Self {
        Self {
            mode: Mode::Adaptive {
                reinsertion_steps: 8,
                separator_steps: 128,
                work_ms: Some(DEFAULT_WORK_MS),
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
                work_ms: None,
                separator: FlowCutterConfig::default(),
            },
        }
    }

    /// Also cap the complete adaptive stage by `milliseconds` of the
    /// construction's work clock, including validation, proposal assembly,
    /// conversion and acceptance.
    ///
    /// The work clock advances by the graph and formula work the stage
    /// charges, at goatd's calibration of work units per millisecond, so the
    /// stage stops after the same work on every machine and under any load,
    /// whether or not the construction as a whole is metered. The kernel
    /// scheduling limits still apply. The limit is cooperative: it is read
    /// between the kernel's scheduling steps and between proposals, so a step
    /// or a conversion in progress completes, and a conversion always
    /// completes its first tree.
    ///
    /// Returns an error for a legacy policy, which has no adaptive allocation.
    pub fn with_work_limit(mut self, milliseconds: u64) -> Result<Self, crate::error::VitriError> {
        let Mode::Adaptive {
            ref mut work_ms, ..
        } = self.mode
        else {
            return Err(crate::error::VitriError::config(
                "goatd polishing work limit requires adaptive polishing",
            ));
        };
        *work_ms = Some(milliseconds);
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
            work_ms,
            separator,
        } = self.mode
        else {
            return Ok(built);
        };
        let started = Instant::now();
        // The stage runs on the work clock. A metered construction has it
        // running already; any other arms it here, for this stage alone, so
        // where polishing stops depends on the formula and the proposals and
        // not on how fast or how loaded the machine is.
        let metered = meter::is_armed();
        let _work_clock = (!metered).then(|| meter::arm(started));
        let stage_end = work_ms
            .map(|ms| {
                meter::now()
                    .checked_add(Duration::from_millis(ms))
                    .ok_or("goatd polishing work limit is too large")
            })
            .transpose()?;
        // The caller's deadline stays on the clock it was set on: the work
        // clock for a metered construction, real time for any other, where it
        // joins the caller's real-time cutoff.
        let (work_deadline, real_deadline) = if metered {
            (request.deadline, request.real_deadline)
        } else {
            (None, earliest(request.deadline, request.real_deadline))
        };
        let work_end = earliest(stage_end, work_deadline);
        let expired = || {
            crate::budget::expired(work_end)
                || real_deadline.is_some_and(|end| Instant::now() >= end)
        };
        // A session advances one scheduling step at a time, so the work clock is
        // read between the kernel's steps as well as between its proposals; a
        // resumed session carries on exactly where the step left it.
        let budget = |steps| {
            real_deadline.map_or(Budget::new(steps), |end| {
                Budget::new(steps).with_deadline(end)
            })
        };
        // Proposals that differ little convert to many of the same trees and
        // bags, so every conversion the loop makes shares one memo — the
        // caller's when it has one — and the winner of each, which is read back
        // below, is already in it.
        let own;
        let memo = match request.memo {
            Some(memo) => memo,
            None => {
                own = ConversionMemo::new(formula);
                &own
            }
        };
        let mut cost = memo
            .costs
            .cost(&built.vtree, formula)
            .expect(BUILT_FROM_THIS_FORMULA);
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
                    deadline: work_end,
                    real_deadline,
                    memo: Some(memo),
                    ..nested
                },
            );
            let next = memo
                .costs
                .cost(&candidate.vtree, formula)
                .expect(BUILT_FROM_THIS_FORMULA);
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
                match session.advance(budget(1)) {
                    Advance::Proposal(proposal) => score(proposal),
                    Advance::Paused(Pause::Steps) => {}
                    _ => break,
                }
            }
            tree = session.into_tree();
        }
        if separator_steps > 0 && !expired() {
            let mut session =
                FlowCutterSession::new(graph, tree, separator).map_err(|e| e.to_string())?;
            while session.progress().steps < separator_steps && !expired() {
                match session.advance(budget(1)) {
                    Advance::Proposal(proposal) => score(proposal),
                    Advance::Paused(Pause::Steps) => {}
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
