//! Literal equivalence extraction via Tarjan SCC on the binary implication graph:
//! equivalence classes are substituted by a canonical representative, tautologies
//! and duplicate clauses/literals are removed, and a literal found equivalent to
//! its own negation reports UNSAT.
//!
//! Same core technique as PreLite (KCBox), implemented in-process rather than as
//! a subprocess.
//!
//! Reference: "The Power of Literal Equivalence in Model Counting" (AAAI-21)

use std::collections::{HashMap, HashSet};

use crate::cnf::VarId;
use crate::cnf::{Clause, CnfFormula, Literal};

use super::renumber::Renumber;

/// Encode a literal as a graph node index: positive x → 2*x, negative x → 2*x+1.
#[inline]
fn lit_to_node(lit: Literal) -> usize {
    crate::cnf::occ::literal_index(lit.var.idx(), lit.positive)
}

#[inline]
fn node_to_lit(node: usize) -> Literal {
    let var = VarId((node / 2) as u32);
    let positive = node.is_multiple_of(2);
    Literal::new(var, positive)
}

/// The literal variable `v` is equivalent to: `v ≡ rep_of(…)`. A representative
/// is its own positive literal.
///
/// `representative` is the node→node array Tarjan produced, so the answer for a
/// variable is read off its POSITIVE node and decoded back to a literal.
#[inline]
fn rep_of(representative: &[usize], v: usize) -> Literal {
    node_to_lit(representative[v * 2])
}

/// [`rep_of`] for every variable: the variable→representative table both the
/// substituted formula and [`EquivMapping`] are derived from.
fn var_to_rep_of(representative: &[usize], num_vars: u32) -> Vec<Literal> {
    (0..num_vars as usize)
        .map(|v| rep_of(representative, v))
        .collect()
}

#[inline]
fn neg_node(node: usize) -> usize {
    node ^ 1
}

/// Tarjan's SCC algorithm on the implication graph.
/// Returns a mapping from each node to its SCC representative (smallest node in SCC).
///
/// Delegates to the shared iterative implementation in `super::tarjan` to avoid
/// stack overflow on large graphs, then derives the node→representative map by
/// taking the minimum element of each SCC group (preserving the previous tie-breaking
/// rule: representative = smallest node index).
fn tarjan_scc(num_nodes: usize, adj: &[Vec<usize>]) -> Vec<usize> {
    let groups = super::tarjan::tarjan_scc_groups(num_nodes, adj);
    let mut representative = vec![0usize; num_nodes];
    for group in &groups {
        let rep = *group.iter().min().unwrap_or(&0);
        for &node in group {
            representative[node] = rep;
        }
    }
    representative
}

/// Result of equivalence extraction.
pub(super) struct EquivalenceResult {
    /// The simplified formula (same num_vars).
    pub formula: CnfFormula,
    /// Number of equivalence classes found (each with 2+ literals).
    pub num_equivalences: usize,
    /// Whether the formula was detected as UNSAT (literal ≡ ¬literal).
    pub is_unsat: bool,
}

/// Mapping from variables to their equivalence class representatives.
///
/// Enables two optimizations:
/// 1. Building vtrees with only representative variables (fewer, more meaningful vars)
/// 2. Compiling without equivalent variables, then expanding the TDD post-compilation
pub(crate) struct EquivMapping {
    /// For each original variable v: the literal `v` is equivalent to. A
    /// representative maps to its own positive literal.
    pub var_to_rep: Vec<Literal>,
    /// For each representative variable r: the literals, over the OTHER members
    /// of r's class, equivalent to r's positive literal.
    pub rep_to_equivs: HashMap<VarId, Vec<Literal>>,
    /// Sorted list of representative VarIds.
    pub representatives: Vec<VarId>,
}

impl EquivMapping {
    /// Build from the variable→representative table: the reverse index and the
    /// sorted representative list are derived from it, never assembled
    /// independently.
    fn from_var_to_rep(var_to_rep: Vec<Literal>) -> Self {
        let mut rep_to_equivs: HashMap<VarId, Vec<Literal>> = HashMap::new();
        let mut rep_set = HashSet::new();

        for (v, &rep) in var_to_rep.iter().enumerate() {
            rep_set.insert(rep.var);
            if rep.var.0 != v as u32 {
                rep_to_equivs
                    .entry(rep.var)
                    .or_default()
                    .push(Literal::new(VarId(v as u32), rep.positive));
            }
        }

        let mut representatives: Vec<VarId> = rep_set.into_iter().collect();
        representatives.sort_by_key(|v| v.0);

        EquivMapping {
            var_to_rep,
            rep_to_equivs,
            representatives,
        }
    }

    /// Remap this equivalence mapping into backbone-stripped variable space:
    /// backbone variables are removed — they can't be equivalence representatives
    /// or equivalents — and every surviving variable is renumbered via
    /// `bb.renumbering`.
    pub(crate) fn remap_for_stripped(
        &self,
        bb: &super::simplify::VariableStripping,
    ) -> Option<Self> {
        let stripped_num_vars = bb.renumbering.num_new_vars();

        let mut new_var_to_rep = Vec::with_capacity(stripped_num_vars as usize);
        for &orig_var in bb.renumbering.kept() {
            let rep = self.var_to_rep[orig_var.idx()];
            // The representative should also be non-backbone (a backbone var can't be
            // a representative of a non-backbone var because it's forced).
            if let Some(stripped_rep) = bb.renumbering.new_id(rep.var) {
                new_var_to_rep.push(Literal::new(stripped_rep, rep.positive));
            } else {
                // Backbone vars are forced, so equiv detection shouldn't pair them
                // with non-forced vars. If this fires, backbone/equiv detection ran
                // out of order or one of them produced an inconsistent mapping.
                debug_assert!(
                    false,
                    "non-backbone var {:?} has backbone representative {:?}",
                    orig_var, rep.var
                );
                let stripped_self = bb.renumbering.new_id(orig_var).unwrap();
                new_var_to_rep.push(Literal::pos(stripped_self));
            }
        }

        let remapped = EquivMapping::from_var_to_rep(new_var_to_rep);
        if remapped.rep_to_equivs.is_empty() {
            return None;
        }

        Some(remapped)
    }

    /// Create a reduced formula with only representative variables, renumbered
    /// 0..K-1. Does NOT add equivalence constraint clauses (caller handles that).
    ///
    /// Returns `(reduced_formula, renumbering)` — the renumbering keeps exactly
    /// the representatives, so `renumbering.old_id(reduced_id)` is the original
    /// VarId.
    ///
    /// The clause rewrite here SUBSTITUTES before it renumbers, which is why it
    /// is not the shared [`renumber_clauses`](super::renumber::renumber_clauses):
    /// a non-representative's literals are not dropped, they become their
    /// representative's, possibly flipped.
    pub(crate) fn reduce_formula(&self, formula: &CnfFormula) -> (CnfFormula, Renumber) {
        // Representatives are sorted, so keeping them IS the contiguous
        // renumbering into the reduced space.
        let renumbering = Renumber::of_kept(
            formula.num_vars as usize,
            self.representatives.iter().copied(),
        );

        let num_reduced = self.representatives.len() as u32;
        let mut new_clauses: Vec<Vec<Literal>> = Vec::with_capacity(formula.clauses.len());
        let mut clause_set: HashSet<Vec<Literal>> = HashSet::new();

        for clause in &formula.clauses {
            let substituted = substitute_clause(clause, &self.var_to_rep, Some(&renumbering));
            if let Some(lits) = substituted
                && clause_set.insert(lits.clone())
            {
                new_clauses.push(lits);
            }
        }

        let clauses = new_clauses.into_iter().map(Clause::new).collect();
        let reduced = CnfFormula {
            num_vars: num_reduced,
            clauses,
        };
        // `of_kept` already establishes that every kept id is an original-space
        // variable; this is the other half — the renumbering and the formula
        // agree on how many variables survived.
        debug_assert_eq!(
            renumbering.num_new_vars(),
            reduced.num_vars,
            "the renumbering must keep exactly the reduced formula's variables",
        );
        (reduced, renumbering)
    }
}

/// Returns None if the clause becomes tautological, Some(sorted_lits) otherwise.
/// If `renumber` is provided, also renumbers representative VarIds to reduced IDs.
fn substitute_clause(
    clause: &Clause,
    var_to_rep: &[Literal],
    renumber: Option<&Renumber>,
) -> Option<Vec<Literal>> {
    let mut new_lits: Vec<Literal> = Vec::with_capacity(clause.literals.len());
    let mut lit_set: HashSet<usize> = HashSet::new();

    for &lit in &clause.literals {
        // `v ≡ rep` substituted into a literal over `v`: the positive literal
        // becomes `rep`, the negative one `¬rep`.
        let rep = var_to_rep[lit.var.idx()];
        let sub = if lit.positive { rep } else { rep.negated() };
        // A representative is by construction one of the variables the
        // renumbering keeps, so this never fires.
        let final_var = renumber.map_or(sub.var, |r| {
            r.new_id(sub.var)
                .expect("an equivalence representative must survive into the reduced formula")
        });
        let rep_node = final_var.0 as usize * 2 + if sub.positive { 0 } else { 1 };

        if lit_set.contains(&(rep_node ^ 1)) {
            return None;
        }

        if lit_set.insert(rep_node) {
            new_lits.push(Literal::new(final_var, sub.positive));
        }
    }

    new_lits.sort_by_key(|l| (l.var.0, !l.positive));
    Some(new_lits)
}

/// Build the binary implication graph for the formula.
///
/// Each variable contributes two nodes (positive and negative literal). Each binary
/// clause `(a ∨ b)` encodes the implications `¬a → b` and `¬b → a` as graph edges.
fn build_implication_graph(formula: &CnfFormula) -> Vec<Vec<usize>> {
    let num_nodes = formula.num_vars as usize * 2;
    let mut adj = vec![Vec::new(); num_nodes];
    for clause in &formula.clauses {
        if clause.literals.len() == 2 {
            let a = lit_to_node(clause.literals[0]);
            let b = lit_to_node(clause.literals[1]);
            adj[neg_node(a)].push(b);
            adj[neg_node(b)].push(a);
        }
    }
    adj
}

/// Check whether any variable's positive and negative literals fall in the same SCC,
/// which would imply `x ↔ ¬x` — an UNSAT formula.
fn has_equiv_contradiction(representative: &[usize], num_vars: usize) -> bool {
    (0..num_vars).any(|v| representative[v * 2] == representative[v * 2 + 1])
}

enum EquivSccResult {
    /// Some variable's pos/neg literals landed in the same SCC → `x ↔ ¬x` → UNSAT.
    Unsat,
    /// Graph build + SCC succeeded but no multi-vertex SCCs (nothing to substitute).
    NoEquivs,
    /// At least one non-trivial equivalence class found.
    Found {
        representative: Vec<usize>,
        equiv_count: usize,
    },
}

/// Runs the SCC pipeline once and classifies the result — merging the UNSAT and
/// equivalence-found paths avoids rebuilding the graph / re-running Tarjan the
/// two distinct outcomes would otherwise need.
fn find_equivalences(formula: &CnfFormula) -> EquivSccResult {
    let num_nodes = formula.num_vars as usize * 2;
    if num_nodes == 0 {
        return EquivSccResult::NoEquivs;
    }

    let adj = build_implication_graph(formula);
    let representative = tarjan_scc(num_nodes, &adj);

    if has_equiv_contradiction(&representative, formula.num_vars as usize) {
        return EquivSccResult::Unsat;
    }

    let mut equiv_count = 0;
    let mut seen_reps = vec![false; num_nodes];
    for (node, &rep) in representative.iter().enumerate() {
        if rep != node && !seen_reps[rep] {
            seen_reps[rep] = true;
            equiv_count += 1;
        }
    }

    if equiv_count == 0 {
        EquivSccResult::NoEquivs
    } else {
        EquivSccResult::Found {
            representative,
            equiv_count,
        }
    }
}

/// Build the substituted formula, adding equivalence constraint clauses (`x ↔
/// rep`) so a non-representative variable stays in the formula's variable space
/// instead of silently dropping out of the model count.
fn build_substituted_formula(
    formula: &CnfFormula,
    representative: &[usize],
    equiv_count: usize,
) -> EquivalenceResult {
    let n = formula.num_vars as usize;

    let var_to_rep = var_to_rep_of(representative, formula.num_vars);

    let mut new_clauses: Vec<Vec<Literal>> = Vec::with_capacity(formula.clauses.len());
    let mut clause_set: HashSet<Vec<Literal>> = HashSet::new();

    for clause in &formula.clauses {
        if let Some(lits) = substitute_clause(clause, &var_to_rep, None)
            && clause_set.insert(lits.clone())
        {
            new_clauses.push(lits);
        }
    }

    for v in 0..n {
        let pos_node = v * 2;
        let rep_node = representative[pos_node];
        if rep_node == pos_node {
            continue;
        }
        let v_pos = Literal::new(VarId(v as u32), true);
        let v_neg = Literal::new(VarId(v as u32), false);
        let rep_lit = node_to_lit(rep_node);
        let rep_neg = Literal::new(rep_lit.var, !rep_lit.positive);

        new_clauses.push(vec![v_neg, rep_lit]);
        new_clauses.push(vec![v_pos, rep_neg]);
    }

    let clauses = new_clauses.into_iter().map(Clause::new).collect();
    EquivalenceResult {
        formula: CnfFormula {
            num_vars: formula.num_vars,
            clauses,
        },
        num_equivalences: equiv_count,
        is_unsat: false,
    }
}

/// Extract equivalences and also return the variable mapping for reduced compilation.
///
/// Returns `(result, Some(mapping))` when equivalences are found,
/// `(result, None)` when no equivalences exist or formula is UNSAT.
pub(super) fn extract_equivalences_with_mapping(
    formula: &CnfFormula,
) -> (EquivalenceResult, Option<EquivMapping>) {
    match find_equivalences(formula) {
        EquivSccResult::Unsat => (
            EquivalenceResult {
                formula: CnfFormula {
                    num_vars: formula.num_vars,
                    clauses: vec![Clause::new(vec![])],
                },
                num_equivalences: 0,
                is_unsat: true,
            },
            None,
        ),
        EquivSccResult::NoEquivs => (
            EquivalenceResult {
                formula: formula.clone(),
                num_equivalences: 0,
                is_unsat: false,
            },
            None,
        ),
        EquivSccResult::Found {
            representative,
            equiv_count,
        } => {
            let mapping =
                EquivMapping::from_var_to_rep(var_to_rep_of(&representative, formula.num_vars));
            let result = build_substituted_formula(formula, &representative, equiv_count);
            (result, Some(mapping))
        }
    }
}
