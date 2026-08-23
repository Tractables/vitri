#pragma once
#include <cstdint>
#include <limits>
#include <vector>

namespace CadiBack {
// `deadline` is a wall-clock limit expressed as ABSOLUTE CLOCK_MONOTONIC
// seconds (the same quantity CryptoMiniSat's real_time_sec() returns).
// The default, numeric_limits<double>::max(), means "no deadline" and keeps
// behaviour bit-identical to a build without this parameter. When the
// deadline passes, the CaDiCaL search is terminated and `*limit_hit` is set,
// exactly as for an exhausted conflict budget — the caller gets the PARTIAL
// backbone found so far, which is sound (every literal in it was proved).
int doit (const std::vector<int>& cnf,
    int _verb,
    std::vector<int>& drop_cands,
    std::vector<int>& ret_backbone,
    std::vector<int>& ret_red_cls,
    std::vector<std::pair<int, int>>& ret_eqlits,
    int64_t max_confl = -1,
    bool* limit_hit = nullptr,
    double deadline = std::numeric_limits<double>::max());
}
