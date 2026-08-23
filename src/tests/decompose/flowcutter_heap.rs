//! The vendored k-way id-heap behind FlowCutter, driven through its own C++
//! self-test.
//!
//! Nothing here names a Rust item: the self-test is a C symbol in the archive
//! `build.rs` links, so this sits with the crate's other decomposition tests
//! rather than inside `flowcutter`, which it does not reach into.

use std::os::raw::c_int;

// SAFETY: mirrors the declaration in `vendor/treedecomp/heap_selftest.cpp`, which is
// vendored here and built by `build.rs`; the self-test takes no arguments and
// returns a plain int, so there is nothing else to match.
unsafe extern "C" {
    /// Self-test of the vendored k-way id-heap (push/pop ordering, contains,
    /// and the child-index overflow boundary). 0 = pass; nonzero = the id of
    /// the first failing check (see `vendor/treedecomp/heap_selftest.cpp`).
    fn treedecomp_heap_selftest() -> c_int;
}

/// Heap-overflow regression. The vendored FlowCutter k-way heap computed
/// child positions as `k*pos+1` in signed 32-bit int. A bag-adjacency graph
/// with more than ~5.4e8 elements makes `move_down` descend past
/// `INT_MAX/k`, the product overflows to a negative index, and
/// `heap[negative]` faults inside C++ — uncatchable from Rust. The fix makes
/// that arithmetic 64-bit.
///
/// This drives the C++ self-test directly: reproducing the real crash needs
/// tens of gigabytes and minutes of preprocessing, whereas the self-test
/// checks the same overflow boundary at zero memory cost, alongside general
/// heap correctness.
#[test]
fn flowcutter_heap_selftest_passes() {
    // SAFETY: the self-test takes no arguments, touches no shared state, and
    // returns a plain int. It is compiled into libtreedecomp with NDEBUG —
    // the same configuration the production FFI uses.
    let rc = unsafe { treedecomp_heap_selftest() };
    assert_eq!(
        rc, 0,
        "treedecomp k-way heap self-test failed at check {rc} \
         (see vendor/treedecomp/heap_selftest.cpp for the check id; nonzero in the 30s \
         means the child-index overflow has regressed)"
    );
}
