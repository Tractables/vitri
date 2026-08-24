// The two CaDiCaL accessors that reach past its public class: per-variable
// VSIDS activity, and the CDCL search counters.
//
// Kept in its own translation unit because `internal.hpp` pulls in CaDiCaL's
// whole internal header set, including the tracer and file hierarchies.
// `cadical_shim.cpp` includes only `cadical.hpp` and defines the stable C ABI
// the Rust side calls; dragging the internal headers in there would put that
// ABI's definition in the same file as several hundred internal declarations,
// and every internal rename would become a shim rebuild.
//
// Both functions are declared `friend` inside `CaDiCaL::Solver` (see
// `upstream/cadical/src/cadical.hpp`) so they can reach the private `internal`
// pointer. The friend clauses name them at global scope, so these definitions
// must be at global scope too.
//
// This file MUST be compiled with the same preprocessor definitions the
// vendored CaDiCaL library itself was built with — `Internal` and `Stats` have
// conditionally compiled members, so a mismatched define set changes their
// layout and this file would read the wrong offsets. `build.rs` passes that
// define set and checks it against CaDiCaL's own CMakeLists.

#include "upstream/cadical/src/internal.hpp"

double vitri_cadical_score_of (const CaDiCaL::Solver *solver, int lit) {
  return solver->internal->score (lit);
}

// Slot order is the contract between this file and `SearchStats` on the Rust
// side. APPEND only, never reorder.
//
//   0 conflicts               1 decisions
//   2 propagations (search)   3 restarts
//   4 learned clauses         5 searched decisions (decide-loop scan)
//
// A caller-sized array is filled as far as this file can go and the rest is
// zeroed, so appending a slot here leaves an older caller working and a caller
// that asks for more than exists gets zeros rather than garbage.
void vitri_cadical_search_stats (const CaDiCaL::Solver *solver, long long *out,
                                 unsigned long n) {
  const CaDiCaL::Internal *i = solver->internal;
  const long long vals[] = {
      (long long) i->stats.conflicts,
      (long long) i->stats.decisions,
      (long long) i->stats.propagations.search,
      (long long) i->stats.restarts,
      (long long) i->stats.learned.clauses,
      (long long) i->stats.searched,
  };
  const unsigned long have = sizeof (vals) / sizeof (vals[0]);
  for (unsigned long k = 0; k < n; k++)
    out[k] = k < have ? vals[k] : 0;
}
