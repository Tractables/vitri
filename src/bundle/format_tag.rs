//! The `format` tag both bundle JSON files carry.
//!
//! `#[serde(with = ...)]` shims sit beside the two structs, where the tag
//! constant is; what they share is here. The tag is written as it stands and
//! read back only if it is the one this version of the crate writes.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serializer};

/// Write the tag.
pub(super) fn serialize<S: Serializer>(tag: &str, ser: S) -> Result<S::Ok, S::Error> {
    ser.serialize_str(tag)
}

/// Read a tag back, refusing any but `expected`. `what` names the file, since a
/// consumer reading a bundle holds both and their tags are not interchangeable.
///
/// # Errors
///
/// The format's own error, naming both tags, for a file this version of the
/// crate does not write.
pub(super) fn read<'de, D: Deserializer<'de>>(
    de: D,
    expected: &str,
    what: &str,
) -> Result<String, D::Error> {
    let tag = String::deserialize(de)?;
    if tag != expected {
        return Err(D::Error::custom(format!(
            "unknown {what} format {tag:?}; this version reads {expected:?}"
        )));
    }
    Ok(tag)
}
