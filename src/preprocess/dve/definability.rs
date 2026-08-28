//! Definability infrastructure: primal graph, dual-CNF construction, and candidate selection.

use crate::preprocess::cadical_ffi::{CaDiCal, note_solver_unavailable};

use crate::cnf::Clause;

pub(super) struct PrimalGraph {
    adj_mat: Vec<Vec<u64>>,
    adj_list: Vec<Vec<u32>>,
}

impl PrimalGraph {
    pub(super) fn new(num_vars: usize, clauses: &[Clause]) -> Self {
        let words_per_row = num_vars.div_ceil(64);
        let mut adj_mat = vec![vec![0u64; words_per_row]; num_vars];
        let mut adj_list = vec![Vec::new(); num_vars];

        for clause in clauses {
            let lits = &clause.literals;
            for i in 0..lits.len() {
                for j in (i + 1)..lits.len() {
                    let v1 = lits[i].var.0 as usize;
                    let v2 = lits[j].var.0 as usize;
                    let (word, bit) = (v2 / 64, 1u64 << (v2 % 64));
                    if adj_mat[v1][word] & bit == 0 {
                        adj_mat[v1][word] |= bit;
                        adj_mat[v2][v1 / 64] |= 1u64 << (v1 % 64);
                        adj_list[v1].push(v2 as u32);
                        adj_list[v2].push(v1 as u32);
                    }
                }
            }
        }

        PrimalGraph { adj_mat, adj_list }
    }

    #[inline]
    fn has_edge(&self, v1: usize, v2: usize) -> bool {
        self.adj_mat[v1][v2 / 64] & (1u64 << (v2 % 64)) != 0
    }

    pub(super) fn is_simplicial(&self, v: usize) -> bool {
        let neighbors = &self.adj_list[v];
        for (i, &a) in neighbors.iter().enumerate() {
            for &b in &neighbors[i + 1..] {
                if !self.has_edge(a as usize, b as usize) {
                    return false;
                }
            }
        }
        true
    }
}

/// Above this variable count, skip the dense PrimalGraph (O(n²/64) bytes)
/// and the simplicial check. Use frequency-only candidate selection.
pub(super) const PRIMAL_GRAPH_MAX_VARS: usize = 10_000;

/// Largest rare-polarity occurrence count that still earns the simplicial
/// exemption below. Empirically chosen: raising it admits more variables whose
/// elimination grows the clause set, and spends more time in the O(d²)
/// simpliciality scan; lowering it loses eliminations that add no fill.
const SIMPLICIAL_RARE_POLARITY_MAX: u64 = 6;

/// Whether resolving `var` away is worth attempting.
///
/// Two ways to qualify. Either the resolvent count cannot exceed the number of
/// clauses that disappear — `pos·neg` bounds the resolvents from above and
/// `pos + neg` counts the occurrences resolution removes, so the clause set
/// does not grow. Or `var` is simplicial in the primal graph: its neighbours
/// already form a clique, so resolving it away adds no primal edge, and the
/// clause growth this permits is bounded by
/// [`SIMPLICIAL_RARE_POLARITY_MAX`].
///
/// A variable occurring in no clause is rejected up front: it would otherwise
/// satisfy the first test as `0 ≤ 0`.
///
/// `freq` is indexed `2·var` for positive and `2·var + 1` for negative
/// occurrences. `graph` is `None` when the formula was too large to build the
/// primal graph for, which leaves only the first test.
pub(super) fn is_ve_candidate(graph: Option<&PrimalGraph>, freq: &[u32], var: usize) -> bool {
    let pos_freq = freq[var * 2] as u64;
    let neg_freq = freq[var * 2 + 1] as u64;

    if pos_freq == 0 && neg_freq == 0 {
        return false;
    }

    if pos_freq * neg_freq <= pos_freq + neg_freq {
        return true;
    }

    if let Some(g) = graph
        && pos_freq.min(neg_freq) <= SIMPLICIAL_RARE_POLARITY_MAX
        && g.is_simplicial(var)
    {
        return true;
    }

    false
}

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
    /// The original copy of variable `v`.
    #[inline]
    pub(crate) fn original_dimacs(self, v: u32) -> i32 {
        (v + 1) as i32
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
        let has_candidate = clause
            .literals
            .iter()
            .any(|l| is_candidate[l.var.0 as usize]);
        if !has_candidate {
            continue;
        }
        for lit in &clause.literals {
            let v = lit.var.0 as usize;
            let dimacs_var = if is_candidate[v] {
                layout.primed_dimacs(candidate_idx[v])
            } else {
                layout.original_dimacs(lit.var.0)
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

#[cfg(test)]
pub(super) fn pick_def_vars(
    clauses: &[Clause],
    num_vars: usize,
    candidates: &[u32],
    time_limit_ms: u64,
) -> Vec<u32> {
    let mut meter =
        crate::preprocess::meter::PreprocessMeter::new(crate::config::PreprocessClock::WallClock);
    pick_def_vars_with_meter(clauses, num_vars, candidates, time_limit_ms, &mut meter)
}

pub(super) fn pick_def_vars_with_meter(
    clauses: &[Clause],
    num_vars: usize,
    candidates: &[u32],
    time_limit_ms: u64,
    meter: &mut crate::preprocess::meter::PreprocessMeter,
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
