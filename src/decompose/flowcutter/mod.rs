//! In-process FlowCutter backend: tree-decomposition construction
//! (`td_compute*`) through the vendored C++ library at `vendor/treedecomp/`.
//!
//! The other half of FlowCutter — one top-level balanced separator, with no
//! decomposition around it — is the pure-Rust port in
//! [`super::flowcutter_rs`]; nothing here crosses into it.
//!
//! **Determinism:** with `timeout_ms = 0` and `patience_ms = 0` this path is
//! deterministic for a given graph (no wall-clock dependence).
//! Wall-clock-based termination (nonzero `timeout_ms` or `patience_ms`)
//! introduces non-determinism because different runs may complete different
//! numbers of iterations before the deadline. A [`WallCapMode::BoundOnly`] cap
//! is the exception: it changes nothing about the search, so a build that
//! finishes well inside such a cap is as deterministic as the untimed one.
//!
//! A build inside a metered construction is the other exception. It is handed a
//! budget in work units rather than left to its clock ([`TdHandle::compute`]),
//! and a nonzero unit budget stands the deadline and the patience check down, so
//! where the search stops is decided by the work it has done and not by how long
//! that work took.
//!
//! # Safety
//!
//! Every `unsafe` call below goes into the vendored builder, so they share one
//! argument; the per-site comments add only what is specific to a call.
//!
//! * **The handle.** [`TdHandle`] wraps what one of the `td_compute*` entry
//!   points returned, and [`TdHandle::compute`] is the only way to build one —
//!   it rejects a null before wrapping, so every readback has a live result. It
//!   is released exactly once, in `Drop`; the type is neither `Copy` nor
//!   `Clone`, so no second owner can free it or read through it afterwards, and
//!   its raw pointer field keeps the type off `Send`/`Sync`, so the handle
//!   stays on the thread that built it.
//! * **The edge buffer** is described by a pointer and a count taken from the
//!   same live slice in the same expression, and holds exactly two `c_int`s per
//!   edge. The C side reads `edges[2*i]` and `edges[2*i+1]` for `i` below the
//!   count it is given, so every read stays inside the buffer — a count that
//!   did not fit in the `c_int` parameter can only ask for fewer edges than are
//!   there, never more. The builder copies what it needs during the call and
//!   keeps no pointer to it, so nothing has to outlive the call.
//! * **The counts a build reports back** travel through the two `*mut i64`
//!   out-parameters of the `td_compute*` entry points. They are `&mut i64`
//!   locals of the single caller, [`TdHandle::compute`], live for the whole
//!   call and borrowed nowhere else; the builder writes each at most once and
//!   reads neither, so a build that returns before its search starts leaves
//!   them at the zero the caller initialised them to, which is the right count
//!   for that case.
//! * **Buffers read back** are allocated with exactly the length the matching
//!   size query returned an instant earlier, which is what the C API asks of
//!   the caller, and every bag index handed to a readback is below the bag
//!   count the same handle just reported. Nothing in between can change either,
//!   because the readback calls only borrow the decomposition. The counts are
//!   C++ container sizes, so they are non-negative and the casts preserve them.

use std::os::raw::c_int;

use crate::cnf::CnfFormula;

use super::*;

mod treedecomp_ffi {
    use std::os::raw::c_int;

    #[repr(C)]
    pub(super) struct TdResult {
        _private: [u8; 0],
    }

    // SAFETY: hand-written mirror of the C declarations in
    // `vendor/treedecomp/ffi.h`, which is vendored here and compiled by
    // `build.rs` from that same header — the two sides move together. The
    // `td_compute*` entry points hand back ownership of the result, which the
    // caller returns through `td_free`; the readback calls borrow it and copy
    // into caller-provided buffers, so nothing they return has to be freed.
    // The header's `int` is `c_int` here; its `int64_t` is `i64`, which is that
    // type on every target rather than a C spelling of it.
    unsafe extern "C" {
        pub(super) fn td_compute(
            num_nodes: c_int,
            num_edges: c_int,
            edges: *const c_int,
            steps: i64,
            iters: c_int,
            iters_done: *mut i64,
            greedy_touches: *mut i64,
            unit_budget: i64,
            units_per_iter: i64,
        ) -> *mut TdResult;
        pub(super) fn td_compute_timed_patience(
            num_nodes: c_int,
            num_edges: c_int,
            edges: *const c_int,
            steps: i64,
            iters: c_int,
            timeout_ms: i64,
            patience_ms: i64,
            tight_gates: c_int,
            iters_done: *mut i64,
            greedy_touches: *mut i64,
            unit_budget: i64,
            units_per_iter: i64,
        ) -> *mut TdResult;
        pub(super) fn td_num_bags(td: *const TdResult) -> c_int;
        pub(super) fn td_bag_size(td: *const TdResult, bag_idx: c_int) -> c_int;
        pub(super) fn td_bag_vertices(td: *const TdResult, bag_idx: c_int, out: *mut c_int);
        pub(super) fn td_bag_num_neighbors(td: *const TdResult, bag_idx: c_int) -> c_int;
        pub(super) fn td_bag_neighbors(td: *const TdResult, bag_idx: c_int, out: *mut c_int);
        pub(super) fn td_free(td: *mut TdResult);
    }
}

/// The vendored builder's result: the one foreign resource this crate owns,
/// and the only thing here that has to be released by hand.
///
/// A handle exists only if the builder produced one (§ Safety), so nothing
/// downstream re-checks for a null, and it is freed by going out of scope
/// rather than at a `td_free` some path can miss.
struct TdHandle(*mut treedecomp_ffi::TdResult);

impl Drop for TdHandle {
    fn drop(&mut self) {
        // SAFETY: live handle (§ Safety), freed exactly here.
        unsafe { treedecomp_ffi::td_free(self.0) };
    }
}

impl TdHandle {
    /// Decompose the graph `flat_edges` describes over `num_vertices` vertices,
    /// under `budget`. `None` when no decomposition came back — either the
    /// builder produced none, or the build was declined before it started
    /// because the share it was given could not pay for its setup.
    ///
    /// `flat_edges` holds each edge as two consecutive endpoints.
    ///
    /// This is also where a FlowCutter build meets the construction meter
    /// ([`crate::decompose::meter`]): the build is handed a budget in work
    /// units, and what it reports having spent is charged back afterwards by
    /// [`charge_fc_build`].
    fn compute(num_vertices: u32, flat_edges: &[c_int], budget: FcBudget) -> Option<Self> {
        let num_edges = flat_edges.len() / 2;
        let nodes = u64::from(num_vertices);
        let elem = fc_elements(nodes, num_edges as u64);

        // The build's budget in work rather than in wall: a FlowCutter build's
        // unit budget is its wall cap converted at the meter's rate, and only
        // while the meter is armed. Under a deterministic budget `timeout_ms` is
        // ITSELF a work-clock quantity — whoever derived it measured it against
        // `meter::now`, which is the work clock for as long as the meter is
        // armed — so converting it back into units at the same rate recovers the
        // count it was cut from instead of estimating a fresh one.
        //
        // Zero is the "unbudgeted" sentinel the vendored library reads as "the
        // clock and the step budget decide". That is what a step-budgeted build
        // passes, and what every build of an unmetered construction passes, so
        // both search exactly as they did before the meter existed.
        let unit_budget: i64 = match budget {
            FcBudget::Timed { timeout_ms, .. } if crate::decompose::meter::metering() => {
                let wall_ms = timeout_ms.max(0) as u64;
                let units = wall_ms.saturating_mul(crate::decompose::meter::UNITS_PER_MS);
                i64::try_from(units).unwrap_or(i64::MAX)
            }
            _ => 0,
        };

        // A build whose share cannot even pay for its setup is DECLINED rather
        // than started. The setup price is known before the call, and a build
        // that cannot afford it would otherwise search until the wall cap cut it
        // off, whatever its budget said — and declining is a decision the work
        // clock can make, where being cut off by the wall is not. The edge list
        // was constructed either way, so that one pass is charged.
        let setup = FC_SETUP_UNITS_PER_ELEM.saturating_mul(elem);
        if unit_budget > 0 && setup >= unit_budget as u64 {
            crate::decompose::meter::charge(FC_DECLINE_UNITS_PER_ELEM.saturating_mul(elem));
            return None;
        }

        // What the restart loop gets is the remainder after setup, priced per
        // iteration; the library holds the loop to it from the inside.
        let loop_units = (unit_budget.max(0) as u64).saturating_sub(setup);
        let loop_budget = i64::try_from(loop_units).unwrap_or(i64::MAX);
        let iter_units = i64::try_from(fc_iter_units(nodes, num_edges as u64)).unwrap_or(i64::MAX);

        // Written by the builder, charged below. They stay at zero when it
        // returns before the restart loop runs, which is the right charge for
        // that build.
        let mut iters_done: i64 = 0;
        let mut greedy_touches: i64 = 0;

        let raw = match budget {
            FcBudget::Timed {
                timeout_ms,
                patience_ms,
                iters,
                steps,
                cap_mode,
            } => {
                let steps = match cap_mode {
                    WallCapMode::Tight => steps,
                    WallCapMode::BoundOnly => scaled_steps(steps, num_edges),
                };
                // SAFETY: edge buffer and count out-params (§ Safety); the
                // returned handle becomes ours.
                unsafe {
                    treedecomp_ffi::td_compute_timed_patience(
                        num_vertices as c_int,
                        num_edges as c_int,
                        flat_edges.as_ptr(),
                        steps,
                        iters,
                        timeout_ms,
                        patience_ms,
                        cap_mode.as_ffi(),
                        &mut iters_done,
                        &mut greedy_touches,
                        loop_budget,
                        iter_units,
                    )
                }
            }
            FcBudget::Steps { steps, iters } => {
                // SAFETY: as the timed call above (§ Safety).
                unsafe {
                    treedecomp_ffi::td_compute(
                        num_vertices as c_int,
                        num_edges as c_int,
                        flat_edges.as_ptr(),
                        scaled_steps(steps, num_edges),
                        iters,
                        &mut iters_done,
                        &mut greedy_touches,
                        loop_budget,
                        iter_units,
                    )
                }
            }
        };

        // The work was done whether or not a decomposition came back, so the
        // charge lands on both arms and on a null result alike.
        charge_fc_build(nodes, num_edges as u64, iters_done, greedy_touches);

        (!raw.is_null()).then_some(TdHandle(raw))
    }
}

/// **WHAT THE SETUP OF A FLOWCUTTER BUILD COSTS THE CONSTRUCTION METER**, per
/// graph element: the passes that read the edge list into the vendored
/// library's structures before any searching happens, which touch every vertex
/// and every arc a fixed number of times. An element is what [`fc_elements`]
/// counts.
///
/// It is also the price of admission [`TdHandle::compute`] compares a build's
/// whole share against, because a build that cannot pay for its setup cannot
/// run a search either.
///
/// The number is a fit rather than a derivation, and it sits at the pessimistic
/// end of the band it was fitted over deliberately. The point of a work clock is
/// to stop a build before the wall-clock cap has to, and a constant fitted to
/// the middle of the population leaves half of that population stopped by the
/// wall instead — which is the machine-dependence the meter exists to remove.
const FC_SETUP_UNITS_PER_ELEM: u64 = 6_000;

/// What one completed restart iteration costs, charged on
/// `arcs · floor(sqrt(nodes))` rather than on elements.
///
/// An iteration is a max-flow, and its cost is not linear in the graph. Over 18
/// builds spanning 91 to 13 564 nodes, cost per arc per iteration rose about
/// fivefold with node count, so charging an iteration per element under-charged
/// the largest builds nearly threefold. `arcs · sqrt(nodes)` is the
/// unit-capacity max-flow bound the search is actually paying, and charging on
/// it flattens the residual:
///
/// | measure | charged/measured spread over the 18 builds | worst under-charge |
/// |---|---|---|
/// | per element | 0.37 – 2.65 | 2.74× |
/// | `arcs · sqrt(nodes)` | 0.69 – 2.28 | 1.44× |
///
/// At this value 2 of the 18 builds are still under-charged and none by more
/// than 1.44×, against 7 of 18 and 2.74× on the per-element measure. Like
/// [`FC_SETUP_UNITS_PER_ELEM`] it is placed on the expensive side of the fitted
/// band on purpose.
const FC_ITER_UNITS_PER_FLOW: u64 = 50;

/// What a DECLINED build costs, per element: the edge list was constructed
/// before the decline, and that is one linear pass. Measured at about 18 units
/// per element on the incidence graph that motivated the decline — 300 459
/// elements built in 7 ms — and rounded up.
///
/// Charging the full setup for a build that never started would bill a candidate
/// for a search it did not run, and starve the candidates behind it of what that
/// bill consumed.
const FC_DECLINE_UNITS_PER_ELEM: u64 = 20;

/// Graph elements a build touches: every vertex, and every arc in both
/// directions. The measure [`FC_SETUP_UNITS_PER_ELEM`] and
/// [`FC_DECLINE_UNITS_PER_ELEM`] are denominated in.
fn fc_elements(nodes: u64, num_edges: u64) -> u64 {
    nodes.saturating_add(num_edges.saturating_mul(2))
}

/// What one restart iteration over this graph costs the meter.
///
/// Shared with the pure-Rust separator port in [`super::flowcutter_rs`], whose
/// outer loop makes the same pass over the same graph: one cost model for both
/// FlowCutter implementations, with no second constant to keep in step with this
/// one.
pub(super) fn fc_iter_units(nodes: u64, num_edges: u64) -> u64 {
    let arcs = num_edges.saturating_mul(2);
    // Integer square root, not the floating-point one. This feeds a budget whose
    // whole purpose is to answer identically on every machine and in every run.
    FC_ITER_UNITS_PER_FLOW
        .saturating_mul(arcs)
        .saturating_mul(nodes.isqrt())
}

/// **CHARGE A FINISHED FLOWCUTTER BUILD TO THE CONSTRUCTION METER**, from the
/// counts the build itself reports.
///
/// Nothing here models the vendored library's control flow. Two quantities come
/// back across the FFI and each is charged for what it did:
///
/// * `iters_done` — restart iterations the loop actually completed. A build cut
///   short by its budget is charged for the part it ran, and one that stops
///   early is not charged for a search it never did.
/// * `greedy_touches` — neighbourhood entries swept by the greedy pre-orderings,
///   which run ahead of the restart loop and spend none of its budget. That
///   count is already in the meter's unit, so it is charged one for one with no
///   constant in between.
///
/// The node-count and degree gates deciding whether each pre-ordering runs stay
/// in the library, rather than being mirrored here where the mirror could only
/// drift.
///
/// Charged after the call, necessarily — which does not weaken the bound,
/// because the charge is not the bound. The bound is the unit budget the call
/// was given, which the library enforces from the inside.
fn charge_fc_build(nodes: u64, num_edges: u64, iters_done: i64, greedy_touches: i64) {
    if !crate::decompose::meter::metering() {
        return;
    }
    let setup = FC_SETUP_UNITS_PER_ELEM.saturating_mul(fc_elements(nodes, num_edges));
    let search = (iters_done.max(0) as u64).saturating_mul(fc_iter_units(nodes, num_edges));
    let units = setup
        .saturating_add(search)
        .saturating_add(greedy_touches.max(0) as u64);
    crate::decompose::meter::charge(units);
}

/// The step budget scaled to graph size: small graphs converge quickly and do
/// not need 1M+ steps.
///
/// Dropped only under a [`WallCapMode::Tight`] wall, where the clock stops the
/// search long before the ceiling matters. A bound-only wall keeps the clamp,
/// so arming such a wall leaves the step budget the untimed one.
fn scaled_steps(steps: i64, num_edges: usize) -> i64 {
    steps.min(10_000i64.max(50 * num_edges as i64))
}

/// What a non-zero FlowCutter wall cap MEANS, which is orthogonal to how large
/// it is.
///
/// The vendored library's timed entry is not a superset of its step-budgeted
/// one: under a deadline it also tightens the pre-loop heuristic node gates
/// (min-degree 50 000 → 2 000, min-shortcut 10 000 → 1 000) and drops the step
/// clamp. Those land whether or not the cap is ever reached, so keying them on
/// the mere presence of a deadline makes every deadline-armed build search less
/// patiently, in service of bounding the few that overrun.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WallCapMode {
    /// The cap is an outer bound the caller does not expect to reach. The search
    /// keeps its untimed gates and step clamp, so the decompositions it
    /// considers are exactly the unbounded ones and the deadline only stops it
    /// once the wall has genuinely passed.
    BoundOnly,
    /// The cap is expected to bite: a short window in which finishing matters
    /// more than searching. The pre-loop heuristics take their tight gates,
    /// because a single ordering pass on a large graph can consume the whole
    /// window.
    Tight,
}

impl WallCapMode {
    /// FFI encoding: nonzero = tight.
    fn as_ffi(self) -> c_int {
        match self {
            WallCapMode::BoundOnly => 0,
            WallCapMode::Tight => 1,
        }
    }
}

/// How hard one FlowCutter run looks for a decomposition — one variant per mode
/// the search has, so a caller cannot name a wall clock and a step budget that
/// contradict each other.
#[derive(Clone, Copy)]
pub(crate) enum FcBudget {
    /// Stop on the wall clock.
    Timed {
        /// Wall-clock limit in milliseconds.
        timeout_ms: i64,
        /// Milliseconds without a treewidth improvement before giving up early;
        /// `0` waits out the whole clock.
        patience_ms: i64,
        /// Source/sink pairs tried per level.
        iters: i32,
        /// Ceiling on the search the clock is meant to reach first:
        /// [`FC_TIMED_STEPS`] for a run whose only limit is time, and the count
        /// it was going to spend anyway for a caller capping a run mid-build.
        steps: i64,
        /// Whether `timeout_ms` is expected to bite. See [`WallCapMode`].
        cap_mode: WallCapMode,
    },
    /// Stop after a fixed number of computation steps, which is what makes the
    /// run deterministic (see the module docs).
    Steps {
        /// Computation-step budget for the search, scaled down to the graph in
        /// [`TdHandle::compute`].
        steps: i64,
        /// Source/sink pairs tried per level.
        iters: i32,
    },
}

/// The step ceiling a purely timed run carries — high enough that the wall
/// clock is what ends the search.
const FC_TIMED_STEPS: i64 = 1_000_000;

/// The wall clock a bare `flowcutter-*` spec means.
pub(crate) const FC_BARE_TIMEOUT_MS: i64 = 200;
/// Patience for a bare `flowcutter-*` spec, whose clock is short enough that it
/// gives up on a stalled search sooner than a spec that names its own timeout.
pub(crate) const FC_PATIENCE_MS_BARE: i64 = 100;
/// Patience for a spec that names a timeout but no patience.
pub(crate) const FC_PATIENCE_MS_PARAMETRIZED: i64 = 150;
/// Source/sink pairs a timed spec tries per level when it names no count.
pub(crate) const FC_DEFAULT_ITERS: i32 = 100_000;
/// Source/sink pairs a step-budgeted spec tries per level when it names no
/// count — far below the timed default, because nothing stops the search early.
pub(crate) const FC_DEFAULT_STEPS_ITERS: i32 = 900;

impl FcBudget {
    /// The budget a timed spec asks for: the clock decides, so the step ceiling
    /// is the one it will not reach.
    ///
    /// A timeout that is not positive is no clock at all, and what it names is
    /// the step-budgeted search — the tree that spelling has always built.
    ///
    /// The clock this constructor is given is the one the caller expects to run
    /// out, so it is a [`WallCapMode::Tight`] one. A caller arming a wall it
    /// does not expect to reach builds [`FcBudget::Timed`] directly and says so.
    pub(crate) const fn timed(timeout_ms: i64, patience_ms: i64, iters: i32) -> Self {
        if timeout_ms > 0 {
            FcBudget::Timed {
                timeout_ms,
                patience_ms,
                iters,
                steps: FC_TIMED_STEPS,
                cap_mode: WallCapMode::Tight,
            }
        } else {
            FcBudget::Steps {
                steps: FC_TIMED_STEPS,
                iters,
            }
        }
    }
}

/// THE FlowCutter construction: decompose one graph view of `formula` at
/// `budget`, then read a vtree off the result the way `request` says.
///
/// Every `--vtree` spec in the FlowCutter family lands here. The spec grammar
/// varies exactly three things — which graph, how hard to look, how to read the
/// result — so those are the three arguments, and there is nothing left for a
/// per-spec wrapper to encode in its name.
pub(crate) fn flowcutter_vtree(
    formula: &CnfFormula,
    kind: GraphKind,
    budget: FcBudget,
    request: ConversionRequest<'_>,
) -> Result<TdConversion, String> {
    let td = flowcutter_td(formula, kind, budget)?;
    Ok(convert_td(formula, &td, request))
}

/// Refuse a graph the vendored builder cannot be handed at all.
///
/// `TWD::Graph::init` allocates an n×n bitset adjacency matrix, and the vendored
/// `Bitset` constructor does not check its allocation, so an oversized graph is
/// a SIGSEGV inside the FFI rather than an error anything here could return.
/// Checked before the edge list is built, so a formula that cannot be
/// decomposed this way costs nothing to reject.
fn vendor_size_guard(kind: GraphKind, formula: &CnfFormula) -> Result<(), String> {
    match kind {
        GraphKind::Primal => {
            // At 1.1M variables the matrix is ~155 GB, which exhausts the
            // address space. 500K matches PORTFOLIO_HEAVY_MAX_VARS in
            // portfolio.rs; the matrix at n=500K is 29 GiB, fitting under the
            // 32 GiB address-space limit.
            const MAX_PRIMAL_VERTICES: u32 = 500_000;
            let num_vars = formula.num_vars;
            if num_vars > MAX_PRIMAL_VERTICES {
                return Err(format!(
                    "primal graph too large ({} vertices; n×n adjacency matrix would exceed memory)",
                    num_vars
                ));
            }

            // Even when vertex count fits, the primal graph can be too dense: a single
            // 4258-wide clause contributes C(4258,2) ≈ 9 M raw pair-edges, and a
            // formula with millions of such clauses runs to hundreds of millions.
            // The C++ td_compute aborts via std::bad_alloc on inputs this dense and
            // the exception propagates through `extern "C"` as `terminate()`, killing
            // the whole process before flowcutter-primal's `Err` can be returned. Estimate the
            // raw pair count (without materialising it) and bail before we allocate
            // the edge list. 20 M is conservative — flowcutter-primal use in portfolio today
            // runs on ≤ 30k-vertex / ≤ 100k-clause sub-components where the raw count
            // is < 1 M.
            const MAX_PRIMAL_RAW_EDGES: u64 = 20_000_000;
            let raw_edges: u64 = formula
                .clauses
                .iter()
                .map(|c| {
                    let w = c.literals.len() as u64;
                    w.saturating_mul(w.saturating_sub(1)) / 2
                })
                .sum();
            if raw_edges > MAX_PRIMAL_RAW_EDGES {
                return Err(format!(
                    "primal graph too dense (~{raw_edges} raw pair-edges; C++ td_compute would exceed memory budget)"
                ));
            }
            Ok(())
        }
        GraphKind::Incidence => {
            // A large incidence graph (666k nodes → 55 TB) overflows RLIMIT_AS.
            const MAX_INCIDENCE_VERTICES: u32 = 100_000;
            let total_vertices = formula.num_vars + formula.clauses.len() as u32;
            if total_vertices > MAX_INCIDENCE_VERTICES {
                return Err(format!(
                    "incidence graph too large ({} vertices; n×n adjacency matrix would exceed memory)",
                    total_vertices
                ));
            }
            Ok(())
        }
    }
}

/// Run the vendored C++ FlowCutter over one graph view of `formula` and return
/// the raw decomposition.
///
/// The single place this crate calls into `vendor/treedecomp/`: both graph views
/// come through here, and everything that distinguishes them is settled by
/// `kind` before the call. Replacing the vendored builder with a port therefore
/// means replacing one function body, not auditing every backend that wanted a
/// decomposition.
///
/// Bags come back over `formula`'s variables in both views: an incidence run
/// decomposes clause vertices too, but the extracted decomposition is labelled
/// with `formula.num_vars`, exactly as the primal one is.
pub(crate) fn flowcutter_td(
    formula: &CnfFormula,
    kind: GraphKind,
    budget: FcBudget,
) -> Result<TreeDecomposition, String> {
    vendor_size_guard(kind, formula)?;
    let PaceGraph {
        num_vertices: total_vertices,
        edges,
        ..
    } = kind.build(formula);

    let mut flat_edges: Vec<c_int> = Vec::with_capacity(edges.len() * 2);
    for &(u, v) in &edges {
        flat_edges.push(u as c_int);
        flat_edges.push(v as c_int);
    }

    let td = TdHandle::compute(total_vertices, &flat_edges, budget)
        .ok_or_else(|| "td_compute returned null".to_string())?;
    extract_td(&td, kind, formula.num_vars)
}

/// Copy a result out of the vendored builder into the crate's own type,
/// labelled with the view it decomposed and over `num_vars` variables.
fn extract_td(td: &TdHandle, kind: GraphKind, num_vars: u32) -> Result<TreeDecomposition, String> {
    use treedecomp_ffi::*;

    // SAFETY: live handle (§ Safety), read back under the probe-then-fill
    // protocol described there; every call below only borrows it.
    unsafe {
        let n = td_num_bags(td.0) as usize;
        if n == 0 {
            return Err("empty tree decomposition".into());
        }

        let mut bags: Vec<TdBag> = Vec::with_capacity(n);
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];

        for i in 0..n {
            let bag_sz = td_bag_size(td.0, i as c_int) as usize;
            let mut vertices = vec![0 as c_int; bag_sz];
            td_bag_vertices(td.0, i as c_int, vertices.as_mut_ptr());
            bags.push(TdBag {
                id: i,
                vertices: vertices.iter().map(|&v| v as u32).collect(),
            });

            let num_nb = td_bag_num_neighbors(td.0, i as c_int) as usize;
            let mut neighbors = vec![0 as c_int; num_nb];
            td_bag_neighbors(td.0, i as c_int, neighbors.as_mut_ptr());
            for &nb in &neighbors {
                let nb = nb as usize;
                if nb > i {
                    adj[i].push(nb);
                    adj[nb].push(i);
                }
            }
        }

        Ok(TreeDecomposition {
            kind,
            bags,
            adj,
            num_vars,
        })
    }
}

#[cfg(test)]
mod tests;
