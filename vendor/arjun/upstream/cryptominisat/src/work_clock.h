// A work clock for the wall deadlines (vitri; see PROVENANCE.md).
//
// Every deadline the Arjun stack sets (conf.deadline in CryptoMiniSat and
// Arjun, Arjun's progress checkpoint, the oracle's and CadiBack's deadline)
// is a point on one clock, read through work_clock_now() and tested through
// work_clock_passed(). Until start_work_clock() is called that clock is
// real_time_sec(), and every test is the wall comparison it replaced.
//
// Once started, the clock counts work instead of time: each kind of work
// below has its own counter and its own charge in units, and the clock reads
// the charged units at a fixed number of units per second. A deadline is
// then in work seconds since the start, and a stage stops at the same point
// on every run of the same input. The wall stays a runaway stop: a deadline
// also passes once the wall time since the start is `runaway` times the
// deadline, and wall_stopped records that it did.
//
// Header-only, with one inline object, so the oracle library and
// CryptoMiniSat share the clock without a link dependency between them. The
// object is per thread: a reduction runs on one thread, and reductions on
// other threads keep their own clocks. An Emscripten build keeps one plain
// object: the web module links without threads, so nothing sets up
// thread-local storage in it, yet CryptoMiniSat, the oracle and CadiBack are
// compiled with -pthread, which keeps a thread_local real in their objects, and
// the clock would then be read and written at a wrong address.
#pragma once

#include <cstdint>

#include "time_mem.h"

namespace CMSat {

// The kinds of work the clock counts. The order is the order of the charges
// passed to work_clock_start() and of the counts read_work_clock() returns.
enum WorkKind : int {
    // CryptoMiniSat's search: literals propagated, watches visited (of every
    // kind), binary watches among them, long clauses dereferenced past their
    // blocking literal, conflicts.
    WK_CMS_PROP,
    WK_CMS_WATCH,
    WK_CMS_BIN,
    WK_CMS_DEREF,
    WK_CMS_CONFL,
    // CryptoMiniSat's other propagation (light and occurrence propagation,
    // failed-literal probing), in its own bogoprops.
    WK_CMS_LIGHT,
    // The oracle: literals propagated, watches visited, binary watches among
    // them, long clauses dereferenced past their blocking literal, literals
    // scanned for a new watch, conflicts, literals visited in conflict
    // analysis and minimization, solution cache entries checked and stored
    // (one per variable), assignments undone, calls, and the variables and
    // literals each oracle is built from.
    WK_ORC_PROP,
    WK_ORC_WATCH,
    WK_ORC_BIN,
    WK_ORC_DEREF,
    WK_ORC_SCAN,
    WK_ORC_CONFL,
    WK_ORC_LEARN,
    WK_ORC_CACHE,
    WK_ORC_UNDO,
    WK_ORC_CALL,
    WK_ORC_BUILD,
    // The oracle's local search: clause occurrences visited by its flips,
    // and the flips.
    WK_LS_VISIT,
    WK_LS_FLIP,
    // CaDiCaL, in Arjun's solves and in CadiBack's: literals propagated
    // (search and inprocessing), conflicts, search ticks, learned literals,
    // subsumption checks, and Arjun's solves.
    WK_CDC_PROP,
    WK_CDC_CONFL,
    WK_CDC_TICKS,
    WK_CDC_LEARN,
    WK_CDC_SUBCHK,
    WK_CDC_SOLVE,
    // CadiBack's terminator polls.
    WK_POLL,
    // Literals in the formula at each occurrence simplification and each
    // SBVA run, and literals loaded.
    WK_OCC_LITS,
    WK_SBVA_LITS,
    WK_LOAD,
    WK_N
};

struct WorkClock {
    bool on = false;
    double units_per_sec = 1.0;
    double runaway = 0.0;
    double wall_start = 0.0;
    double charge[WK_N] = {};
    uint64_t count[WK_N] = {};
    bool wall_stopped = false;
};

#if defined(__EMSCRIPTEN__)
inline WorkClock work_clock;
#else
inline thread_local WorkClock work_clock;
#endif

// Start the clock: `charge` holds WK_N units, one per kind of work.
inline void work_clock_start(double units_per_sec, double runaway, const double* charge)
{
    work_clock = WorkClock();
    work_clock.on = true;
    work_clock.units_per_sec = units_per_sec;
    work_clock.runaway = runaway;
    for (int k = 0; k < WK_N; k++) work_clock.charge[k] = charge[k];
    work_clock.wall_start = real_time_sec();
}

inline void work_clock_stop() { work_clock.on = false; }

inline double work_clock_units()
{
    double units = 0;
    for (int k = 0; k < WK_N; k++) units += work_clock.charge[k] * (double)work_clock.count[k];
    return units;
}

// Now, on the clock deadlines are set against.
inline double work_clock_now()
{
    if (!work_clock.on) return real_time_sec();
    return work_clock_units() / work_clock.units_per_sec;
}

// Whether `deadline`, a point on the same clock, has passed.
inline bool work_clock_passed(double deadline)
{
    if (!work_clock.on) return real_time_sec() > deadline;
    if (work_clock_now() > deadline) return true;
    if (real_time_sec() - work_clock.wall_start > work_clock.runaway * deadline) {
        work_clock.wall_stopped = true;
        return true;
    }
    return false;
}

// Count CaDiCaL work: `w` holds the vitri_cadical_work_n counts that
// vitri_cadical_work() reads (cadical.hpp), as differences since the last
// charge.
inline void work_clock_charge_cadical(const long long* w)
{
    const WorkKind kinds[] = {WK_CDC_PROP, WK_CDC_CONFL, WK_CDC_TICKS, WK_CDC_LEARN, WK_CDC_SUBCHK};
    for (int k = 0; k < 5; k++) work_clock.count[kinds[k]] += (uint64_t)(w[k] > 0 ? w[k] : 0);
}

// The test CadiBack's terminator makes: one poll and the CaDiCaL work `w`
// done since the last poll charged, then the deadline.
inline bool work_clock_poll(double deadline, const long long* w)
{
    work_clock.count[WK_POLL]++;
    work_clock_charge_cadical(w);
    return work_clock_passed(deadline);
}

}
