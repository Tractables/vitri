//! Reading the `VITRI_*` environment variables.
//!
//! One rule, for every knob: unset means the documented default.
//!
//! Each reader here comes in two halves: a pure function over the value, so the
//! accepted spellings can be tested without mutating the process environment,
//! and a thin reader that fetches the variable and hands it over.
//!
//! How a value is written is settled here too, once, for every knob: the shell
//! that exports one leaves whitespace around it easily and none of these values
//! is whitespace, so surrounding whitespace is not part of the value; and where
//! a value is a *word* rather than a number, the word is read regardless of
//! case. [`is_form`] is that rule, and every vocabulary in the crate matches
//! through it.

use std::str::FromStr;

use crate::error::VitriError;

/// The spellings a flag accepts, quoted in its error message so the message and
/// the parser cannot drift apart.
const FLAG_FORMS: &str = "1, on or true (on), or 0, off or false (off)";

/// [`FLAG_FORMS`] plus what UNSET means for the knob at hand. Most knobs ship
/// off, but a knob whose production setting is ON has to say so, or its error
/// message tells the reader the opposite of what the code does.
fn flag_forms(default: bool) -> String {
    format!(
        "{FLAG_FORMS}; unset is {}",
        if default { "on" } else { "off" }
    )
}

/// The raw value of `name`, if it is set at all.
///
/// # Errors
///
/// [`VitriError::Env`] when the variable is set to a non-UTF-8 value.
pub(crate) fn env_raw(name: &'static str, expected: &str) -> Result<Option<String>, VitriError> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(VitriError::env(
            name,
            format!("must be {expected}; the value set is not valid UTF-8"),
        )),
    }
}

/// The raw value of `name` for a knob that TOLERATES a value it cannot use.
///
/// The one shape here with no `expected` clause and no error: a caller that
/// reads an unusable value as unset has no message to put one in. Bytes that
/// are not UTF-8 read as unset for the same reason the unparseable ones do.
/// [`crate::config::RunConfig::budget_ms`]'s default is the only knob of this
/// kind, and `docs/env.md` records that it is.
pub(crate) fn env_opt(name: &'static str) -> Option<String> {
    std::env::var(name).ok()
}

/// Parse an already-read value as `T`. The pure half of [`parse`].
///
/// # Errors
///
/// [`VitriError::Env`] when `value` is `Some` and does not parse as `T`.
pub(crate) fn parse_value<T: FromStr>(
    name: &'static str,
    value: Option<&str>,
    default: T,
    expected: &str,
) -> Result<T, VitriError> {
    match value {
        None => Ok(default),
        Some(raw) => raw
            .trim()
            .parse()
            // What the variable was SET to, not what parsing made of it: the
            // reader has to recognise their own value in the message.
            .map_err(|_| VitriError::env(name, format!("must be {expected}; got {raw:?}"))),
    }
}

/// Whether `value` is written as `form` — the one rule for reading a knob whose
/// value is a word (see the module header).
pub(crate) fn is_form(value: &str, form: &str) -> bool {
    value.trim().eq_ignore_ascii_case(form)
}

/// Read an already-read value against an enumerated vocabulary. The pure half
/// of every knob whose values are words.
///
/// `forms` is searched in order, through [`is_form`]; `expected` states the
/// vocabulary in the words of whoever sets the variable and is quoted verbatim
/// in the message, so the vocabulary and its description sit at one site.
///
/// # Errors
///
/// [`VitriError::Env`] when `value` is `Some` and is none of the `forms`.
pub(crate) fn from_forms<T: Copy>(
    name: &'static str,
    value: Option<&str>,
    default: T,
    forms: &[(&str, T)],
    expected: &str,
) -> Result<T, VitriError> {
    let Some(raw) = value else { return Ok(default) };
    forms
        .iter()
        .find(|(form, _)| is_form(raw, form))
        .map(|&(_, picked)| picked)
        .ok_or_else(|| VitriError::env(name, format!("must be {expected}; got {raw:?}")))
}

/// Parse environment variable `name` as `T`.
///
/// `expected` states the accepted form in the words of whoever sets the
/// variable ("a number of milliseconds", not "u64") — it is quoted verbatim in
/// the message.
///
/// # Errors
///
/// [`VitriError::Env`] when the variable is set to a value that does not parse
/// as `T`, or to bytes that are not UTF-8.
pub(crate) fn parse<T: FromStr>(
    name: &'static str,
    default: T,
    expected: &str,
) -> Result<T, VitriError> {
    let raw = env_raw(name, expected)?;
    parse_value(name, raw.as_deref(), default, expected)
}

/// Read an already-read value as an on/off flag. The pure half of
/// [`env_flag_or`].
///
/// The empty string is an error, not treated as unset.
///
/// # Errors
///
/// [`VitriError::Env`] when `value` is `Some` and matches none of the accepted
/// spellings.
pub(crate) fn flag_value(
    name: &'static str,
    value: Option<&str>,
    default: bool,
) -> Result<bool, VitriError> {
    const FLAG: &[(&str, bool)] = &[
        ("1", true),
        ("on", true),
        ("true", true),
        ("0", false),
        ("off", false),
        ("false", false),
    ];
    from_forms(name, value, default, FLAG, &flag_forms(default))
}

/// Read environment variable `name` as an on/off flag that is OFF when unset —
/// which is every switch that ships disabled.
///
/// # Errors
///
/// [`VitriError::Env`] when the variable is set to something that is neither an
/// on nor an off spelling.
pub(crate) fn env_flag(name: &'static str) -> Result<bool, VitriError> {
    env_flag_or(name, false)
}

/// Read environment variable `name` as an on/off flag whose meaning when UNSET
/// is `default`, over [`flag_value`]. [`env_flag`] is this with `default =
/// false`; a switch whose production setting is on takes the other one, so that
/// turning it off is spelled the same way as turning anything else on.
///
/// # Errors
///
/// [`VitriError::Env`] when the variable is set to something that is neither an
/// on nor an off spelling.
pub(crate) fn env_flag_or(name: &'static str, default: bool) -> Result<bool, VitriError> {
    let raw = env_raw(name, &flag_forms(default))?;
    flag_value(name, raw.as_deref(), default)
}
