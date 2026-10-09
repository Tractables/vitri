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
use crate::cnf::occ::literal_index;
use crate::cnf::{Clause, CnfFormula, Literal, normalize_literals};

use super::renumber::Renumber;

/// The literal graph node `node` stands for. A literal's node is its
/// [`literal_index`]: positive x → 2*x, negative x → 2*x+1.
#[inline]
fn node_to_lit(node: usize) -> Literal {
    let var = VarId::from_idx(node / 2);
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

/// The strongly connected components of the binary implication graph of
/// `clauses` that hold two or more literals, as groups of literal nodes. A
/// literal in no group is equivalent to no other.
///
/// Two literals in one component imply each other, so they are equivalent.
/// What a caller does with a component differs: this module substitutes the
/// smallest node of each ([`scc_representatives`]), while DVE's merge picks
/// the representative under its own frozen-variable policy.
///
/// The graph spans only the variables some binary clause mentions (see
/// [`ImplicationGraph`]), so its cost follows the binary clauses rather than
/// the declared variable count; the groups are stated in the original nodes.
pub(super) fn implication_sccs(clauses: &[Clause], num_vars: usize) -> Vec<Vec<usize>> {
    let graph = ImplicationGraph::of(clauses, num_vars);
    let mut groups = super::tarjan::tarjan_scc_groups(graph.num_nodes(), |v| graph.out(v));
    for node in groups.iter_mut().flatten() {
        *node = graph.original_node(*node);
    }
    groups
}

/// Each node's representative, the smallest node of its component; a node in
/// none of `groups` is its own.
///
/// `groups` comes from [`implication_sccs`], whose groups are in pop order, so
/// the minimum is searched for rather than read off the front.
pub(super) fn scc_representatives(groups: &[Vec<usize>], num_nodes: usize) -> Vec<usize> {
    let mut representative: Vec<usize> = (0..num_nodes).collect();
    for group in groups {
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
    ///
    /// A representative is its own positive literal, so the representatives
    /// are exactly the variables that map to themselves, met in ascending
    /// order.
    fn from_var_to_rep(var_to_rep: Vec<Literal>) -> Self {
        let mut rep_to_equivs: HashMap<VarId, Vec<Literal>> = HashMap::new();
        let mut representatives = Vec::new();

        for (v, &rep) in var_to_rep.iter().enumerate() {
            if rep.var.idx() == v {
                representatives.push(rep.var);
            } else {
                rep_to_equivs
                    .entry(rep.var)
                    .or_default()
                    .push(Literal::new(VarId::from_idx(v), rep.positive));
            }
        }
        debug_assert!(
            rep_to_equivs
                .keys()
                .all(|rep| var_to_rep[rep.idx()] == Literal::pos(*rep)),
            "every representative must map to its own positive literal",
        );

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
                let stripped_self = bb
                    .renumbering
                    .new_id(orig_var)
                    .expect("a variable the stripping kept must have an id in the stripped space");
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
            formula.num_vars() as usize,
            self.representatives.iter().copied(),
        );

        let num_reduced = self.representatives.len() as u32;
        let mut new_clauses: Vec<Vec<Literal>> = Vec::with_capacity(formula.clauses().len());
        let mut clause_set: HashSet<Vec<Literal>> = HashSet::new();

        for clause in formula.clauses() {
            let substituted = substitute_clause(clause, &self.var_to_rep, Some(&renumbering));
            if let Some(lits) = substituted
                && clause_set.insert(lits.clone())
            {
                new_clauses.push(lits);
            }
        }

        let clauses = new_clauses.into_iter().map(Clause::new).collect();
        let reduced = CnfFormula::from_parts(num_reduced, clauses);
        // `of_kept` already establishes that every kept id is an original-space
        // variable; this is the other half — the renumbering and the formula
        // agree on how many variables survived.
        debug_assert_eq!(
            renumbering.num_new_vars(),
            reduced.num_vars(),
            "the renumbering must keep exactly the reduced formula's variables",
        );
        (reduced, renumbering)
    }
}

/// Returns None if the clause becomes tautological, Some(sorted_lits) otherwise.
/// If `renumber` is provided, also renumbers representative VarIds to reduced IDs.
pub(super) fn substitute_clause(
    clause: &Clause,
    var_to_rep: &[Literal],
    renumber: Option<&Renumber>,
) -> Option<Vec<Literal>> {
    let mut new_lits: Vec<Literal> = Vec::with_capacity(clause.literals.len());

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
        new_lits.push(Literal::new(final_var, sub.positive));
    }

    normalize_literals(new_lits)
}

/// The binary implication graph of a clause set: each binary clause `(a ∨ b)`
/// is the two edges `¬a → b` and `¬b → a`.
///
/// Only the variables some binary clause mentions get nodes. They are numbered
/// in ascending order, two nodes each laid out as [`literal_index`] lays out
/// the original ones, so a search that visits nodes in index order meets them in
/// the order it would over the whole variable space; a variable no binary
/// clause mentions has no edge and is its own component either way. The edges
/// are stored flat, each node's in clause order.
struct ImplicationGraph {
    /// Graph variable `c` is original variable `vars[c]`.
    vars: Vec<usize>,
    /// The out-edges of node `v` are `targets[starts[v]..starts[v + 1]]`.
    starts: Vec<usize>,
    targets: Vec<usize>,
}

impl ImplicationGraph {
    fn of(clauses: &[Clause], num_vars: usize) -> Self {
        let binary = || clauses.iter().filter(|c| c.literals.len() == 2);

        // Mark the variables the binary clauses mention, then number the
        // marked ones in ascending order.
        let mut graph_var = vec![usize::MAX; num_vars];
        for clause in binary() {
            for lit in &clause.literals {
                graph_var[lit.var.idx()] = 0;
            }
        }
        let mut vars = Vec::new();
        for (v, slot) in graph_var.iter_mut().enumerate() {
            if *slot != usize::MAX {
                *slot = vars.len();
                vars.push(v);
            }
        }
        let node = |lit: Literal| literal_index(graph_var[lit.var.idx()], lit.positive);

        // Count each node's out-edges into the slot after its own, sum the
        // counts into start offsets, then fill: each start advances to the
        // next node's as its edges land, and one shift puts them back.
        let num_nodes = vars.len() * 2;
        let mut starts = vec![0usize; num_nodes + 1];
        for clause in binary() {
            let (a, b) = (node(clause.literals[0]), node(clause.literals[1]));
            starts[neg_node(a) + 1] += 1;
            starts[neg_node(b) + 1] += 1;
        }
        let mut sum = 0;
        for start in &mut starts {
            sum += *start;
            *start = sum;
        }
        let mut targets = vec![0usize; starts[num_nodes]];
        for clause in binary() {
            let (a, b) = (node(clause.literals[0]), node(clause.literals[1]));
            for (from, to) in [(neg_node(a), b), (neg_node(b), a)] {
                targets[starts[from]] = to;
                starts[from] += 1;
            }
        }
        starts.copy_within(0..num_nodes, 1);
        starts[0] = 0;

        ImplicationGraph {
            vars,
            starts,
            targets,
        }
    }

    fn num_nodes(&self) -> usize {
        self.vars.len() * 2
    }

    fn out(&self, node: usize) -> &[usize] {
        &self.targets[self.starts[node]..self.starts[node + 1]]
    }

    /// The node of the original variable space that graph node `node` is.
    fn original_node(&self, node: usize) -> usize {
        self.vars[node / 2] * 2 + node % 2
    }
}

/// Check whether any variable's positive and negative literals fall in the same SCC,
/// which would imply `x ↔ ¬x` — an UNSAT formula.
pub(super) fn has_equiv_contradiction(representative: &[usize], num_vars: usize) -> bool {
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
    let num_nodes = formula.num_vars() as usize * 2;
    if num_nodes == 0 {
        return EquivSccResult::NoEquivs;
    }

    let groups = implication_sccs(formula.clauses(), formula.num_vars() as usize);
    if groups.is_empty() {
        return EquivSccResult::NoEquivs;
    }
    let representative = scc_representatives(&groups, num_nodes);

    if has_equiv_contradiction(&representative, formula.num_vars() as usize) {
        return EquivSccResult::Unsat;
    }

    // Every group holds two or more literals and has one representative.
    EquivSccResult::Found {
        representative,
        equiv_count: groups.len(),
    }
}

/// Build the substituted formula, adding equivalence constraint clauses (`x ↔
/// rep`) so a non-representative variable stays in the formula's variable space
/// instead of silently dropping out of the model count.
fn build_substituted_formula(
    formula: &CnfFormula,
    representative: &[usize],
    var_to_rep: &[Literal],
    equiv_count: usize,
) -> EquivalenceResult {
    let n = formula.num_vars() as usize;

    let mut new_clauses: Vec<Vec<Literal>> = Vec::with_capacity(formula.clauses().len());
    let mut clause_set: HashSet<Vec<Literal>> = HashSet::new();

    for clause in formula.clauses() {
        if let Some(lits) = substitute_clause(clause, var_to_rep, None)
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
        let v_pos = Literal::pos(VarId::from_idx(v));
        let rep_lit = node_to_lit(rep_node);

        new_clauses.push(vec![v_pos.negated(), rep_lit]);
        new_clauses.push(vec![v_pos, rep_lit.negated()]);
    }

    let clauses = new_clauses.into_iter().map(Clause::new).collect();
    EquivalenceResult {
        formula: CnfFormula::from_parts(formula.num_vars(), clauses),
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
                formula: CnfFormula::from_parts(formula.num_vars(), vec![Clause::new(vec![])]),
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
            // One reading of the representative table, shared by the mapping the
            // caller keeps and the formula built under it.
            let var_to_rep = var_to_rep_of(&representative, formula.num_vars());
            let result =
                build_substituted_formula(formula, &representative, &var_to_rep, equiv_count);
            let mapping = EquivMapping::from_var_to_rep(var_to_rep);
            (result, Some(mapping))
        }
    }
}
