/*
 Copyright (c) 2016, Ben Strasser
 All rights reserved.

 Redistribution and use in source and binary forms, with or without modification,
 are permitted provided that the following conditions are met:

 Redistributions of source code must retain the above copyright notice, this list
 of conditions and the following disclaimer.
 Redistributions in binary form must refroduce the above copyright notice, this
 list of conditions and the following disclaimer in the documentation and/or
 other materials provided with the distribution.

 THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
 ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
 WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
 DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR
 ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
 (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
 LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON
 ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
 (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
 SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

 Kenji Hashimoto has to say:
 *   This file is a degraded copy of pace.cpp in flow-cutter-pace17.
 *   I made a small change to the source code for our use (e.g., timeout).
 *
 *  Editted on: 2022/03/29
 *      Author: k-hasimt
 */

#pragma once

#include <cstdint>  // vitri: int64_t — not guaranteed transitively on all stdlibs
#include <string>
#include <vector>
#include "flow-cutter-pace17/src/array_id_func.hpp"
#include "flow-cutter-pace17/src/cell.hpp"
#include "TreeDecomposition.hpp"

namespace TWD {

struct SeparatorOutput {
  std::vector<int> separator;   // vertex IDs in input-node-id space
};

class IFlowCutter {
public:
  IFlowCutter(int n, int m, int verb = 0);

  void importGraph(const Graph& g);
  TreeDecomposition constructTD(const int64_t steps = 1e5, const int iters = 900);
  TreeDecomposition constructTD_timed(int64_t steps, int iters, int64_t timeout_ms);
  // vitri: `tight_gates` is independent of whether a deadline exists.
  //   false — the deadline is a BOUND the caller does not expect to reach. The
  //           pre-loop heuristics keep their untimed node gates, so the search
  //           is exactly the untimed one until the deadline actually fires.
  //   true  — the deadline is expected to bite, so the heuristics take their
  //           tight gates; on a large graph a single ordering pass can eat the
  //           whole budget.
  // Conflating the two made every deadline-armed build search less patiently
  // whether or not the deadline was ever reached.
  TreeDecomposition constructTD_timed_patience(int64_t steps, int iters, int64_t timeout_ms, int64_t patience_ms, bool tight_gates);
  // Compute one top-level balanced separator (no TD construction).
  // Runs ComputeSeparator up to `iters` times with different seeds, keeping the
  // smallest valid separator.  `timeout_ms == 0` means step-budget only.
  SeparatorOutput computeSeparator(int64_t steps, int iters, int64_t timeout_ms);
  auto num_nodes() const { return nodes; }

private:
  void print_comment(std::string msg);
  int compute_max_bag_size_of_order(const ArrayIDIDFunc&order);
  void test_new_order(const ArrayIDIDFunc&order, TreeDecomposition&td);

  TreeDecomposition output_tree_decompostion_of_order(ArrayIDIDFunc tail, ArrayIDIDFunc head, const ArrayIDIDFunc&order);
  TreeDecomposition output_tree_decompostion_of_multilevel_partition(const ArrayIDIDFunc&tail, const ArrayIDIDFunc&head, const ArrayIDIDFunc&to_input_node_id, const std::vector<Cell>&cell_list);

  int nodes;
  int best_bag_size;

  ArrayIDIDFunc head, tail;
  int verb = 0;
  double start_time;
};
}
