//! The PACE tree-decomposition interchange: the [`TreeDecomposition`] type, the
//! two graph projections of a CNF (primal and incidence), and the `.gr` writer
//! and `.td` reader around them.
//!
//! Both halves of the bring-your-own-decomposer round trip — build the edges,
//! write them out, run any PACE treewidth solver, read its solution back, and
//! hand the result to [`td_to_vtree`](crate::decompose::td_to_vtree). The
//! crate's own bundled decomposers reach the conversion the same way.
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

/// One bag of a tree decomposition: a set of variables that must all be
/// mutually adjacent in the vtree's separator structure at that point.
#[derive(Clone, Debug, PartialEq)]
pub struct TdBag {
    /// Index of this bag within [`TreeDecomposition::bags`]; also the index used
    /// by [`TreeDecomposition::adj`].
    pub id: usize,
    /// The bag's vertices, 0-indexed (PACE `.td` vertex ids minus one) —
    /// variables, and on an incidence decomposition clause vertices too;
    /// [`TreeDecomposition::variable_vertices`] separates them.
    pub vertices: Vec<u32>,
}

/// A parsed PACE tree decomposition, ready for `td_to_vtree` conversion.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeDecomposition {
    /// Which graph over the formula the bags decompose, and so what a bag's
    /// vertices are.
    pub kind: GraphKind,
    /// The decomposition's bags, indexed by [`TdBag::id`].
    pub bags: Vec<TdBag>,
    /// Tree adjacency between bags, indexed like `bags`: `adj[i]` lists the
    /// bag indices connected to bag `i`.
    pub adj: Vec<Vec<usize>>,
    /// How many of the decomposed graph's vertices are the original CNF's
    /// variables: vertex ids below this are variables, and on a
    /// [`GraphKind::Incidence`] decomposition the ids from here up are the
    /// clause vertices. Not every variable need appear in a bag.
    pub num_vars: u32,
}

impl TreeDecomposition {
    /// The variables in `bag`: its vertices, with a
    /// [`GraphKind::Incidence`] decomposition's clause vertices left out.
    ///
    /// What a consumer holding a bag and a variable-length array wants, rather
    /// than [`TdBag::vertices`] itself.
    pub fn variable_vertices<'a>(&'a self, bag: &'a TdBag) -> impl Iterator<Item = u32> + 'a {
        let cutoff = match self.kind {
            GraphKind::Primal => u32::MAX,
            GraphKind::Incidence => self.num_vars,
        };
        bag.vertices.iter().copied().filter(move |&v| v < cutoff)
    }

    /// This decomposition's width: the vertices in its largest bag, less one.
    /// `0` where there is nothing to separate — no bags, one empty bag and one
    /// single-vertex bag alike.
    ///
    /// An UPPER bound on the decomposed graph's treewidth, which is the
    /// minimum width over all of its decompositions.
    pub fn treewidth(&self) -> u32 {
        self.bags
            .iter()
            .map(|b| b.vertices.len() as u32)
            .max()
            .unwrap_or(0)
            .saturating_sub(1)
    }

    /// Sum of bag sizes. Used as a secondary quality signal alongside the
    /// treewidth: two TDs of equal width can have very different total bag
    /// volume, and the tighter one usually produces a smaller compilation.
    pub fn total_bag_size(&self) -> usize {
        self.bags.iter().map(|b| b.vertices.len()).sum()
    }
}

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

/// Put an edge list in the form [`PaceGraph::edges`] describes.
fn canonical(mut edges: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    edges.sort_unstable();
    edges.dedup();
    edges
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
    canonical(edges)
}

/// The primal edges induced on `subset`, renumbered so that vertex `subset[i]`
/// becomes local id `i`, in the form [`PaceGraph::edges`] describes — a caller
/// must map back through `subset` before touching variable ids again.
///
/// Every recursive vtree construction here works on a shrinking variable subset
/// and needs the induced graph in a dense id space to hand a partitioner or an
/// elimination order. This is that operation for a caller that already holds the
/// whole formula's edge list; [`primal_edges_on_subset`] is the same result for
/// one that does not.
pub(crate) fn restrict_to_subset(edges: &[(u32, u32)], subset: &[u32]) -> Vec<(u32, u32)> {
    // Charged at the length of the FULL list, because that is what the
    // restriction reads: every caller here is a recursion that tests every edge
    // of the formula for containment at every level, so on a deep recursion over
    // a large primal graph this scan — not the partition or the elimination that
    // follows it — is where the level's work goes.
    crate::decompose::meter::charge(edges.len() as u64);
    let local = local_index(subset);
    let mut out = Vec::new();
    for &(u, v) in edges {
        if let (Some(&lu), Some(&lv)) = (local.get(&u), local.get(&v)) {
            out.push((lu.min(lv), lu.max(lv)));
        }
    }
    canonical(out)
}

/// [`restrict_to_subset`] of [`build_primal_edges`], read straight off the
/// clauses.
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
    canonical(edges)
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
    canonical(edges)
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
            num_vertices,
            edges,
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
/// then bring its `.td` back through [`parse_pace_td`] and on to
/// [`td_to_vtree`](crate::decompose::td_to_vtree). Nothing about that round
/// trip is privileged — it is the same route this crate's own bundled
/// decomposers take.
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
/// let td = d::parse_pace_td(&solution, graph.kind, formula.num_vars)?;
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
    /// Which view of the formula the edges are over.
    pub kind: GraphKind,
    /// The vertices are `0..num_vertices`; for [`GraphKind::Incidence`] the
    /// variables come first and the clause vertices after them.
    pub num_vertices: u32,
    /// Sorted, deduplicated `(u, v)` with `u < v`, 0-indexed.
    pub edges: Vec<(u32, u32)>,
}

impl PaceGraph {
    /// Render as a PACE `.gr` graph (1-indexed vertices), the input format
    /// every PACE treewidth solver reads.
    pub fn to_gr(&self) -> String {
        let mut out = format!(
            "c vitri {} graph\np tw {} {}\n",
            self.kind.name(),
            self.num_vertices,
            self.edges.len()
        );
        for &(u, v) in &self.edges {
            out.push_str(&format!("{} {}\n", u + 1, v + 1));
        }
        out
    }
}

/// Read a PACE `.td` solution — an external treewidth solver's output — into a
/// [`TreeDecomposition`] this crate can convert (1-indexed in the file,
/// 0-indexed once stored).
///
/// The inbound half of the path [`PaceGraph::to_gr`] opens: `kind` is the view
/// whose `.gr` the solver was given, and the result records it. `num_vars`
/// bounds the variable ids only; a decomposition of the incidence graph
/// legitimately carries higher vertex ids, and the solution line's own count is
/// what bounds those.
///
/// # Errors
///
/// [`VitriError::Input`] describing what is wrong with `td_output`: a malformed
/// solution or bag line, an unparseable id, a bag or vertex id outside the range
/// the solution line declares, a bag list that does not define each declared bag
/// exactly once, or no bags at all.
pub fn parse_pace_td(
    td_output: &str,
    kind: GraphKind,
    num_vars: u32,
) -> Result<TreeDecomposition, VitriError> {
    parse_pace_td_inner(td_output, kind, num_vars).map_err(VitriError::input)
}

/// The parse itself, reporting a plain sentence. Private: the sentence becomes a
/// [`VitriError`] at the one public entry point above.
fn parse_pace_td_inner(
    td_output: &str,
    kind: GraphKind,
    num_vars: u32,
) -> Result<TreeDecomposition, String> {
    let mut bags: Vec<TdBag> = Vec::new();
    let mut adj: Vec<Vec<usize>> = Vec::new();
    let mut num_bags = 0usize;
    // The solution line's vertex count. A decomposition of a graph other than
    // the primal one — the incidence graph, say — has more vertices than the
    // formula has variables, so it is the line's own count that bounds a bag's
    // vertices, never `num_vars`.
    let mut declared_vertices = 0usize;
    let mut saw_solution_line = false;

    // Bag and vertex ids are written 1-based and stored 0-based, so `0` is not
    // an id at all — reject it rather than wrap the subtraction. `limit` is what
    // the solution line declared, which the id also has to address.
    let to_zero_based = |what: &str, id: usize, limit: usize| -> Result<usize, String> {
        if id == 0 {
            return Err(format!("{what} 0 is out of range; ids are 1-based"));
        }
        if id > limit {
            return Err(format!(
                "{what} {id} is out of range; the solution line declares {limit}"
            ));
        }
        Ok(id - 1)
    };

    for line in td_output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('c') {
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        match tokens[0] {
            "s" => {
                // "s td <num_bags> <max_bag_size> <num_vertices>" — the counts
                // every id below is checked against, so all three are required.
                if tokens.len() < 5 {
                    return Err(format!("Malformed solution line: {}", line));
                }
                num_bags = tokens[2].parse::<usize>().map_err(|e| e.to_string())?;
                declared_vertices = tokens[4].parse::<usize>().map_err(|e| e.to_string())?;
                bags = Vec::with_capacity(num_bags);
                adj = vec![Vec::new(); num_bags];
                saw_solution_line = true;
            }
            "b" => {
                // "b <bag_id> <v1> <v2> ..."  — all 1-indexed
                if tokens.len() < 2 {
                    return Err(format!("Malformed bag line: {}", line));
                }
                if !saw_solution_line {
                    return Err(format!("bag line before the solution line: {}", line));
                }
                let bag_id = to_zero_based(
                    "bag id",
                    tokens[1].parse::<usize>().map_err(|e| e.to_string())?,
                    num_bags,
                )?;
                let vertices: Vec<u32> = tokens[2..]
                    .iter()
                    .map(|t| {
                        let v = t
                            .parse::<usize>()
                            .map_err(|e: std::num::ParseIntError| e.to_string())?;
                        to_zero_based("vertex", v, declared_vertices).map(|v| v as u32)
                    })
                    .collect::<Result<_, String>>()?;
                bags.push(TdBag {
                    id: bag_id,
                    vertices,
                });
            }
            _ => {
                // Tree edge: two bare integers "<bag_id1> <bag_id2>" (1-indexed)
                if tokens.len() == 2
                    && let (Ok(a), Ok(b)) = (tokens[0].parse::<usize>(), tokens[1].parse::<usize>())
                {
                    let a = to_zero_based("bag id", a, num_bags)?;
                    let b = to_zero_based("bag id", b, num_bags)?;
                    adj[a].push(b);
                    adj[b].push(a);
                }
            }
        }
    }

    if bags.is_empty() {
        return Err("No bags found in tree decomposition output".to_string());
    }

    // Sort bags by id so indexing is consistent
    bags.sort_by_key(|b| b.id);

    // `adj` is sized from the solution line and `bags` from the bag lines, so
    // the two are co-indexed — which every consumer reads them as — only when
    // the file defines each declared bag exactly once. A short or repeated bag
    // list would otherwise hand one bag another bag's neighbours.
    if bags.len() != num_bags {
        return Err(format!(
            "the solution line declares {num_bags} bags but the file defines {}",
            bags.len()
        ));
    }
    if let Some((_, bag)) = bags.iter().enumerate().find(|(i, b)| b.id != *i) {
        return Err(format!(
            "bag {} is defined more than once; each of the {num_bags} declared bags is \
             defined once",
            bag.id + 1
        ));
    }

    Ok(TreeDecomposition {
        kind,
        bags,
        adj,
        num_vars,
    })
}
