// A work clock for the wall deadlines (vitri; see PROVENANCE.md).
//
// Every deadline the Arjun stack sets (conf.deadline in CryptoMiniSat and
// Arjun, Arjun's progress checkpoint, the oracle's and CadiBack's deadline)
// is a point on one clock, read through work_clock_now() and tested through
// work_clock_passed(). Until start_work_clock() is called that clock is
// real_time_sec(), and every test is the wall comparison it replaced.
//
// Once started, the clock counts work instead of time: for each literal the
// CryptoMiniSat search and the oracle propagate, one unit and one per watch
// it visits; one per clause visited by a flip of the oracle's local search;
// the literals CaDiCaL propagates in Arjun's solves; plus a fixed charge for each
// time CadiBack's CaDiCaL polls its terminator, at a fixed number of units
// per second. A deadline is then in work seconds since the start, and a stage
// stops at the same point on every run of the same input. The wall stays a
// runaway stop: a deadline also passes once the wall time since the start is
// `runaway` times the deadline, and wall_stopped records that it did.
//
// Header-only, with one inline object, so the oracle library and
// CryptoMiniSat share the clock without a link dependency between them. The
// object is per thread: a reduction runs on one thread, and reductions on
// other threads keep their own clocks.
#pragma once

#include <cstdint>

#include "time_mem.h"

namespace CMSat {

struct WorkClock {
    bool on = false;
    double units_per_sec = 1.0;
    double runaway = 0.0;
    double units_per_poll = 0.0;
    double wall_start = 0.0;
    // Propagated literals and the watches and clauses they visit.
    uint64_t visits = 0;
    // CadiBack terminator polls.
    uint64_t polls = 0;
    bool wall_stopped = false;
};

inline thread_local WorkClock work_clock;

inline void work_clock_start(double units_per_sec, double runaway, double units_per_poll)
{
    work_clock = WorkClock();
    work_clock.on = true;
    work_clock.units_per_sec = units_per_sec;
    work_clock.runaway = runaway;
    work_clock.units_per_poll = units_per_poll;
    work_clock.wall_start = real_time_sec();
}

inline void work_clock_stop() { work_clock.on = false; }

inline double work_clock_units()
{
    return (double)work_clock.visits
        + work_clock.units_per_poll * (double)work_clock.polls;
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

// The test CadiBack's terminator makes: one poll charged, then the deadline.
inline bool work_clock_poll(double deadline)
{
    work_clock.polls++;
    return work_clock_passed(deadline);
}

}
