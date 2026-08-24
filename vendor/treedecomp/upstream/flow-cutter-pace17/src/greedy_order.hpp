#ifndef GREEDY_ORDER_H
#define GREEDY_ORDER_H

#include "array_id_func.hpp"
#include <chrono>  // vitri: wall-clock abandonment (see below)
#include <cstdint>  // vitri: int64_t — not guaranteed transitively on all stdlibs

// vitri: both passes accept an optional ABANDONMENT DEADLINE.
//
// They were previously the only unbounded units in a timed constructTD: the
// caller checks the clock before starting one, but a pass already under way runs
// to completion however long it takes. A wall-clock bound on the surrounding
// search cannot bind while these can overrun it.
//
// CONTRACT — all or nothing. A pass that reaches its deadline returns an EMPTY
// ArrayIDIDFunc (`preimage_count() == 0`) and the caller skips it, exactly as if
// it had been gated out before starting. A partially filled order is never
// returned: it is not a permutation, and feeding one to test_new_order would
// produce a decomposition over a subset of the nodes. Callers must check
// `preimage_count() != 0` before using the result.
//
// The default `time_point::max()` means no deadline, so untimed callers are
// unchanged.
//
// Granularity is ONE node contraction: the clock is consulted at the top of each
// elimination step, so a single very expensive contraction still runs to
// completion.
//
// vitri: both passes also accept an optional TOUCH BUDGET, counted in graph
// elements swept rather than in time. A pass that spends its budget is abandoned
// under the same all-or-nothing contract as one that reaches its deadline. A
// budget stops a pass at the same point on every run, which a deadline cannot
// do. `0` means no budget, so callers that do not meter their work are
// unchanged.
ArrayIDIDFunc compute_greedy_min_degree_order(
	const ArrayIDIDFunc&tail, const ArrayIDIDFunc&head,
	std::chrono::steady_clock::time_point deadline = std::chrono::steady_clock::time_point::max(),
	int64_t touch_budget = 0);
ArrayIDIDFunc compute_greedy_min_shortcut_order(
	const ArrayIDIDFunc&tail, const ArrayIDIDFunc&head,
	std::chrono::steady_clock::time_point deadline = std::chrono::steady_clock::time_point::max(),
	int64_t touch_budget = 0);

// vitri: read and reset the touch counter the two passes accumulate into. The
// definition in greedy_order.cpp says what a touch is and where one is counted.
int64_t greedy_order_take_touches();

#endif
