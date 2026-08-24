// C FFI wrapper for meelgroup/treedecomp (in-process FlowCutter).
#include <cstddef>  // vitri: size_t — not guaranteed transitively on all stdlibs
#include <cstdint>  // vitri: int64_t
#include "ffi.h"
#include "TreeDecomposition.hpp"
#include "IFlowCutter.hpp"

struct TdResult {
    TWD::TreeDecomposition td;
    // Cache adjacency as vectors since TWD::Graph stores adj_list as protected
    std::vector<std::vector<int>> adj;
};

extern "C" {

static TdResult* td_compute_impl(int num_nodes, int num_edges,
                                  const int* edges, int64_t steps, int iters,
                                  int64_t timeout_ms, int64_t* iters_done,
                                  int64_t* greedy_touches, int64_t unit_budget,
                                  int64_t units_per_iter) {
    // Build the TWD::Graph
    TWD::Graph g(num_nodes);
    for (int i = 0; i < num_edges; i++) {
        int u = edges[2 * i];
        int v = edges[2 * i + 1];
        g.addEdge(u, v);
    }

    // Run FlowCutter
    TWD::IFlowCutter fc(num_nodes, num_edges, /*verb=*/0);
    fc.importGraph(g);
    TWD::TreeDecomposition td = (timeout_ms > 0)
        ? fc.constructTD_timed(steps, iters, timeout_ms, iters_done, greedy_touches, unit_budget, units_per_iter)
        : fc.constructTD(steps, iters, iters_done, greedy_touches, unit_budget, units_per_iter);

    auto* result = new TdResult();
    result->td = std::move(td);

    // Cache adjacency list for bag neighbors
    int nb = result->td.numNodes();
    result->adj.resize(nb);
    for (int i = 0; i < nb; i++) {
        result->adj[i] = result->td.Neighbors(i);
    }

    return result;
}

TdResult* td_compute(int num_nodes, int num_edges,
                     const int* edges, int64_t steps, int iters,
                     int64_t* iters_done, int64_t* greedy_touches,
                     int64_t unit_budget, int64_t units_per_iter) {
    return td_compute_impl(num_nodes, num_edges, edges, steps, iters, 0,
                           iters_done, greedy_touches, unit_budget, units_per_iter);
}

TdResult* td_compute_timed(int num_nodes, int num_edges,
                           const int* edges, int64_t steps, int iters,
                           int64_t timeout_ms) {
    return td_compute_impl(num_nodes, num_edges, edges, steps, iters, timeout_ms,
                           nullptr, nullptr, 0, 0);
}

TdResult* td_compute_timed_patience(int num_nodes, int num_edges,
                                    const int* edges, int64_t steps, int iters,
                                    int64_t timeout_ms, int64_t patience_ms,
                                    int tight_gates, int64_t* iters_done,
                                    int64_t* greedy_touches,
                                    int64_t unit_budget, int64_t units_per_iter) {
    TWD::Graph g(num_nodes);
    for (int i = 0; i < num_edges; i++) {
        int u = edges[2 * i];
        int v = edges[2 * i + 1];
        g.addEdge(u, v);
    }

    TWD::IFlowCutter fc(num_nodes, num_edges, /*verb=*/0);
    fc.importGraph(g);
    TWD::TreeDecomposition td = fc.constructTD_timed_patience(
        steps, iters, timeout_ms, patience_ms, tight_gates != 0, iters_done,
        greedy_touches, unit_budget, units_per_iter);

    auto* result = new TdResult();
    result->td = std::move(td);

    int nb = result->td.numNodes();
    result->adj.resize(nb);
    for (int i = 0; i < nb; i++) {
        result->adj[i] = result->td.Neighbors(i);
    }

    return result;
}

int td_num_bags(const TdResult* td) {
    return td->td.numNodes();
}

int td_width(const TdResult* td) {
    return td->td.width();
}

int td_bag_size(const TdResult* td, int bag_idx) {
    // Bags() is non-const in TWD, but we stored the result — cast away const
    auto& bags = const_cast<TWD::TreeDecomposition&>(td->td).Bags();
    return static_cast<int>(bags[bag_idx].size());
}

void td_bag_vertices(const TdResult* td, int bag_idx, int* out) {
    auto& bags = const_cast<TWD::TreeDecomposition&>(td->td).Bags();
    const auto& bag = bags[bag_idx];
    for (size_t i = 0; i < bag.size(); i++) {
        out[i] = bag[i];
    }
}

int td_bag_num_neighbors(const TdResult* td, int bag_idx) {
    return static_cast<int>(td->adj[bag_idx].size());
}

void td_bag_neighbors(const TdResult* td, int bag_idx, int* out) {
    const auto& nb = td->adj[bag_idx];
    for (size_t i = 0; i < nb.size(); i++) {
        out[i] = nb[i];
    }
}

void td_free(TdResult* td) {
    delete td;
}

} // extern "C"
