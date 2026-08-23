//! Restart seeding shared by the multilevel bisectors.

/// Seed for restart `restart` under `base_seed`. `base_seed = 0` reproduces the
/// `restart * 7919 + 42` stream exactly; any other seed — including one a small
/// distance from another — lands on an unrelated stream.
pub(super) fn restart_seed(base_seed: u64, restart: usize) -> u64 {
    let legacy = (restart as u64).wrapping_mul(7919).wrapping_add(42);
    legacy ^ fmix64(base_seed)
}

/// SplitMix64 finalizer: avalanches `base_seed` so seeds a small distance apart
/// land on unrelated streams.
pub(super) fn fmix64(mut z: u64) -> u64 {
    z ^= z >> 30;
    z = z.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z ^= z >> 27;
    z = z.wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}
