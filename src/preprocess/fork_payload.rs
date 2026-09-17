//! The wire codec that carries a reduce's result out of the forked child of
//! [`super::fork_budget`] and back into the parent.

use crate::cnf::{Literal, VarId};

/// A result type that can travel from the forked child back to the parent.
///
/// The encoding is a length-prefixed little-endian byte stream, decoded in the
/// same order; it is private to one `fork`/`read` pair in one process, so there
/// is no versioning or endianness concern.
pub(crate) trait ForkPayload: Sized {
    /// Append `self` to `out`.
    fn encode(&self, out: &mut Vec<u8>);
    /// Read a value back. `None` on a truncated or malformed stream.
    fn decode(d: &mut Dec<'_>) -> Option<Self>;
}

pub(super) fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
pub(super) fn put_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}
pub(super) fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}
pub(super) fn put_len(out: &mut Vec<u8>, v: usize) {
    put_u64(out, v as u64);
}
pub(super) fn put_str(out: &mut Vec<u8>, s: &str) {
    put_len(out, s.len());
    out.extend_from_slice(s.as_bytes());
}
/// A literal packs into one `u32` as `var * 2 + polarity`. Variable ids are CNF
/// indices (bounded by the formula's `num_vars`), so the top bit is never used
/// in practice; the assert documents and checks the assumption in debug builds.
pub(super) fn put_literal(out: &mut Vec<u8>, l: Literal) {
    debug_assert!(l.var.0 < 1 << 31, "var id too large to pack: {}", l.var.0);
    put_u32(out, (l.var.0 << 1) | u32::from(l.positive));
}

/// Cursor over the child's byte stream.
pub(crate) struct Dec<'a> {
    pub(super) rest: &'a [u8],
}

impl<'a> Dec<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Dec { rest: bytes }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.rest.len() < n {
            return None;
        }
        let (head, tail) = self.rest.split_at(n);
        self.rest = tail;
        Some(head)
    }
    pub(super) fn get_u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    pub(super) fn get_u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    pub(super) fn get_i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    pub(super) fn get_u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    /// Length prefixes are bounded by the bytes actually received, so a corrupt
    /// stream cannot make us reserve a huge vector.
    pub(super) fn get_len(&mut self) -> Option<usize> {
        let n = usize::try_from(self.get_u64()?).ok()?;
        (n <= self.rest.len()).then_some(n)
    }
    pub(super) fn get_str(&mut self) -> Option<&'a str> {
        let n = self.get_len()?;
        std::str::from_utf8(self.take(n)?).ok()
    }
    pub(super) fn get_literal(&mut self) -> Option<Literal> {
        let packed = self.get_u32()?;
        Some(Literal::new(VarId(packed >> 1), packed & 1 == 1))
    }
}

/// The byte-count prefix is validated against the bytes remaining before use,
/// so a corrupt stream cannot force a huge allocation or call `elem` more
/// times than the stream can supply.
pub(super) fn get_vec<T>(
    d: &mut Dec<'_>,
    mut elem: impl FnMut(&mut Dec<'_>) -> Option<T>,
) -> Option<Vec<T>> {
    let n = d.get_len()?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(elem(d)?);
    }
    Some(out)
}
