//! Python bindings for vitri: DIMACS text goes in, and the bundle
//! `vitri --out-dir` writes comes back in memory.
//!
//! Every setting is a key of `vitri::request::Request` under the same name,
//! and every check on a setting is the library's. This crate converts the
//! arguments, calls `vitri::request::prepare`, and converts the result back.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyInt, PyString};
use vitri::bundle::{REDUCED_CNF_NAME, VTREE_NAME};
use vitri::error::ErrorKind;
use vitri::request::{self, Prepared, Request};

// One class per kind of `vitri::VitriError`, whose documentation says what
// each kind covers. The message is the library's own and names the value at
// fault.
create_exception!(
    vitri,
    VitriError,
    PyException,
    "Base class of the errors vitri raises."
);
create_exception!(vitri, ConfigError, VitriError, "The error kind `config`.");
create_exception!(vitri, SpecError, VitriError, "The error kind `spec`.");
create_exception!(vitri, EnvError, VitriError, "The error kind `env`.");
create_exception!(vitri, InputError, VitriError, "The error kind `input`.");
create_exception!(
    vitri,
    MismatchError,
    VitriError,
    "The error kind `mismatch`."
);
create_exception!(
    vitri,
    ConstructionError,
    VitriError,
    "The error kind `construction`."
);
create_exception!(vitri, IoError, VitriError, "The error kind `io`.");

/// The exception class for `error`'s kind, carrying its message.
fn py_error(error: vitri::VitriError) -> PyErr {
    let message = error.to_string();
    match error.kind() {
        ErrorKind::Config => ConfigError::new_err(message),
        ErrorKind::Spec => SpecError::new_err(message),
        ErrorKind::Env => EnvError::new_err(message),
        ErrorKind::Input => InputError::new_err(message),
        ErrorKind::Mismatch => MismatchError::new_err(message),
        ErrorKind::Construction => ConstructionError::new_err(message),
        ErrorKind::Io => IoError::new_err(message),
    }
}

/// `value` as an unsigned integer setting. A negative value, or one past 64
/// bits, is refused in the library's words.
fn non_negative(key: &str, value: Option<&Bound<'_, PyInt>>) -> PyResult<Option<u64>> {
    value
        .map(|value| {
            value
                .extract::<u64>()
                .map_err(|_| py_error(request::refuse_integer(key, value)))
        })
        .transpose()
}

/// Held for the whole of every call into the library: nothing in it is known
/// to be safe to enter from two threads at once.
static ONE_CALL: Mutex<()> = Mutex::new(());

/// Parse `text` with Python's `json` module.
fn json_loads<'py>(py: Python<'py>, text: &str) -> PyResult<Bound<'py, PyAny>> {
    py.import("json")?.call_method1("loads", (text,))
}

/// The outcome of one `vitri.prepare` call: a summary of the run and every
/// file of its bundle.
///
/// The result holds its own copy of the files and stays valid on its own.
/// `summary` and `files` build new Python objects on each access, so keep
/// what they return rather than reading them in a loop.
#[pyclass(module = "vitri", name = "Result", frozen)]
pub struct PrepareResult {
    inner: Prepared,
}

impl PrepareResult {
    /// The text of the bundle file at `path`, if the bundle has one.
    fn text(&self, path: &str) -> Option<&str> {
        let file = self.inner.files.iter().find(|file| file.path == path)?;
        Some(std::str::from_utf8(&file.contents).expect("the library writes bundle files as UTF-8"))
    }
}

#[pymethods]
impl PrepareResult {
    /// `"built"`, `"fully_resolved"` or `"refuted"`.
    ///
    /// Only a built run has a vtree. Preprocessing settled the other two on
    /// its own: the count of the input is the lift in `summary` for a fully
    /// resolved formula, and zero for a refuted one.
    #[getter]
    fn status(&self) -> &'static str {
        self.inner.summary.status
    }

    /// What the run did, as a dict tagged `"vitri-result-v1"` in its
    /// `"format"` key: the status and mode, the sizes of the input and the
    /// reduced formula, the count lift, each preprocessing stage's outcome,
    /// the vtree's size (`None` unless built), the settings the run used after
    /// defaults and detection, and the file paths in the order they were
    /// written.
    #[getter]
    fn summary<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        json_loads(py, &self.inner.summary.to_json())
    }

    /// Every file of the bundle as a dict from path to bytes, in the order the
    /// files were written.
    ///
    /// The paths and the bytes are those `vitri --out-dir` writes for the same
    /// settings; `/` separates directories. Every file is UTF-8 text.
    #[getter]
    fn files<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let files = PyDict::new(py);
        for file in &self.inner.files {
            files.set_item(&file.path, PyBytes::new(py, &file.contents))?;
        }
        Ok(files)
    }

    /// The text of `reduced.cnf`, the formula to compile.
    #[getter]
    fn reduced_cnf(&self) -> &str {
        self.text(REDUCED_CNF_NAME)
            .expect("every bundle holds the reduced formula")
    }

    /// The text of `vtree.vtree`, the vtree over the whole reduced formula,
    /// or `None` when `status` is not `"built"`.
    #[getter]
    fn vtree(&self) -> Option<&str> {
        self.text(VTREE_NAME)
    }

    /// Write every file of the bundle under `directory`, creating it and its
    /// subdirectories as needed.
    ///
    /// A file already at one of the bundle's paths is replaced; any other file
    /// in the directory is left alone, as `vitri --out-dir` leaves it. Raises
    /// `IoError` naming the path that could not be written. The interpreter
    /// lock is released while the files are written.
    fn write(&self, py: Python<'_>, directory: PathBuf) -> PyResult<()> {
        py.detach(|| self.inner.write_to_dir(&directory))
            .map_err(py_error)
    }

    fn __repr__(&self) -> String {
        format!(
            "<vitri.Result: {}, {} files>",
            self.inner.summary.status,
            self.inner.files.len()
        )
    }
}

/// Preprocess a DIMACS CNF and build a vtree over what is left, returning the
/// bundle `vitri --out-dir` writes.
///
/// `dimacs` is the formula as text or bytes, with its `c t`, `c p show` and
/// `c p weight` lines if it has them. The keywords are the command line's
/// settings under the request's key names, and a keyword left as `None` keeps
/// the library default:
///
/// - `mode`: what preprocessing preserves, one of `capabilities()["modes"]`;
///   detected from the formula's headers when left out.
/// - `vtree`: the vtree spec string; `capabilities()["default_vtree"]` when
///   left out.
/// - `budget_ms`: the wall-clock budget for the whole run, in milliseconds;
///   unbounded when left out.
/// - `components`: one of `capabilities()["components"]`, whether each
///   component of the formula gets its own vtree.
/// - `candidates`: how many ranked vtree candidates to keep per built vtree.
/// - `simplify`, `arjun`: `False` switches that preprocessing stage off.
///   Setting either, to either value, under a mode whose preprocessing has no
///   such stage raises `ConfigError`; `capabilities()["mode_stages"]` lists
///   the stages of each mode.
/// - `dot`: write a Graphviz `.dot` file beside every `.vtree`.
///
/// No environment variable changes these settings. The variables the vendored
/// preprocessing stack reads itself still apply.
///
/// Raises `ConfigError` or `SpecError` for a setting the library refuses,
/// including a negative `budget_ms` or `candidates`, `InputError` for DIMACS
/// that does not parse, and another `VitriError` subclass for a run that
/// fails. An unknown keyword or an argument of the wrong Python type raises
/// `TypeError`.
///
/// The interpreter lock is released while the library runs, so other Python
/// threads keep running; calls from several threads run one at a time. A
/// `KeyboardInterrupt` is raised only after the call returns.
///
/// `budget_ms` is checked between the run's steps, so a step can run past it.
/// In a process with one thread the Arjun stage runs in a forked child, which
/// runs no Python code and is killed shortly after the budget has passed; with
/// more threads, or with `SIGCHLD` ignored, the stage runs in the calling
/// thread and can overrun. The library's documentation states the rule in
/// full under "Process model". For a hard wall-clock limit, make the call in a
/// separate process started with the `spawn` method and kill that process at
/// the limit, as `examples/hard_timeout.py` does.
#[pyfunction]
#[pyo3(signature = (
    dimacs,
    *,
    mode = None,
    vtree = None,
    budget_ms = None,
    components = None,
    candidates = None,
    simplify = None,
    arjun = None,
    dot = false
))]
#[allow(clippy::too_many_arguments)]
fn prepare(
    py: Python<'_>,
    dimacs: &Bound<'_, PyAny>,
    mode: Option<&str>,
    vtree: Option<String>,
    budget_ms: Option<Bound<'_, PyInt>>,
    components: Option<&str>,
    candidates: Option<Bound<'_, PyInt>>,
    simplify: Option<bool>,
    arjun: Option<bool>,
    dot: bool,
) -> PyResult<PrepareResult> {
    let mut settings = Request::default();
    settings.mode = mode
        .map(|token| request::parse_mode("mode", token))
        .transpose()
        .map_err(py_error)?;
    settings.vtree = vtree;
    settings.budget_ms = non_negative("budget_ms", budget_ms.as_ref())?;
    settings.components = components
        .map(|token| request::parse_components("components", token))
        .transpose()
        .map_err(py_error)?;
    settings.candidates = non_negative("candidates", candidates.as_ref())?;
    settings.simplify = simplify;
    settings.arjun = arjun;
    settings.dot = dot;

    let text;
    let bytes: &[u8] = if let Ok(bytes) = dimacs.cast::<PyBytes>() {
        bytes.as_bytes()
    } else if let Ok(string) = dimacs.cast::<PyString>() {
        text = string.to_str()?;
        text.as_bytes()
    } else {
        return Err(PyTypeError::new_err(format!(
            "dimacs expects str or bytes, got {}",
            dimacs.get_type().name()?
        )));
    };

    let inner = py
        .detach(|| {
            let _one_at_a_time = ONE_CALL.lock().unwrap_or_else(PoisonError::into_inner);
            request::prepare(bytes, &settings)
        })
        .map_err(py_error)?;
    Ok(PrepareResult { inner })
}

/// What this build accepts, as a dict tagged `"vitri-capabilities-v1"` in its
/// `"format"` key: the library version, the request keys, the modes and
/// component policies, the default vtree spec and the spec bases, the
/// candidate ceiling, and which preprocessing stages run unless switched off.
#[pyfunction]
fn capabilities(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    json_loads(py, &request::capabilities_json())
}

// `pymodule` expands this function into a Rust module of the same name, so it
// cannot be called `vitri` without shadowing the crate being wrapped. `name`
// is what Python imports.
/// CNF preprocessing and vtree construction: a DIMACS formula goes in; a
/// reduced formula, the arithmetic that lifts its count back, and a vtree come
/// out.
#[pymodule]
#[pyo3(name = "vitri")]
fn python_module(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    module.add("VitriError", py.get_type::<VitriError>())?;
    module.add("ConfigError", py.get_type::<ConfigError>())?;
    module.add("SpecError", py.get_type::<SpecError>())?;
    module.add("EnvError", py.get_type::<EnvError>())?;
    module.add("InputError", py.get_type::<InputError>())?;
    module.add("MismatchError", py.get_type::<MismatchError>())?;
    module.add("ConstructionError", py.get_type::<ConstructionError>())?;
    module.add("IoError", py.get_type::<IoError>())?;
    module.add_class::<PrepareResult>()?;
    module.add_function(wrap_pyfunction!(prepare, module)?)?;
    module.add_function(wrap_pyfunction!(capabilities, module)?)?;
    // The `__init__.py` maturin generates around the extension re-exports
    // exactly this list.
    module.add(
        "__all__",
        vec![
            "ConfigError",
            "ConstructionError",
            "EnvError",
            "InputError",
            "IoError",
            "MismatchError",
            "Result",
            "SpecError",
            "VitriError",
            "__version__",
            "capabilities",
            "prepare",
        ],
    )
}
