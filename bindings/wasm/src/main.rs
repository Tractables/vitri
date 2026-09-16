//! vitri built for the browser: the C functions below wrap
//! [`vitri::request::capabilities_json`] and [`vitri::request::prepare_json`].
//!
//! Emscripten links a program, so `main` has to exist. It does nothing and the
//! page never runs it; the page calls the exported functions.
//!
//! Every string these functions return is NUL-terminated UTF-8, and the caller
//! releases it with [`vitri_string_free`].

use std::ffi::{CStr, CString, c_char};

fn main() {}

/// [`vitri::request::capabilities_json`].
#[unsafe(no_mangle)]
pub extern "C" fn vitri_capabilities_json() -> *mut c_char {
    into_c_string(vitri::request::capabilities_json())
}

/// [`vitri::request::prepare_json`] on the `len` bytes at `dimacs` and the
/// request at `request`; a null `request` is the empty request `{}`.
///
/// A request that is not UTF-8 is read with each invalid sequence replaced by
/// U+FFFD, which no request key, token or spec contains, so it is refused.
///
/// # Safety
///
/// `dimacs` points to `len` readable bytes, or is null or `len` is 0, which is
/// the empty input. `request` is null or points to a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitri_prepare_json(
    dimacs: *const u8,
    len: usize,
    request: *const c_char,
) -> *mut c_char {
    let dimacs = if dimacs.is_null() || len == 0 {
        &[][..]
    } else {
        // SAFETY: the caller guarantees `len` readable bytes at `dimacs`.
        unsafe { std::slice::from_raw_parts(dimacs, len) }
    };
    let request = if request.is_null() {
        "{}".into()
    } else {
        // SAFETY: the caller guarantees a NUL-terminated string.
        String::from_utf8_lossy(unsafe { CStr::from_ptr(request) }.to_bytes())
    };
    into_c_string(vitri::request::prepare_json(dimacs, &request))
}

/// Release a string [`vitri_capabilities_json`] or [`vitri_prepare_json`]
/// returned.
///
/// # Safety
///
/// `text` is null or a string one of those functions returned that nothing has
/// released since.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitri_string_free(text: *mut c_char) {
    if !text.is_null() {
        // SAFETY: the pointer came from `CString::into_raw` in
        // `into_c_string`, and this is its only release point.
        drop(unsafe { CString::from_raw(text) });
    }
}

fn into_c_string(json: String) -> *mut c_char {
    // JSON escapes a NUL inside a string, so serialized JSON never holds one.
    CString::new(json)
        .expect("serialized JSON contains no NUL byte")
        .into_raw()
}
