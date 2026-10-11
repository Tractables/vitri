//! Definability: the dual-CNF construction and the greedy probe that picks a
//! set of variables jointly defined by the rest.

use crate::preprocess::cadical_ffi::{CaDiCal, note_solver_unavailable};

use crate::cnf::{Clause, VarId};

/// Where the three id families of the dual CNF live, 1-indexed DIMACS:
///
///   [1 .. num_vars]                          — original variables
///   [num_vars+1 .. num_vars+nc]              — primed copies of candidates
///   [num_vars+nc+1 .. num_vars+2·nc]         — XOR indicator per candidate
///
/// Every id the construction and the probe loop name comes from one of the
/// three methods below, so the layout cannot be spelled one way in the builder
/// and another way in the reader.
#[derive(Clone, Copy)]
pub(crate) struct DualLayout {
    pub(crate) num_vars: usize,
    pub(crate) num_candidates: usize,
}

impl DualLayout {
    /// The original copy of the variable at index `v` ([`VarId::idx`]).
    #[inline]
    pub(crate) fn original_dimacs(self, v: u32) -> i32 {
        VarId::from_idx(v as usize).to_dimacs()
    }

    /// The primed copy of the `i`-th candidate.
    #[inline]
    pub(crate) fn primed_dimacs(self, i: usize) -> i32 {
        (self.num_vars + i + 1) as i32
    }

    /// The XOR indicator of the `i`-th candidate.
    #[inline]
    pub(crate) fn indicator_dimacs(self, i: usize) -> i32 {
        (self.num_vars + self.num_candidates + i + 1) as i32
    }

    /// How many variables the dual formula spans.
    #[inline]
    pub(crate) fn total_vars(self) -> usize {
        self.num_vars + 2 * self.num_candidates
    }
}

/// Dual-CNF construction with XOR indicators for Padoa-style definability
/// probing. Non-candidate vars are shared between the original and primed
/// clauses; the indicator clauses enforce `indicator → (v ↔ v')`, so assuming
/// an indicator true makes a candidate and its primed copy agree. The ids are
/// [`DualLayout`]'s.
pub(crate) struct DualCnf {
    pub(crate) solver: CaDiCal,
    pub(crate) layout: DualLayout,
}

/// Largest input this crate will duplicate into a dual CNF.
///
/// Both the internal DVE pass and the public projection classifier use this
/// one ceiling. Above it, duplicating the clauses has caused allocation failure
/// or stack overflow before a SAT budget can take effect.
pub(crate) const MAX_DUAL_CNF_CLAUSES: usize = 500_000;

/// `None` when constructing the dual formula is unsafe at this size or no solver
/// could be allocated — there is no dual formula to probe in, and the caller
/// reads that as "nothing proven defined".
pub(crate) fn build_dual_cnf_with_indicators(
    clauses: &[Clause],
    num_vars: usize,
    candidates: &[u32],
) -> Option<DualCnf> {
    // This must precede every allocation in the builder. A caller's wall-clock
    // terminator can bound SAT search only after the complete encoding exists.
    if clauses.len() > MAX_DUAL_CNF_CLAUSES {
        return None;
    }

    let layout = DualLayout {
        num_vars,
        num_candidates: candidates.len(),
    };
    let mut is_candidate = vec![false; num_vars];
    let mut candidate_idx = vec![0usize; num_vars];
    for (i, &v) in candidates.iter().enumerate() {
        is_candidate[v as usize] = true;
        candidate_idx[v as usize] = i;
    }

    let mut solver = CaDiCal::new()?;
    solver.reserve(layout.total_vars() as i32);

    let signed = |dimacs_var: i32, positive: bool| if positive { dimacs_var } else { -dimacs_var };

    // Original clauses: every literal uses its native DIMACS var (1-indexed).
    for clause in clauses {
        for lit in &clause.literals {
            solver.add(lit.to_dimacs());
        }
        solver.add(0);
    }

    for clause in clauses {
        let has_candidate = clause.literals.iter().any(|l| is_candidate[l.var.idx()]);
        if !has_candidate {
            continue;
        }
        for lit in &clause.literals {
            let v = lit.var.idx();
            let dimacs_var = if is_candidate[v] {
                layout.primed_dimacs(candidate_idx[v])
            } else {
                lit.var.to_dimacs()
            };
            solver.add(signed(dimacs_var, lit.positive));
        }
        solver.add(0);
    }

    for (i, &v) in candidates.iter().enumerate() {
        let v_dimacs = layout.original_dimacs(v);
        let v_prime_dimacs = layout.primed_dimacs(i);
        let indicator = layout.indicator_dimacs(i);

        solver.add(-indicator);
        solver.add(v_dimacs);
        solver.add(-v_prime_dimacs);
        solver.add(0);
        solver.add(-indicator);
        solver.add(-v_dimacs);
        solver.add(v_prime_dimacs);
        solver.add(0);
    }

    Some(DualCnf { solver, layout })
}

/// Conflicts granted per clause of the input formula, so that the per-probe
/// budget below scales with instance size. Empirically chosen.
const CONFLICTS_PER_CLAUSE: usize = 5;

/// Ceiling on the per-probe conflict budget. Empirically chosen: a probe that
/// is going to answer "defined" answers quickly, so a long search is evidence
/// against definability rather than a reason to keep going, and every conflict
/// spent here comes out of `time_limit_ms` for the candidates not yet tried.
const MAX_CONFLICTS_PER_PROBE: usize = 5_000;

/// A set of candidates jointly defined by the variables outside it, found
/// greedily in `candidates` order (Padoa's method on the dual CNF).
///
/// Candidate `i` joins the set when it is defined by every variable except
/// itself and the candidates already in the set: `known_defined(v, in_set)`
/// may vouch for that without a solver call (`in_set` is indexed by variable),
/// and otherwise the solver must show it, within its conflict budget. Each
/// member is therefore defined by variables that are outside the set or joined
/// it later, and composing the definitions from the last member back gives the
/// property the caller needs: the whole set is defined by the variables
/// outside it, and stays so after any member is eliminated by resolution.
/// A probe cut short by `time_limit_ms` returns the members found so far,
/// which have the same property.
pub(super) fn pick_def_vars_with_meter(
    clauses: &[Clause],
    num_vars: usize,
    candidates: &[u32],
    time_limit_ms: u64,
    meter: &mut crate::preprocess::meter::PreprocessMeter,
    mut known_defined: impl FnMut(u32, &[bool]) -> bool,
) -> Vec<u32> {
    if candidates.is_empty() {
        return Vec::new();
    }

    let mark = meter.begin(
        crate::bundle::PreprocessPhase::Dve,
        std::time::Duration::from_millis(time_limit_ms),
    );
    let Some(mut dc) = build_dual_cnf_with_indicators(clauses, num_vars, candidates) else {
        note_solver_unavailable("definability", "no variable is proven defined");
        return Vec::new();
    };

    let conflict_budget =
        (clauses.len() * CONFLICTS_PER_CLAUSE).min(MAX_CONFLICTS_PER_PROBE) as i32;

    let mut defined = Vec::new();
    let mut is_defined = vec![false; num_vars];

    for (i, &v) in candidates.iter().enumerate() {
        if meter.elapsed_ms(mark) > time_limit_ms {
            break;
        }

        if known_defined(v, &is_defined) {
            is_defined[v as usize] = true;
            defined.push(v);
            continue;
        }

        let v_dimacs = dc.layout.original_dimacs(v);
        let v_prime_dimacs = dc.layout.primed_dimacs(i);

        dc.solver.limit(c"conflicts", conflict_budget);

        dc.solver.assume(v_dimacs);
        dc.solver.assume(-v_prime_dimacs);

        for (j, &w) in candidates.iter().enumerate() {
            if j != i && !is_defined[w as usize] {
                let indicator = dc.layout.indicator_dimacs(j);
                dc.solver.assume(indicator);
            }
        }

        let result = meter.solve(crate::bundle::PreprocessPhase::Dve, &mut dc.solver);
        if result == crate::preprocess::cadical_ffi::Status::Unsatisfiable {
            is_defined[v as usize] = true;
            defined.push(v);
        }
    }

    defined
}
