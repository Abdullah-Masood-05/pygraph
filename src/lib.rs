use pyo3::prelude::*;
use pyo3::types::*;

mod format;
mod graph;

use format::encoder;
use format::decoder;
use graph::traversal::Walker;

#[pyfunction]
fn __version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[pyfunction]
#[pyo3(signature = (obj, *, protocol=5, schema_version=None, hmac_key=None))]
fn dumps(
    py: Python,
    obj: &Bound<'_, PyAny>,
    protocol: u8,
    schema_version: Option<u32>,
    hmac_key: Option<&[u8]>,
) -> PyResult<PyObject> {
    if protocol != 5 {
        return Err(pyo3::exceptions::PyValueError::new_err(format!(
            "Unsupported protocol {}; only protocol 5 is supported",
            protocol
        )));
    }
    if hmac_key.is_some() {
        return Err(pyo3::exceptions::PyNotImplementedError::new_err(
            "HMAC signatures are not yet supported",
        ));
    }
    let _ = schema_version;

    let encoded: encoder::Encoded<'_> = Walker::new(py).walk(obj)?;
    Ok(encoded.into_pybytes(py)?.into_any().unbind())
}

/// Inputs at least this large are parsed with the GIL released; for smaller
/// ones the release/re-acquire costs more than it frees up.
const GIL_RELEASE_THRESHOLD: usize = 64 * 1024;

fn loads_from_slice(
    py: Python<'_>,
    bytes: &[u8],
    allowlist: Option<&Bound<'_, PySet>>,
) -> PyResult<PyObject> {
    let decoded = if bytes.len() >= GIL_RELEASE_THRESHOLD {
        py.allow_threads(|| decoder::decode(bytes))?
    } else {
        decoder::decode(bytes)?
    };

    if let Some(al) = allowlist {
        for ti in &decoded.types {
            if !al.contains(ti.name)? {
                return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                    "Type '{}' is not in the allowlist",
                    ti.name
                )));
            }
        }
    }

    Ok(decoder::reconstruct(py, &decoded)?.unbind())
}

/// Decodes any bytes-like object. `bytes` is borrowed directly (immutable);
/// anything else (bytearray, memoryview, other buffers) is first copied into
/// an owned buffer, since it could be mutated while the GIL is released or
/// while Python code runs during reconstruction.
fn loads_bytes_like(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
    allowlist: Option<&Bound<'_, PySet>>,
    err_prefix: &str,
) -> PyResult<PyObject> {
    if let Ok(b) = data.downcast::<PyBytes>() {
        return loads_from_slice(py, b.as_bytes(), allowlist);
    }
    if let Ok(ba) = data.downcast::<PyByteArray>() {
        let owned = ba.to_vec();
        return loads_from_slice(py, &owned, allowlist);
    }
    let copied = PyMemoryView::from(data)
        .and_then(|mv| mv.call_method0("tobytes"))
        .map_err(|_| {
            let tname = data
                .get_type()
                .name()
                .map(|n| n.to_string())
                .unwrap_or_else(|_| "?".to_string());
            pyo3::exceptions::PyTypeError::new_err(format!(
                "{}a bytes-like object is required, not '{}'",
                err_prefix, tname
            ))
        })?;
    let b = copied.downcast::<PyBytes>()?;
    loads_from_slice(py, b.as_bytes(), allowlist)
}

#[pyfunction]
fn _decode_only(data: &Bound<'_, PyBytes>) -> PyResult<usize> {
    let d = decoder::decode(data.as_bytes())?;
    Ok(d.strings.len())
}

#[pyfunction]
fn _floor(py: Python<'_>, n: usize, kind: u8) -> PyResult<PyObject> {
    match kind {
        0 => Ok(PyList::new(py, (0..n).map(|_| py.None()))?.into_any().unbind()),
        1 => {
            let v: Vec<Bound<'_, PyAny>> = (0..n).map(|_| py.None().into_bound(py)).collect();
            Ok(PyList::new(py, v)?.into_any().unbind())
        }
        2 => {
            let l = PyList::empty(py);
            for _ in 0..n {
                l.append(py.None())?;
            }
            Ok(l.into_any().unbind())
        }
        _ => {
            let v: Vec<Bound<'_, PyAny>> = (0..n as i64).map(|i| i.into_pyobject(py).unwrap().into_any()).collect();
            Ok(PyList::new(py, v)?.into_any().unbind())
        }
    }
}

#[pyfunction]
#[pyo3(signature = (data, *, allowlist=None))]
fn loads(
    py: Python,
    data: &Bound<'_, PyAny>,
    allowlist: Option<&Bound<'_, PySet>>,
) -> PyResult<PyObject> {
    loads_bytes_like(py, data, allowlist, "")
}

#[pyfunction]
#[pyo3(signature = (obj, file, **kwargs))]
fn dump(
    py: Python,
    obj: &Bound<'_, PyAny>,
    file: &Bound<'_, PyAny>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<()> {
    let _ = kwargs;
    let encoded: encoder::Encoded<'_> = Walker::new(py).walk(obj)?;
    file.call_method1(pyo3::intern!(py, "write"), (encoded.into_pybytes(py)?,))?;
    Ok(())
}

#[pyfunction]
#[pyo3(signature = (file, *, allowlist=None))]
fn load(
    py: Python,
    file: &Bound<'_, PyAny>,
    allowlist: Option<&Bound<'_, PySet>>,
) -> PyResult<PyObject> {
    let read_method = file.getattr("read")?;
    let data = read_method.call0()?;
    loads_bytes_like(py, &data, allowlist, "file.read() did not return bytes: ")
}

#[pymodule]
fn _pysafe_pickle(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(__version, m)?)?;
    m.add_function(wrap_pyfunction!(dumps, m)?)?;
    m.add_function(wrap_pyfunction!(loads, m)?)?;
    m.add_function(wrap_pyfunction!(dump, m)?)?;
    m.add_function(wrap_pyfunction!(load, m)?)?;
    m.add_function(wrap_pyfunction!(_decode_only, m)?)?;
    m.add_function(wrap_pyfunction!(_floor, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
