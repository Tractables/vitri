// getrusage for a build for Emscripten, compiled only there.
//
// libc's getrusage on Emscripten reports a CPU time that never advances, and
// before Emscripten 4.0.12 it also wrote past the end of the struct it was
// given. Arjun, CryptoMiniSat and SBVA measure their CPU-time limits with it.
// This definition, linked ahead of libc's, reads CLOCK_PROCESS_CPUTIME_ID.

#include <sys/resource.h>

#include <cstring>
#include <ctime>

extern "C" int getrusage(int who, struct rusage *usage) {
    (void)who;
    timespec now{};
    clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &now);
    std::memset(usage, 0, sizeof *usage);
    usage->ru_utime.tv_sec = now.tv_sec;
    usage->ru_utime.tv_usec = static_cast<suseconds_t>(now.tv_nsec / 1000);
    return 0;
}
