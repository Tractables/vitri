// The self-test hook for the vendored FlowCutter k-way id-heap, in a
// translation unit of its own: the linker pulls an object out of a static
// archive only when something references a symbol it defines, so a program
// that never calls this never links it. Carries vitri's licence, like the rest
// of the FFI shim; the heap it exercises is upstream's.
#include <climits>  // vitri: INT_MAX for the heap-overflow regression check
#include "ffi.h"
#include "heap.hpp"  // vitri: kway id-heaps under test by treedecomp_heap_selftest

extern "C" {

// Self-test for the vendored k-way id-heap. Returns 0 on success, or the id of
// the first failing check. Compiled with NDEBUG (asserts off) like the rest of
// the FFI, so these checks must be explicit `return`s, not assert()s — exactly
// the configuration in which bug #18 went undetected.
int treedecomp_heap_selftest(void) {
    typedef kway_min_id_heap<int, 4> MinHeap4;

    // --- bug #18 regression: child-index arithmetic must stay 64-bit-safe ---
    // pos=626481186 is the heap position that SIGSEGV'd: children_begin = 4*pos+1
    // overflowed int32 to a negative index. The helpers now compute in long long.
    {
        long long pos = 626481186LL;
        if (MinHeap4::children_begin(pos) != 2505924745LL) return 30;
        if (MinHeap4::children_end(pos)   != 2505924749LL) return 31; // 4*(pos+1)+1
        if (MinHeap4::children_begin(pos) <= (long long)INT_MAX) return 32; // must exceed INT_MAX
        if (MinHeap4::children_begin(pos) < 0) return 33; // must not be negative
        // Small positions remain correct (k=4: children of 0 are 1..4, of 1 are 5..8).
        if (MinHeap4::children_begin(0) != 1)  return 34;
        if (MinHeap4::children_end(0)   != 5)  return 35;
        if (MinHeap4::children_begin(1) != 5)  return 36;
    }

    // --- push/pop max-ordering on a moderate heap (exercises move_up/move_down) ---
    {
        const int N = 200000;
        kway_max_id_heap<int, 4> q(N);
        for (int i = 0; i < N; i++) {
            int key = (int)(((long long)i * 1103515245LL + 12345LL) & 0x7fffffffLL);
            q.push(i, key);
        }
        if (q.size() != N) return 40;
        long long prev = (long long)INT_MAX + 1; // max-heap: keys must be non-increasing
        int count = 0;
        while (!q.empty()) {
            int key = q.peek_max_key();
            if ((long long)key > prev) return 41; // ordering broken
            prev = key;
            q.pop();
            count++;
        }
        if (count != N) return 42;
        if (!q.empty()) return 43;
    }

    // --- contains / get_key / pop-removes-id ---
    {
        kway_max_id_heap<int, 4> r(16);
        r.push(3, 100);
        r.push(7, 50);
        r.push(11, 200);
        if (!r.contains(3))  return 50;
        if (!r.contains(7))  return 51;
        if (r.contains(5))   return 52; // never pushed
        if (r.get_key(3) != 100) return 53;
        if (r.get_key(11) != 200) return 54;
        int top = r.pop();              // max key 200 -> id 11
        if (top != 11)       return 55;
        if (r.contains(11))  return 56; // popped
        if (r.size() != 2)   return 57;
    }

    return 0;
}

} // extern "C"
