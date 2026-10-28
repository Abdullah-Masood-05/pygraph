//! Raising the package's own exception classes from Rust.
//!
//! The classes live in `pysafe_pickle.exceptions` (pure Python) and use
//! multiple inheritance (e.g. `UnsafeTypeError(PySafePickleError, TypeError)`),
//! so callers catching the builtin base keep working. Each class is imported
//! once and cached; if the import ever fails (e.g. the extension is loaded
//! without its Python package) the builtin base exception is used instead.

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::sync::GILOnceCell;
use pyo3::types::PyType;

static UNSAFE_TYPE_ERROR: GILOnceCell<Py<PyType>> = GILOnceCell::new();
static HMAC_ERROR: GILOnceCell<Py<PyType>> = GILOnceCell::new();
static DECODE_ERROR: GILOnceCell<Py<PyType>> = GILOnceCell::new();

fn class<'py>(
    py: Python<'py>,
    cell: &'static GILOnceCell<Py<PyType>>,
    name: &str,
) -> PyResult<&'py Bound<'py, PyType>> {
    cell.get_or_try_init(py, || {
        let cls = py.import("pysafe_pickle.exceptions")?.getattr(name)?;
        Ok::<_, PyErr>(cls.downcast_into::<PyType>()?.unbind())
    })
    .map(|c| c.bind(py))
}

fn make(
    cell: &'static GILOnceCell<Py<PyType>>,
    name: &str,
    msg: String,
    fallback: fn(String) -> PyErr,
) -> PyErr {
    Python::with_gil(|py| match class(py, cell, name) {
        Ok(cls) => PyErr::from_type(cls.clone(), msg),
        Err(_) => fallback(msg),
    })
}

/// `UnsafeTypeError` (also a `TypeError`).
pub fn unsafe_type_error(msg: impl Into<String>) -> PyErr {
    make(&UNSAFE_TYPE_ERROR, "UnsafeTypeError", msg.into(), PyTypeError::new_err)
}

/// `HMACError` (also a `ValueError`).
pub fn hmac_error(msg: impl Into<String>) -> PyErr {
    make(&HMAC_ERROR, "HMACError", msg.into(), PyValueError::new_err)
}

/// `DecodeError` (also a `ValueError`).
pub fn decode_error(msg: impl Into<String>) -> PyErr {
    make(&DECODE_ERROR, "DecodeError", msg.into(), PyValueError::new_err)
}

/// Re-raises a serializer "Unsupported type: ..." `TypeError` as
/// `UnsafeTypeError` with the same message; any other error is returned
/// unchanged.
pub fn map_unsupported_type(py: Python<'_>, err: PyErr) -> PyErr {
    if !err.get_type(py).is(&py.get_type::<PyTypeError>()) {
        return err;
    }
    let msg = match err.value(py).str() {
        Ok(s) => s.to_string(),
        Err(_) => return err,
    };
    if !msg.starts_with("Unsupported type") {
        return err;
    }
    unsafe_type_error(msg)
}
