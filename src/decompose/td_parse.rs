//! The PACE tree-decomposition interchange: the two graph projections of a CNF
//! (primal and incidence) around goatd's graph and decomposition types.
//!
//! Both halves of the bring-your-own-decomposer round trip — build the edges,
//! write them out, run any PACE treewidth solver, read its solution back, and
//! hand the result to [`td_to_vtree`](crate::decompose::td_to_vtree). Vitri's
//! goatd-backed constructions reach the conversion through the same types.
//!
//! With the CNF still in hand, convert through
//! [`td_to_vtree_reading`](crate::decompose::td_to_vtree_reading) and pass it:
//! one decomposition names many vtrees, and the formula is what lets the
//! conversion score them and keep the cheapest by
//! [`vtree_cost`](crate::score::vtree_cost) — which is what this crate's own
//! constructions do.
//!
//! PACE writes bag and vertex ids 1-based; everything stored here is 0-based. A
//! bag's vertex ids are bounded by the count the solution line declares, not by
//! `num_vars`: a decomposition of the incidence graph legitimately carries
//! clause vertices numbered at or above the variable count.

use rustc_hash::FxHashMap;

use crate::cnf::CnfFormula;
use crate::error::VitriError;

pub use goatd::{TdBag, TreeDecomposition};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// A clause longer than this contributes nothing to the co-occurrence graph.
/// Its O(k²) pairs are a clique, and a hub clause naming a large share of the
/// formula would drop that clique over everything the short clauses say about
/// which variables belong together — outvoting them wherever the two disagree.
///
/// The force-directed layout caps a clause enumeration of its own at
/// `CO_CLAUSE_CAP` (`decompose::force`) over a different object: the weights of
/// its MST candidate edges, not which pairs a graph has. Nothing in the tree
/// records why the two values differ.
pub(super) const COOC_CLAUSE_LEN_CAP: usize = 50;

/// Walk the clause co-occurrence relation of `formula`: `f` is handed each pair
/// of variables sharing a clause, once per clause that holds both.
///
/// THE definition of which pairs the co-occurrence graph has, for every reader
/// of it. One class of clause is left out and one class of vertex dropped; a
/// reader that saw either would be reading a different graph than the rest, on
/// the same formula:
///
/// - clauses longer than [`COOC_CLAUSE_LEN_CAP`], for the reason stated there;
/// - vertices at or above `num_vars`, which a decomposition of the incidence
///   graph legitimately carries and which name clauses rather than variables.
fn for_each_cooccurring_pair(formula: &CnfFormula, num_vars: u32, mut f: impl FnMut(u32, u32)) {
    let nv = num_vars as usize;
    for clause in &formula.clauses {
        if clause.literals.len() > COOC_CLAUSE_LEN_CAP {
            continue;
        }
        let vars: Vec<u32> = clause
            .literals
            .iter()
            .map(|l| l.var.0)
            .filter(|&v| (v as usize) < nv)
            .collect();
        for i in 0..vars.len() {
            for j in (i + 1)..vars.len() {
                f(vars[i], vars[j]);
            }
        }
    }
}

/// The (unweighted, deduplicated) primal-graph adjacency of `formula`: `adj[v]`
/// lists the variables that co-occur with `v` in some clause, ascending. Over
/// the pairs [`for_each_cooccurring_pair`] defines.
pub(crate) fn primal_adjacency(formula: &CnfFormula, num_vars: u32) -> Vec<Vec<u32>> {
    let mut primal_adj: Vec<Vec<u32>> = vec![Vec::new(); num_vars as usize];
    for_each_cooccurring_pair(formula, num_vars, |u, v| {
        primal_adj[u as usize].push(v);
        primal_adj[v as usize].push(u);
    });
    for nbrs in &mut primal_adj {
        nbrs.sort_unstable();
        nbrs.dedup();
    }
    primal_adj
}

/// Append the clique over `vars` to `out`, each pair oriented `u < v`.
fn push_clique(vars: &[u32], out: &mut Vec<(u32, u32)>) {
    for i in 0..vars.len() {
        for j in (i + 1)..vars.len() {
            out.push((vars[i].min(vars[j]), vars[i].max(vars[j])));
        }
    }
}

/// Build the primal graph edges: variables as vertices, clause co-occurrence
/// edges, in the form [`PaceGraph::edges`] describes.
pub(crate) fn build_primal_edges(formula: &CnfFormula) -> Vec<(u32, u32)> {
    let mut edges = Vec::new();
    for clause in &formula.clauses {
        let vars: Vec<u32> = clause.literals.iter().map(|l| l.var.0).collect();
        push_clique(&vars, &mut edges);
    }
    goatd::Graph::new(formula.num_vars, edges).edges().to_vec()
}

/// The primal graph induced on `subset`, read straight off the clauses and
/// renumbered so that vertex `subset[i]` becomes local id `i`.
///
/// Same result, and the same local id space — but the clique of a clause is
/// formed over the variables of `subset` it mentions rather than over all of
/// them, so a formula whose primal graph is far too dense to materialize is
/// still cheap to restrict. That is why this exists beside the other one: the
/// recursion's base case runs on subsets of a few dozen variables and must not
/// pay for the whole graph to get them.
pub(crate) fn primal_edges_on_subset(formula: &CnfFormula, subset: &[u32]) -> Vec<(u32, u32)> {
    let local = local_index(subset);
    let mut edges: Vec<(u32, u32)> = Vec::new();
    for clause in &formula.clauses {
        let local_vars: Vec<u32> = clause
            .literals
            .iter()
            .filter_map(|l| local.get(&l.var.0).copied())
            .collect();
        push_clique(&local_vars, &mut edges);
    }
    goatd::Graph::new(subset.len() as u32, edges)
        .edges()
        .to_vec()
}

/// The local id `subset[i] -> i` that every construction working on a variable
/// subset renumbers through.
///
/// `cnf::components` builds this map for itself: it sits below `decompose` in
/// the layering and keys by [`VarId`](crate::cnf::VarId).
pub(crate) fn local_index(subset: &[u32]) -> FxHashMap<u32, u32> {
    subset
        .iter()
        .enumerate()
        .map(|(i, &v)| (v, i as u32))
        .collect()
}

/// Build the incidence graph edges: variable-clause bipartite edges, in the
/// form [`PaceGraph::edges`] describes. Variables are vertices `0..num_vars`,
/// clauses are `num_vars..num_vars + num_clauses`.
pub(crate) fn build_incidence_edges(formula: &CnfFormula) -> Vec<(u32, u32)> {
    let mut edges = Vec::new();
    for (ci, clause) in formula.clauses.iter().enumerate() {
        let clause_vertex = formula.num_vars + ci as u32;
        for lit in &clause.literals {
            let var_vertex = lit.var.0;
            let (u, v) = (var_vertex.min(clause_vertex), var_vertex.max(clause_vertex));
            edges.push((u, v));
        }
    }
    goatd::Graph::new(formula.num_vars + formula.clauses.len() as u32, edges)
        .edges()
        .to_vec()
}

/// Which graph over a formula a decomposer is handed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphKind {
    /// Variables are the vertices; every pair of variables sharing a clause is
    /// an edge.
    Primal,
    /// Variables AND clauses are vertices; every literal is an edge between its
    /// variable and its clause.
    Incidence,
}

impl GraphKind {
    /// This view of `formula`, as the graph a decomposer takes.
    pub fn build(self, formula: &CnfFormula) -> PaceGraph {
        let (num_vertices, edges) = match self {
            GraphKind::Primal => (formula.num_vars, build_primal_edges(formula)),
            GraphKind::Incidence => (
                formula.num_vars + formula.clauses.len() as u32,
                build_incidence_edges(formula),
            ),
        };
        PaceGraph {
            kind: self,
            graph: goatd::Graph::new(num_vertices, edges),
        }
    }

    /// What this view is called in a `.gr` comment line.
    fn name(self) -> &'static str {
        match self {
            GraphKind::Primal => "primal",
            GraphKind::Incidence => "incidence",
        }
    }
}

/// One graph view of a formula: an edge list together with the vertex count it
/// runs over and the view it came from, paired by [`GraphKind::build`] so no
/// caller can hand one view's edges to the other's vertex count.
///
/// The outbound half of the bring-your-own-decomposer path: build the graph,
/// render it with [`to_gr`](PaceGraph::to_gr), run whatever solver you like,
/// then bring its `.td` back through [`parse_td`](PaceGraph::parse_td) and on to
/// [`td_to_vtree`](crate::decompose::td_to_vtree). Nothing about that round
/// trip is privileged — vitri's goatd-backed constructions use the same graph
/// and decomposition types.
///
/// ```no_run
/// use vitri::CnfFormula;
/// use vitri::decompose as d;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let (formula, _meta) = CnfFormula::from_dimacs("p cnf 3 2\n1 2 0\n-2 3 0\n".as_bytes())?;
///
/// let graph = d::GraphKind::Primal.build(&formula);
/// std::fs::write("instance.gr", graph.to_gr())?;
/// // Run any PACE treewidth solver on `instance.gr`, however you like.
/// let solution = std::fs::read_to_string("instance.td")?;
///
/// let td = graph.parse_td(&solution)?;
/// let vtree = d::td_to_vtree_reading(
///     &td,
///     formula.num_vars,
///     d::Reading::default(),
///     Some(&formula),
///     None,
/// );
/// println!("{} vtree nodes", vtree.num_nodes());
/// # Ok(())
/// # }
/// ```
///
/// Passing the formula to the conversion is what lets it search: with nothing
/// to score a reading against, one decomposition converts one way.
#[derive(Clone, Debug, PartialEq)]
pub struct PaceGraph {
    kind: GraphKind,
    graph: goatd::Graph,
}

impl PaceGraph {
    /// Which view of the formula this graph represents.
    pub fn kind(&self) -> GraphKind {
        self.kind
    }

    /// Number of vertices. For [`GraphKind::Incidence`], variables come first
    /// and clause vertices follow them.
    pub fn num_vertices(&self) -> u32 {
        self.graph.num_vertices()
    }

    /// Canonical undirected edges, sorted and deduplicated with `u < v`.
    pub fn edges(&self) -> &[(u32, u32)] {
        self.graph.edges()
    }

    /// This formula view as the graph type goatd's algorithms take.
    pub(crate) fn as_goatd(&self) -> &goatd::Graph {
        &self.graph
    }

    /// Render as a PACE `.gr` graph (1-indexed vertices), the input format
    /// every PACE treewidth solver reads.
    pub fn to_gr(&self) -> String {
        format!("c vitri {} graph\n{}", self.kind.name(), self.graph.to_gr())
    }

    /// Parse a PACE `.td` solution and validate it against this graph.
    ///
    /// # Errors
    ///
    /// [`VitriError::Input`] when `td_output` is malformed or does not cover
    /// this graph with a valid tree decomposition.
    pub fn parse_td(&self, td_output: &str) -> Result<TreeDecomposition, VitriError> {
        let decomposition = TreeDecomposition::from_td(td_output)
            .map_err(|error| VitriError::input(error.to_string()))?;
        decomposition
            .validate(&self.graph)
            .map_err(|error| VitriError::input(error.to_string()))?;
        Ok(decomposition)
    }
}

/// Read a PACE `.td` solution — an external treewidth solver's output — into a
/// [`TreeDecomposition`] this crate can convert (1-indexed in the file,
/// 0-indexed once stored).
///
/// The inbound half of the path [`PaceGraph::to_gr`] opens. Validation uses the
/// exact graph that was exported, including isolated vertices and incidence
/// clause vertices.
///
/// # Errors
///
/// [`VitriError::Input`] when `td_output` is malformed or is not a valid tree
/// decomposition.
pub fn parse_pace_td(td_output: &str, graph: &PaceGraph) -> Result<TreeDecomposition, VitriError> {
    graph.parse_td(td_output)
}
