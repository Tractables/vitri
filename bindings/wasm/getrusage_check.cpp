// Checks the getrusage that a build for Emscripten links
// (vendor/arjun/emscripten_getrusage.cpp): the user CPU time it reports
// advances over some work, and a call writes nothing outside the struct.
//
//   em++ -O2 ../../vendor/arjun/emscripten_getrusage.cpp getrusage_check.cpp -o target/getrusage_check.js
//   node target/getrusage_check.js

#include <sys/resource.h>

#include <cstdio>
#include <cstring>

namespace {

struct Guarded {
    unsigned char before[64];
    struct rusage usage;
    unsigned char after[64];
};

double user_seconds() {
    struct rusage usage;
    getrusage(RUSAGE_SELF, &usage);
    return static_cast<double>(usage.ru_utime.tv_sec) + usage.ru_utime.tv_usec / 1e6;
}

}  // namespace

int main() {
    Guarded guarded;
    std::memset(&guarded, 0xAA, sizeof guarded);
    getrusage(RUSAGE_SELF, &guarded.usage);
    int written_outside = 0;
    for (unsigned char byte : guarded.before) written_outside += byte != 0xAA;
    for (unsigned char byte : guarded.after) written_outside += byte != 0xAA;

    const double start = user_seconds();
    volatile unsigned long sum = 0;
    for (unsigned long i = 0; i < 200000000UL; ++i) sum += i;
    const double end = user_seconds();

    std::printf("bytes written outside struct rusage: %d; user CPU time %.6f s -> %.6f s\n",
                written_outside, start, end);
    if (written_outside != 0) {
        std::printf("FAIL: getrusage wrote outside the struct it was given\n");
        return 1;
    }
    if (!(end > start)) {
        std::printf("FAIL: the user CPU time getrusage reports did not advance\n");
        return 1;
    }
    return 0;
}
