//! The `key=value` pairs of one spec, and the reads a family rule makes of
//! them.
//!
//! Reading a key marks it used, and [`KeyedParams::finish`] refuses the first
//! key nothing read. That is what makes "a parameter the spec cannot honor is
//! refused rather than ignored" hold for every family without each family
//! restating it.

use crate::error::VitriError;

use super::vocabulary::{SPEC_PARAM_KEYS, VtreeBase, keys_for, lookup, value_names};
use super::{invalid_token, one_of};

/// The `key=value` pairs of one spec, each remembering whether a family rule
/// read it.
///
/// Reading a key marks it used; [`KeyedParams::finish`] then refuses the first
/// key nothing read, which is what makes "a parameter the spec cannot honor is
/// refused rather than ignored" hold for every family without each family
/// restating it.
pub(super) struct KeyedParams<'a> {
    /// The whole spec string, for the messages that name it.
    spec: &'a str,
    /// One entry per written `key=value`, in written order.
    entries: Vec<Entry<'a>>,
}

/// One written `key=value`.
struct Entry<'a> {
    key: &'a str,
    value: &'a str,
    used: bool,
}

impl<'a> KeyedParams<'a> {
    /// Split a spec's parameter text into its pairs, refusing a malformed pair,
    /// an empty key and a key written twice.
    ///
    /// A repeated key is refused rather than resolved: with one of the two
    /// values necessarily dropped, silently keeping either would make the spec
    /// mean something the reader did not write.
    pub(super) fn new(spec: &'a str, raw: Option<&'a str>) -> Result<Self, VitriError> {
        let mut entries: Vec<Entry<'a>> = Vec::new();
        for part in raw.into_iter().flat_map(|p| p.split(',')) {
            let Some((key, value)) = part.split_once('=') else {
                return Err(VitriError::spec(
                    spec,
                    format!("parameter {part:?} must be written key=value"),
                ));
            };
            if key.is_empty() {
                return Err(VitriError::spec(
                    spec,
                    format!("parameter {part:?} has an empty key"),
                ));
            }
            if entries.iter().any(|e| e.key == key) {
                return Err(VitriError::spec(
                    spec,
                    format!(
                        "parameter {key:?} is written twice; one of the two values would be \
                         dropped. Write it once"
                    ),
                ));
            }
            entries.push(Entry {
                key,
                value,
                used: false,
            });
        }
        Ok(KeyedParams { spec, entries })
    }

    /// The pairs the spec wrote, ordered as [`SPEC_PARAM_KEYS`] declares them
    /// rather than as they were typed, so two spellings of one construction
    /// render alike.
    ///
    /// A key outside that table survives only under an unrecognized base, where
    /// `finish` never runs; it keeps its written position, which is all anything
    /// can say about it.
    pub(super) fn written(&self) -> Vec<(&'a str, &'a str)> {
        let rank = |key: &str| {
            SPEC_PARAM_KEYS
                .iter()
                .position(|k| k.key == key)
                .unwrap_or(usize::MAX)
        };
        let mut pairs: Vec<(&'a str, &'a str)> =
            self.entries.iter().map(|e| (e.key, e.value)).collect();
        pairs.sort_by_key(|(key, _)| rank(key));
        pairs
    }

    /// The value written for `key`, marking it read. `None` when the spec did
    /// not write it.
    pub(super) fn take(&mut self, key: &str) -> Option<&'a str> {
        self.entries.iter_mut().find(|e| e.key == key).map(|e| {
            e.used = true;
            e.value
        })
    }

    /// Was `key` written? Does NOT mark it read — for a rule that reacts to a
    /// key some other rule owns.
    pub(super) fn wrote(&self, key: &str) -> bool {
        self.entries.iter().any(|e| e.key == key)
    }

    /// The value of `key` looked up in `table`, marking the key read.
    pub(super) fn enum_value<T: Copy>(
        &mut self,
        key: &str,
        table: &[(&'static str, T)],
    ) -> Result<Option<T>, VitriError> {
        match self.take(key) {
            None => Ok(None),
            Some(v) => match lookup(table, v) {
                Some(found) => Ok(Some(found)),
                None => Err(invalid_token(
                    self.spec,
                    key,
                    v,
                    &one_of(value_names(table)),
                )),
            },
        }
    }

    /// The value of `key` parsed as a number, marking the key read. `expected`
    /// is how a rejection describes what would have been accepted.
    pub(super) fn number<T: std::str::FromStr>(
        &mut self,
        key: &str,
        expected: &str,
    ) -> Result<Option<T>, VitriError> {
        match self.take(key) {
            None => Ok(None),
            Some(v) => match v.parse::<T>() {
                Ok(n) => Ok(Some(n)),
                Err(_) => Err(invalid_token(self.spec, key, v, expected)),
            },
        }
    }

    /// Refuse the first key no family rule read: either the family accepts no
    /// such key at all, or it accepts it only in a mode this spec is not in
    /// (the mode-specific rules report that themselves, before this runs).
    pub(super) fn finish(&self, family: VtreeBase, base: &str) -> Result<(), VitriError> {
        let Some(unread) = self.entries.iter().find(|e| !e.used) else {
            return Ok(());
        };
        let accepted = keys_for(family);
        let offer = if accepted.is_empty() {
            format!("{base:?} takes no parameters")
        } else {
            format!("{base:?} takes {}", one_of(accepted))
        };
        Err(VitriError::spec(
            self.spec,
            format!("parameter \"{}=\" is not one {offer}", unread.key),
        ))
    }
}
