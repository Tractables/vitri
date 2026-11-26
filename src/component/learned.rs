//! The learned tree against the portfolio's, per component
//! ([`RunConfig::model_order_percent`]).

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{VtreeBuild, graft_with_free_vars, local_view};
use crate::cnf::CnfFormula;
use crate::config::{PreprocessClock, RunConfig};
use crate::decompose::{PortfolioBuildHistory, meter, model_order};
use crate::diagnostics::diag;
use crate::preprocess::implicates::Allowance;
use crate::spec::SelectionRecord;
use crate::vtree::Vtree;

/// What [`SelectionRecord::winning_spec`] says of a learned tree.
pub(crate) const MODEL_ORDER: &str = "model-order";

/// Give each component of `built`, a build over `formula`, the learned tree
/// where [`model_order::pick`] takes it over the portfolio's. The allowance,
/// `percent` of the run's budget less what clones of `config` have spent
/// ([`RunConfig::model_order_spent`]), is shared out in turn: each component
/// the pick considers gets the share of what is left that its clauses are of
/// those components' clauses left, cut to what its portfolio tree leaves to
/// gain ([`model_order::allowance`]). No share is more than what is left
/// before `deadline`, construction's.
///
/// Under a deterministic construction budget the builds are measured in
/// charged work, solver work and readings, and charged to the construction
/// meter like the portfolio's entries.
///
/// When a learned tree was built, `history` gets its wall time and, when it
/// replaced a portfolio tree, the learned tree as the winner
/// ([`PortfolioBuildHistory::last_winning_spec`]).
pub(super) fn replace_with_learned(
    formula: &CnfFormula,
    built: &mut VtreeBuild,
    config: &RunConfig,
    percent: u32,
    deadline: Option<Instant>,
    history: &PortfolioBuildHistory,
) {
    // The config is anchored, so a run with a deadline has a budget too.
    let Some(budget_ms) = config.budget_ms else {
        return;
    };
    let started = Instant::now();
    let record = |built_any: bool, took: bool| {
        if built_any {
            let elapsed_ms = started.elapsed().as_millis() as u64;
            history.record_learned(elapsed_ms, took.then_some(MODEL_ORDER));
        }
    };
    let whole = Duration::from_millis(budget_ms.saturating_mul(u64::from(percent)) / 100);
    let mut left = whole.saturating_sub(config.model_order_spent.get());
    let construction_left = || deadline.map_or(Duration::MAX, crate::budget::remaining);
    let Some(comps) = built.components.as_mut() else {
        if formula.num_vars() <= model_order::MIN_VARS {
            return;
        }
        let share = left.min(construction_left());
        let Some(verdict) = consider("whole formula", formula, &built.vtree, share, config) else {
            return;
        };
        let took = verdict.tree.is_some();
        if let Some(tree) = verdict.tree {
            built.vtree = Arc::new(tree);
            built.selections[0] = learned_record();
        }
        record(true, took);
        return;
    };
    let eligible: Vec<bool> = comps
        .iter()
        .map(|cv| cv.local_to_outer.len() > model_order::MIN_VARS as usize)
        .collect();
    let mut clauses_left: usize = comps
        .iter()
        .zip(&eligible)
        .filter(|&(_, &e)| e)
        .map(|(cv, _)| cv.clause_indices.len())
        .sum();
    let mut took = false;
    let mut built_any = false;
    for (index, cv) in comps.iter_mut().enumerate() {
        if !eligible[index] {
            continue;
        }
        let local = local_view(formula, &cv.clause_indices, None).formula;
        let share = left
            .mul_f64(cv.clause_indices.len() as f64 / clauses_left.max(1) as f64)
            .min(construction_left());
        clauses_left -= cv.clause_indices.len();
        let before = config.model_order_spent.get();
        let verdict = consider(
            &format!("component {index}"),
            &local,
            &cv.vtree,
            share,
            config,
        );
        left = left.saturating_sub(config.model_order_spent.get().saturating_sub(before));
        built_any |= verdict.is_some();
        if let Some(tree) = verdict.and_then(|v| v.tree) {
            cv.vtree = Arc::new(tree);
            built.selections[index] = learned_record();
            took = true;
        }
    }
    if took {
        let mut in_component = vec![false; formula.num_vars() as usize];
        for cv in comps.iter() {
            for v in &cv.local_to_outer {
                in_component[v.idx()] = true;
            }
        }
        built.vtree = graft_with_free_vars(comps, &in_component, formula.num_vars());
    }
    record(built_any, took);
}

/// The pick's verdict on the learned tree of `formula` against `portfolio`,
/// built within `share` cut by [`model_order::allowance`], or `None` when
/// none was built; what it spends goes on `config`'s
/// [`RunConfig::model_order_spent`].
fn consider(
    what: &str,
    formula: &CnfFormula,
    portfolio: &Vtree,
    share: Duration,
    config: &RunConfig,
) -> Option<model_order::Verdict> {
    if !model_order::may_fit(formula, share) {
        diag!(
            "[model-order] {what}: {} variables, {} clauses: not built, too large for its share",
            formula.num_vars(),
            formula.clauses().len(),
        );
        return None;
    }
    let prior = model_order::prior(formula, portfolio);
    let Some(plan) = model_order::allowance(formula, &prior, share) else {
        diag!(
            "[model-order] {what}: {}: not built",
            described(formula, &prior)
        );
        return None;
    };
    let metered = meter::is_armed();
    let clock = if metered {
        PreprocessClock::Deterministic {
            configured_wall_ms: None,
        }
    } else {
        config.preprocess_clock
    };
    let verdict = model_order::pick(
        formula,
        portfolio,
        plan.draw,
        plan.repairs,
        &Allowance::of_run(clock, plan.nominal, plan.nominal, config.deadline),
    );
    config.model_order_spent.add(verdict.spent);
    if metered {
        let ms = verdict.spent.as_secs_f64() * 1000.0;
        meter::charge((ms * meter::UNITS_PER_MS as f64) as u64);
    }
    report(
        what,
        &format!("{}, draws of {}", described(formula, &prior), plan.draw),
        &verdict,
    );
    Some(verdict)
}

fn learned_record() -> SelectionRecord {
    SelectionRecord {
        winning_spec: Some(MODEL_ORDER.to_string()),
        scores: None,
        td_meta: None,
    }
}

/// The formula's size and its portfolio tree's [`model_order::Prior`], as the
/// report lines give them.
fn described(formula: &CnfFormula, prior: &model_order::Prior) -> String {
    format!(
        "{} variables, {} clauses, portfolio s0 {:.3} W {}",
        formula.num_vars(),
        formula.clauses().len(),
        prior.s_bound,
        prior.width,
    )
}

/// One line per component the pick built for: the verdict, what it spent,
/// and where the build's time and work went ([`model_order::Stages`]).
fn report(what: &str, described: &str, verdict: &model_order::Verdict) {
    let spent = verdict.spent.as_millis();
    let stages = &verdict.stages;
    match verdict.figures {
        None => diag!(
            "[model-order] {what}: {described}, {} ({spent} ms); stages {stages}",
            if stages.stopped {
                "stopped: the blend tree is too far behind"
            } else {
                "no learned tree within its allowance"
            },
        ),
        Some((mine, theirs)) => diag!(
            "[model-order] {what}: {described}, learned s {:.3} Omega {}, portfolio s {:.3} \
             Omega {}: {} ({spent} ms); stages {stages}",
            mine.s,
            mine.omega,
            theirs.s,
            theirs.omega,
            if verdict.tree.is_some() {
                "learned"
            } else {
                "portfolio"
            },
        ),
    }
}
