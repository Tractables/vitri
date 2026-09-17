//! The C API for vitri: DIMACS text and a JSON request go in, and the files
//! `vitri -o DIR` would write come back in memory with a summary of the run.
//!
//! `include/vitri.h` is generated from this file by cbindgen, so the
//! documentation comments on the exported items are what a C caller reads.
//! `README.md` beside this crate covers building and linking.
//!
//! Every exported function that runs code able to panic runs it under
//! `catch_unwind`, so no unwind reaches the caller.

#![allow(non_camel_case_types)]

use std::any::Any;
use std::ffi::{CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::{ptr, slice};

use vitri::VitriError;
use vitri::error::ErrorKind;
use vitri::request::{self, Prepared, Request};

/// The version of the ABI this header declares. `vitri_abi_version` returns
/// the version of the library that is actually loaded. It is raised whenever
/// a signature, a status code or an ownership rule changes in a way that
/// breaks a program built against the previous header. The JSON documents
/// carry their own format tags.
pub const VITRI_ABI_VERSION: u32 = 1;

/// A status code: `VITRI_OK`, or one of the `VITRI_ERROR_` values.
pub type vitri_code = i32;

/// The call succeeded.
pub const VITRI_OK: vitri_code = 0;
/// Error kind `config`: an unknown key, a value outside its
/// vocabulary, or settings that do not work together.
pub const VITRI_ERROR_CONFIG: vitri_code = 1;
/// Error kind `spec`: the `vtree` spec needs fixing.
pub const VITRI_ERROR_SPEC: vitri_code = 2;
/// Error kind `env`: a `VITRI_*` environment variable needs fixing.
pub const VITRI_ERROR_ENV: vitri_code = 3;
/// Error kind `input`: the DIMACS text cannot be used.
pub const VITRI_ERROR_INPUT: vitri_code = 4;
/// Error kind `mismatch`: two parts of one run describe different formulas.
pub const VITRI_ERROR_MISMATCH: vitri_code = 5;
/// Error kind `construction`: a vtree construction could not answer a well
/// formed request; another spec or a larger budget may.
pub const VITRI_ERROR_CONSTRUCTION: vitri_code = 6;
/// Error kind `io`: a file could not be read or written.
pub const VITRI_ERROR_IO: vitri_code = 7;
/// Error kind `argument`: an argument broke this header's contract, such as a
/// null pointer with a nonzero length.
pub const VITRI_ERROR_INVALID_ARGUMENT: vitri_code = 8;
/// Error kind `panic`: vitri panicked. The panic stopped at this boundary and
/// its message is the error message; report it as a bug.
pub const VITRI_ERROR_PANIC: vitri_code = 9;

/// What one `vitri_prepare` call produced: the files and summary of a run, or
/// the error that stopped it.
///
/// Opaque. Read it with the `vitri_result_` functions and release it with
/// `vitri_result_free`. A result does not change after `vitri_prepare` returns
/// it, so several threads may read one at the same time; free it once, after
/// the last read.
pub struct vitri_result {
    code: vitri_code,
    outcome: Outcome,
}

enum Outcome {
    Prepared(Run),
    Failed(Failure),
}

struct Run {
    status: Text,
    summary: Text,
    files: Vec<File>,
}

struct File {
    path: Text,
    contents: Text,
}

struct Failure {
    kind: Text,
    message: Text,
}

/// Bytes lent to C, stored with one NUL after them that their length does not
/// count.
struct Text(Box<[u8]>);

impl Text {
    fn new(bytes: impl Into<Vec<u8>>) -> Self {
        let mut bytes = bytes.into();
        bytes.push(0);
        Text(bytes.into_boxed_slice())
    }

    fn bytes(&self) -> &[u8] {
        &self.0[..self.0.len() - 1]
    }
}

impl vitri_result {
    fn prepared(prepared: Prepared) -> Self {
        let files = prepared
            .files
            .into_iter()
            .map(|file| File {
                path: Text::new(file.path),
                contents: Text::new(file.contents),
            })
            .collect();
        vitri_result {
            code: VITRI_OK,
            outcome: Outcome::Prepared(Run {
                status: Text::new(prepared.summary.status.token()),
                summary: Text::new(prepared.summary.to_json()),
                files,
            }),
        }
    }

    fn failed(code: vitri_code, kind: &str, message: String) -> Self {
        vitri_result {
            code,
            outcome: Outcome::Failed(Failure {
                kind: Text::new(kind),
                message: Text::new(message),
            }),
        }
    }

    fn run(&self) -> Option<&Run> {
        match &self.outcome {
            Outcome::Prepared(run) => Some(run),
            Outcome::Failed(_) => None,
        }
    }

    fn failure(&self) -> Option<&Failure> {
        match &self.outcome {
            Outcome::Prepared(_) => None,
            Outcome::Failed(failure) => Some(failure),
        }
    }
}

/// What stopped a `vitri_prepare` call.
enum Refusal {
    /// An argument broke the header's contract; the message says which.
    Argument(String),
    /// vitri refused the request or the input, or the run failed.
    Vitri(VitriError),
}

impl From<VitriError> for Refusal {
    fn from(error: VitriError) -> Self {
        Refusal::Vitri(error)
    }
}

/// The ABI version of the loaded library, which is the `VITRI_ABI_VERSION` it
/// was built with.
#[unsafe(no_mangle)]
pub extern "C" fn vitri_abi_version() -> u32 {
    VITRI_ABI_VERSION
}

/// The version of vitri inside the library, such as `0.2.0`, as a
/// NUL-terminated string. The string is static: the caller does not free it,
/// and it stays valid for as long as the library is loaded.
#[unsafe(no_mangle)]
pub extern "C" fn vitri_version() -> *const c_char {
    static VERSION: OnceLock<CString> = OnceLock::new();
    catch_unwind(|| {
        VERSION
            .get_or_init(|| CString::new(request::VERSION).unwrap_or_default())
            .as_ptr()
    })
    .unwrap_or(c"".as_ptr())
}

/// What this build accepts, as the JSON object tagged
/// `"format": "vitri-capabilities-v1"`: the request keys, the modes, component
/// policies and vtree spec bases, the candidate ceiling and the preprocessing
/// stages this build carries. The `vitri::request::Capabilities` documentation
/// defines each field.
///
/// Returns a NUL-terminated string that the caller owns and releases with
/// `vitri_string_free`, and writes its length, not counting the NUL, to `*len`
/// when `len` is not null. Returns null, with `*len` set to 0, if vitri
/// panicked.
///
/// # Safety
///
/// `len` must be null or point to storage for one `size_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitri_capabilities_json(len: *mut usize) -> *mut c_char {
    let json = catch_unwind(request::capabilities_json)
        .ok()
        .and_then(|json| CString::new(json).ok());
    let (text, n) = json.map_or((ptr::null_mut(), 0), |json| {
        let n = json.as_bytes().len();
        (json.into_raw(), n)
    });
    if !len.is_null() {
        unsafe { len.write(n) };
    }
    text
}

/// Release a string returned by `vitri_capabilities_json`. Null is ignored.
///
/// # Safety
///
/// `text` must be null or a string `vitri_capabilities_json` returned that has
/// not been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitri_string_free(text: *mut c_char) {
    if !text.is_null() {
        drop(unsafe { CString::from_raw(text) });
    }
}

/// Run vitri over DIMACS text with the settings in a JSON request, and return
/// the bundle `vitri -o DIR` would write for the same input and settings, held
/// in memory.
///
/// `dimacs` points to `dimacs_len` bytes of DIMACS CNF text; it need not end
/// in a NUL. It may be null when `dimacs_len` is 0.
///
/// `request` points to `request_len` bytes of UTF-8 JSON, which need not end
/// in a NUL: an object with any of the keys `format`, `mode`, `vtree`,
/// `budget_ms`, `components`, `candidates`, `simplify`, `arjun` and `dot`. The
/// `vitri::request::Request` documentation defines them, and
/// `vitri_capabilities_json` lists them with their accepted values. A key left
/// out keeps its default; an unknown key is refused. A null `request` with
/// `request_len` 0 is the request with every key left out. A setting the
/// request leaves out takes the library default, whatever the environment
/// says; `docs/env.md` lists the `VITRI_*` variables a run still reads.
///
/// Returns `VITRI_OK` or a `VITRI_ERROR_` code. Unless `out` is null, `*out`
/// receives a new result whatever the code, and the code equals
/// `vitri_result_code(*out)`: the bundle on success, the error kind and
/// message otherwise. The caller owns `*out` and releases it with
/// `vitri_result_free`. A null `out` returns `VITRI_ERROR_INVALID_ARGUMENT`
/// and produces no result. vitri keeps no reference to `dimacs` or `request`
/// after returning.
///
/// Calls run one at a time: a call made while another thread's call is
/// running waits for it to finish.
///
/// `budget_ms` is not a hard limit: vitri checks the deadline between its
/// stages and at points inside them, so a run can end after it. In a process
/// with one thread the Arjun stage runs in a forked child, which is killed
/// shortly after the deadline; otherwise it runs in the calling thread and
/// can overrun. The library's documentation states the rule in full under
/// "Process model". To stop a run at a hard limit, run it in a separate
/// process that can be killed, such as the `vitri` executable.
///
/// # Safety
///
/// `dimacs` must be null or point to `dimacs_len` readable bytes, `request`
/// must be null or point to `request_len` readable bytes, neither may change
/// during the call, and `out` must be null or point to storage for one
/// pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitri_prepare(
    dimacs: *const u8,
    dimacs_len: usize,
    request: *const c_char,
    request_len: usize,
    out: *mut *mut vitri_result,
) -> vitri_code {
    if out.is_null() {
        return VITRI_ERROR_INVALID_ARGUMENT;
    }
    let result = answer(|| {
        let _one_at_a_time = ONE_CALL.lock().unwrap_or_else(PoisonError::into_inner);
        match unsafe { prepare(dimacs, dimacs_len, request.cast(), request_len) } {
            Ok(prepared) => vitri_result::prepared(prepared),
            Err(Refusal::Argument(message)) => {
                vitri_result::failed(VITRI_ERROR_INVALID_ARGUMENT, "argument", message)
            }
            Err(Refusal::Vitri(error)) => {
                vitri_result::failed(code_of(&error), error.kind().token(), error.to_string())
            }
        }
    });
    let code = result.code;
    unsafe { out.write(Box::into_raw(Box::new(result))) };
    code
}

/// Release a result. Null is ignored.
///
/// # Safety
///
/// `result` must be null or a result `vitri_prepare` produced that has not
/// been freed. Every buffer borrowed from it becomes invalid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitri_result_free(result: *mut vitri_result) {
    if !result.is_null() {
        drop(unsafe { Box::from_raw(result) });
    }
}

/// The code `vitri_prepare` returned with this result: `VITRI_OK` or a
/// `VITRI_ERROR_` value. `VITRI_ERROR_INVALID_ARGUMENT` for a null `result`.
///
/// # Safety
///
/// `result` must be null or a live result from `vitri_prepare`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitri_result_code(result: *const vitri_result) -> vitri_code {
    unsafe { result.as_ref() }.map_or(VITRI_ERROR_INVALID_ARGUMENT, |result| result.code)
}

/// How the run ended: `built` when it built a vtree, `fully_resolved` or
/// `refuted` when preprocessing settled the formula and there is no vtree to
/// build. Only a `built` result has vtree files. This is the `status` field of
/// the summary.
///
/// Writes the string's length to `*len` when `len` is not null. The bytes
/// belong to `result`: they stay valid and unchanged until
/// `vitri_result_free(result)`, and are followed by a NUL that `*len` does not
/// count. Null, with `*len` set to 0, for a failed or null `result`.
///
/// # Safety
///
/// `result` must be null or a live result from `vitri_prepare`, and `len` must
/// be null or point to storage for one `size_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitri_result_status(
    result: *const vitri_result,
    len: *mut usize,
) -> *const c_char {
    let run = unsafe { result.as_ref() }.and_then(vitri_result::run);
    unsafe { lend(run.map(|run| &run.status), len) }.cast()
}

/// The run's summary: the JSON object tagged `"format": "vitri-result-v1"`,
/// with the run's status, the formula's size before and after preprocessing,
/// the count lift, what each stage did, the vtree's size, the settings after
/// defaults were applied, and the bundle's paths in order. The
/// `vitri::request::Summary` documentation defines each field.
///
/// Writes the text's length to `*len` when `len` is not null. The bytes belong
/// to `result`: they stay valid and unchanged until
/// `vitri_result_free(result)`, and are followed by a NUL that `*len` does not
/// count. Null, with `*len` set to 0, for a failed or null `result`.
///
/// # Safety
///
/// `result` must be null or a live result from `vitri_prepare`, and `len` must
/// be null or point to storage for one `size_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitri_result_summary_json(
    result: *const vitri_result,
    len: *mut usize,
) -> *const c_char {
    let run = unsafe { result.as_ref() }.and_then(vitri_result::run);
    unsafe { lend(run.map(|run| &run.summary), len) }.cast()
}

/// How many files the bundle holds. 0 for a failed or null `result`.
///
/// # Safety
///
/// `result` must be null or a live result from `vitri_prepare`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitri_result_file_count(result: *const vitri_result) -> usize {
    let run = unsafe { result.as_ref() }.and_then(vitri_result::run);
    run.map_or(0, |run| run.files.len())
}

/// The path of file `index`, counting from 0, relative to the bundle
/// directory and `/`-separated, such as `reduced.cnf` or
/// `components/comp000.vtree`. The files come in the order the summary lists
/// them.
///
/// Writes the path's length to `*len` when `len` is not null. The bytes belong
/// to `result`: they stay valid and unchanged until
/// `vitri_result_free(result)`, and are followed by a NUL that `*len` does not
/// count. Null, with `*len` set to 0, for a failed or null `result` or an
/// `index` not below `vitri_result_file_count(result)`.
///
/// # Safety
///
/// `result` must be null or a live result from `vitri_prepare`, and `len` must
/// be null or point to storage for one `size_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitri_result_file_path(
    result: *const vitri_result,
    index: usize,
    len: *mut usize,
) -> *const c_char {
    let file = unsafe { file_at(result, index) };
    unsafe { lend(file.map(|file| &file.path), len) }.cast()
}

/// The contents of file `index`, counting from 0: the bytes `vitri -o DIR`
/// writes to that file's path.
///
/// Writes the length to `*len` when `len` is not null. The bytes belong to
/// `result`: they stay valid and unchanged until `vitri_result_free(result)`,
/// and are followed by a NUL that `*len` does not count. Null, with `*len` set
/// to 0, for a failed or null `result` or an `index` not below
/// `vitri_result_file_count(result)`; an empty file is a non-null pointer with
/// length 0.
///
/// # Safety
///
/// `result` must be null or a live result from `vitri_prepare`, and `len` must
/// be null or point to storage for one `size_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitri_result_file_contents(
    result: *const vitri_result,
    index: usize,
    len: *mut usize,
) -> *const u8 {
    let file = unsafe { file_at(result, index) };
    unsafe { lend(file.map(|file| &file.contents), len) }
}

/// The error's kind as one lowercase word: `config`, `spec`, `env`, `input`,
/// `mismatch`, `construction` or `io` from vitri, `argument` for a broken
/// argument contract, or `panic`. Each has its own `VITRI_ERROR_` code.
///
/// Writes the word's length to `*len` when `len` is not null. The bytes belong
/// to `result`: they stay valid and unchanged until
/// `vitri_result_free(result)`, and are followed by a NUL that `*len` does not
/// count. Null, with `*len` set to 0, for a successful or null `result`.
///
/// # Safety
///
/// `result` must be null or a live result from `vitri_prepare`, and `len` must
/// be null or point to storage for one `size_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitri_result_error_kind(
    result: *const vitri_result,
    len: *mut usize,
) -> *const c_char {
    let failure = unsafe { result.as_ref() }.and_then(vitri_result::failure);
    unsafe { lend(failure.map(|failure| &failure.kind), len) }.cast()
}

/// The error message: UTF-8 text saying what went wrong, which names the
/// request key, setting or input line at fault where there is one.
///
/// Writes the message's length to `*len` when `len` is not null. The bytes
/// belong to `result`: they stay valid and unchanged until
/// `vitri_result_free(result)`, and are followed by a NUL that `*len` does not
/// count. Null, with `*len` set to 0, for a successful or null `result`.
///
/// # Safety
///
/// `result` must be null or a live result from `vitri_prepare`, and `len` must
/// be null or point to storage for one `size_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitri_result_error_message(
    result: *const vitri_result,
    len: *mut usize,
) -> *const c_char {
    let failure = unsafe { result.as_ref() }.and_then(vitri_result::failure);
    unsafe { lend(failure.map(|failure| &failure.message), len) }.cast()
}

/// Run `body`, turning a panic into a result that carries its message.
fn answer(body: impl FnOnce() -> vitri_result) -> vitri_result {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or_else(|payload| {
        vitri_result::failed(VITRI_ERROR_PANIC, "panic", panic_message(&*payload))
    })
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    let what = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str));
    match what {
        Some(what) => format!("vitri panicked: {what}"),
        None => "vitri panicked".to_string(),
    }
}

/// Held for the whole of every `vitri_prepare` call: nothing in vitri is
/// known to be safe to enter from two threads at once.
static ONE_CALL: Mutex<()> = Mutex::new(());

/// The status code for an error vitri returned. Exhaustive, so a kind added
/// to vitri gets a code here before this crate builds again.
fn code_of(error: &VitriError) -> vitri_code {
    match error.kind() {
        ErrorKind::Config => VITRI_ERROR_CONFIG,
        ErrorKind::Spec => VITRI_ERROR_SPEC,
        ErrorKind::Env => VITRI_ERROR_ENV,
        ErrorKind::Input => VITRI_ERROR_INPUT,
        ErrorKind::Mismatch => VITRI_ERROR_MISMATCH,
        ErrorKind::Construction => VITRI_ERROR_CONSTRUCTION,
        ErrorKind::Io => VITRI_ERROR_IO,
    }
}

/// Parse the request and run it over the DIMACS bytes.
///
/// # Safety
///
/// As `vitri_prepare`, for the four arguments.
unsafe fn prepare(
    dimacs: *const u8,
    dimacs_len: usize,
    request: *const u8,
    request_len: usize,
) -> Result<Prepared, Refusal> {
    let dimacs = unsafe { borrow(dimacs, dimacs_len, "dimacs") }?.unwrap_or_default();
    let request = match unsafe { borrow(request, request_len, "request") }? {
        None => Request::default(),
        Some(bytes) => {
            let text = std::str::from_utf8(bytes)
                .map_err(|e| VitriError::config(format!("the request is not UTF-8 text: {e}")))?;
            Request::from_json(text)?
        }
    };
    Ok(request::prepare(dimacs, &request)?)
}

/// The `len` bytes at `data`, or `None` for a null `data` with `len` 0.
/// `name` is the argument's name in the header, for the refusal.
///
/// # Safety
///
/// `data` must be null or point to `len` readable bytes that do not change
/// while the slice is in use.
unsafe fn borrow<'a>(data: *const u8, len: usize, name: &str) -> Result<Option<&'a [u8]>, Refusal> {
    if data.is_null() {
        return match len {
            0 => Ok(None),
            _ => Err(Refusal::Argument(format!(
                "{name} is null but {name}_len is {len}"
            ))),
        };
    }
    if isize::try_from(len).is_err() {
        return Err(Refusal::Argument(format!(
            "{name}_len is {len}, larger than any buffer"
        )));
    }
    Ok(Some(unsafe { slice::from_raw_parts(data, len) }))
}

/// File `index` of a successful result, if there is one.
///
/// # Safety
///
/// `result` must be null or a live result from `vitri_prepare`.
unsafe fn file_at<'a>(result: *const vitri_result, index: usize) -> Option<&'a File> {
    let run = unsafe { result.as_ref() }.and_then(vitri_result::run)?;
    run.files.get(index)
}

/// Point C at `text`, and write its length to `len` when `len` is not null.
/// Null and 0 when there is no text.
///
/// # Safety
///
/// `len` must be null or point to storage for one `size_t`.
unsafe fn lend(text: Option<&Text>, len: *mut usize) -> *const u8 {
    let (at, n) = text.map_or((ptr::null(), 0), |text| {
        (text.0.as_ptr(), text.bytes().len())
    });
    if !len.is_null() {
        unsafe { len.write(n) };
    }
    at
}

#[cfg(test)]
mod tests;
