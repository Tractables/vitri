// C FFI wrapper for meelgroup/treedecomp (in-process FlowCutter).
#ifndef TREEDECOMP_FFI_H
#define TREEDECOMP_FFI_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// Opaque handle to a tree decomposition result.
typedef struct TdResult TdResult;

// Run FlowCutter on a graph with `num_nodes` nodes.
// `edges` is a flat array of [u0,v0, u1,v1, ...] (0-indexed), length = 2*num_edges.
// `steps` controls the computation budget (higher = more time, better quality).
// `iters` controls the number of FlowCutter iterations.
// Returns a TdResult handle (caller must free with td_free).
//
// The last four arguments meter the construction. `iters_done` (may be NULL)
// receives the restart iterations the loop actually consumed and
// `greedy_touches` (may be NULL) the graph elements the greedy pre-passes swept,
// so the caller charges measured work rather than a modelled estimate of it.
// `unit_budget` bounds the whole construction in the unit `greedy_touches` is
// counted in, and `units_per_iter` is what one restart iteration costs in that
// unit; `unit_budget = 0` arms no budget.
TdResult* td_compute(int num_nodes, int num_edges,
                     const int* edges, int64_t steps, int iters,
                     int64_t* iters_done, int64_t* greedy_touches,
                     int64_t unit_budget, int64_t units_per_iter);

// Like td_compute but with a wall-clock timeout in milliseconds.
// The computation stops when either the step/iter budget is exhausted or
// the timeout expires, whichever comes first. timeout_ms=0 means no timeout.
TdResult* td_compute_timed(int num_nodes, int num_edges,
                           const int* edges, int64_t steps, int iters,
                           int64_t timeout_ms);

// Like td_compute_timed but with early convergence detection.
// Stops early if the treewidth hasn't improved for `patience_ms` milliseconds.
// patience_ms=0 means no early stopping (behaves like td_compute_timed).
//
// `tight_gates` says whether the deadline is expected to BITE (nonzero) or is
// only an outer bound (0), and is independent of how large `timeout_ms` is. A
// bound-only deadline keeps the untimed pre-loop heuristic gates and the
// step-count clamp, so arming a generous wall does not change which
// decompositions the search considers — it only stops the search once the wall
// has passed. Pass nonzero when the deadline is small enough that a single
// unbounded ordering pass could consume it whole.
//
// The last four arguments mean what they mean for td_compute. A nonzero
// `unit_budget` also stands the deadline and the patience check down, so that
// the work budget alone decides where the search stops.
TdResult* td_compute_timed_patience(int num_nodes, int num_edges,
                                    const int* edges, int64_t steps, int iters,
                                    int64_t timeout_ms, int64_t patience_ms,
                                    int tight_gates, int64_t* iters_done,
                                    int64_t* greedy_touches,
                                    int64_t unit_budget, int64_t units_per_iter);

// Get the number of bags in the tree decomposition.
int td_num_bags(const TdResult* td);

// Get the treewidth (max bag size - 1).
int td_width(const TdResult* td);

// Get the size of bag `bag_idx`.
int td_bag_size(const TdResult* td, int bag_idx);

// Copy the vertices of bag `bag_idx` into `out` (must have room for td_bag_size elements).
// Vertices are 0-indexed.
void td_bag_vertices(const TdResult* td, int bag_idx, int* out);

// Get the number of neighbors of bag `bag_idx` in the TD tree.
int td_bag_num_neighbors(const TdResult* td, int bag_idx);

// Copy the neighbor bag indices of bag `bag_idx` into `out`.
void td_bag_neighbors(const TdResult* td, int bag_idx, int* out);

// Free a TdResult.
void td_free(TdResult* td);

// Self-test of the vendored FlowCutter k-way id-heap: push/pop max-ordering,
// contains/get_key, and — the bug #18 regression — that the child-index
// arithmetic (k*pos+1) does not overflow signed int32 at large heap positions.
// Returns 0 on success, or a nonzero check id identifying the first failing
// assertion (see ffi.cpp). Runs in milliseconds and allocates only modest memory.
int treedecomp_heap_selftest(void);

#ifdef __cplusplus
}
#endif

#endif
