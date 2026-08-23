//! The xorshift64 generator the decomposition backends draw from.

/// Deterministic xorshift64 with the usual 13 / 7 / 17 shift triple.
///
/// State 0 is the recurrence's fixed point and yields an endless run of zeros,
/// so a caller starting from a caller-supplied seed has to move it off zero
/// first — [`SEED_OFFSET`] is one of the two offsets in use for that.
#[derive(Clone, Copy)]
pub(super) struct Xorshift64(u64);

/// Odd constant one group of callers adds to its seed before starting a stream
/// to move seed 0 off the fixed point and send nearby seeds to unrelated
/// streams. The golden ratio scaled to 2^64, the same constant SplitMix64 steps
/// its state by.
///
/// The bisectors add 1 instead, through [`bisector_stream`] — changing either
/// offset changes the stream it feeds.
pub(super) const SEED_OFFSET: u64 = 0x9E37_79B9_7F4A_7C15;

/// Starts a bisector's stream: `+ 1` keeps a seed of 0 off the recurrence's
/// zero fixed point. This offset is part of the stream every existing seed has
/// always drawn from — changing it reshuffles all of them.
pub(super) fn bisector_stream(seed: u64) -> Xorshift64 {
    Xorshift64::from_state(seed.wrapping_add(1))
}

impl Xorshift64 {
    /// Takes `state` exactly as given; callers apply any off-zero offset
    /// themselves before calling.
    pub(super) fn from_state(state: u64) -> Self {
        Xorshift64(state)
    }

    pub(super) fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub(super) fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }
}
