use pyo3::prelude::*;
use pyo3::types::*;

mod errors;
mod format;
mod graph;
mod integrity;

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
    hmac_key: Option<integrity::HmacKey>,
) -> PyResult<PyObject> {
    if protocol != 5 {
        return Err(pyo3::exceptions::PyValueError::new_err(format!(
            "Unsupported protocol {}; only protocol 5 is supported",
            protocol
        )));
    }
    let hmac_key = integrity::key_slice(&hmac_key);
    check_hmac_key(hmac_key)?;
    let _ = schema_version;

    Ok(encode(py, obj, hmac_key)?.into_any().unbind())
}

fn check_hmac_key(hmac_key: Option<&[u8]>) -> PyResult<()> {
    if matches!(hmac_key, Some(k) if k.is_empty()) {
        return Err(pyo3::exceptions::PyValueError::new_err(integrity::MSG_EMPTY_KEY));
    }
    Ok(())
}

/// Serializes `obj`; with a key, the payload is signed (see `integrity`).
fn encode<'py>(
    py: Python<'py>,
    obj: &Bound<'py, PyAny>,
    hmac_key: Option<&[u8]>,
) -> PyResult<Bound<'py, PyBytes>> {
    let encoded: encoder::Encoded<'_> = Walker::new(py)
        .walk(obj)
        .map_err(|e| errors::map_unsupported_type(py, e))?;
    let unsigned = encoded.into_pybytes(py)?;
    match hmac_key {
        None => Ok(unsigned),
        Some(key) => {
            let raw = unsigned.as_bytes();
            let signed = if raw.len() >= GIL_RELEASE_THRESHOLD {
                py.allow_threads(|| integrity::sign(raw, key))
            } else {
                integrity::sign(raw, key)
            };
            Ok(PyBytes::new(py, &signed))
        }
    }
}

/// Inputs at least this large are parsed with the GIL released; for smaller
/// ones the release/re-acquire costs more than it frees up.
const GIL_RELEASE_THRESHOLD: usize = 64 * 1024;

/// How a decode call treats the input buffer.
#[derive(Clone, Copy)]
struct LoadOpts<'k> {
    hmac_key: Option<&'k [u8]>,
    /// `true` for `loads` (the buffer holds one payload, trailing bytes are
    /// ignored); `false` for stream reads (`_loads_prefix`, `load`).
    whole_buffer: bool,
}

/// Decodes one payload from the start of `bytes`; returns the object and the
/// number of bytes the payload used.
fn loads_from_slice(
    py: Python<'_>,
    bytes: &[u8],
    allowlist: Option<&Bound<'_, PySet>>,
    classes: Option<&Bound<'_, PyAny>>,
    opts: LoadOpts<'_>,
) -> PyResult<(PyObject, usize)> {
    check_hmac_key(opts.hmac_key)?;
    let run = || integrity::decode_one(bytes, opts.hmac_key, opts.whole_buffer);
    let result = if bytes.len() >= GIL_RELEASE_THRESHOLD {
        py.allow_threads(run)
    } else {
        run()
    };
    let (decoded, used) = result.map_err(|f| match f {
        integrity::Failure::Decode(e) => PyErr::from(e),
        integrity::Failure::Hmac(msg) => errors::hmac_error(msg),
    })?;

    if let Some(al) = allowlist {
        for ti in &decoded.types {
            if !al.contains(ti.name)? {
                return Err(errors::unsafe_type_error(format!(
                    "Type '{}' is not in the allowlist",
                    ti.name
                )));
            }
        }
    }

    let classes = classes.map(decoder::class_map).transpose()?;
    Ok((decoder::reconstruct(py, &decoded, classes.as_ref())?.unbind(), used))
}

/// Decodes any bytes-like object. `bytes` is borrowed directly (immutable);
/// anything else (bytearray, memoryview, other buffers) is first copied into
/// an owned buffer, since it could be mutated while the GIL is released or
/// while Python code runs during reconstruction.
fn loads_bytes_like(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
    allowlist: Option<&Bound<'_, PySet>>,
    classes: Option<&Bound<'_, PyAny>>,
    err_prefix: &str,
    opts: LoadOpts<'_>,
) -> PyResult<(PyObject, usize)> {
    if let Ok(b) = data.downcast::<PyBytes>() {
        return loads_from_slice(py, b.as_bytes(), allowlist, classes, opts);
    }
    if let Ok(ba) = data.downcast::<PyByteArray>() {
        let owned = ba.to_vec();
        return loads_from_slice(py, &owned, allowlist, classes, opts);
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
    loads_from_slice(py, b.as_bytes(), allowlist, classes, opts)
}

#[pyfunction]
#[pyo3(signature = (data, *, allowlist=None, hmac_key=None, classes=None))]
fn loads(
    py: Python,
    data: &Bound<'_, PyAny>,
    allowlist: Option<&Bound<'_, PySet>>,
    hmac_key: Option<integrity::HmacKey>,
    classes: Option<&Bound<'_, PyAny>>,
) -> PyResult<PyObject> {
    let opts = LoadOpts { hmac_key: integrity::key_slice(&hmac_key), whole_buffer: true };
    Ok(loads_bytes_like(py, data, allowlist, classes, "", opts)?.0)
}

/// Decodes exactly one payload from the start of `data` (or of
/// `data[offset:]` for `bytes` input, without copying) and returns
/// `(obj, consumed_bytes)` (header + tables + records + HMAC tag if signed).
/// An incomplete payload raises `DecodeError`.
#[pyfunction]
#[pyo3(signature = (data, *, allowlist=None, hmac_key=None, offset=0, classes=None))]
fn _loads_prefix(
    py: Python,
    data: &Bound<'_, PyAny>,
    allowlist: Option<&Bound<'_, PySet>>,
    hmac_key: Option<integrity::HmacKey>,
    offset: usize,
    classes: Option<&Bound<'_, PyAny>>,
) -> PyResult<(PyObject, usize)> {
    let opts = LoadOpts { hmac_key: integrity::key_slice(&hmac_key), whole_buffer: false };
    if offset == 0 {
        return loads_bytes_like(py, data, allowlist, classes, "", opts);
    }
    let b = data.downcast::<PyBytes>().map_err(|_| {
        pyo3::exceptions::PyTypeError::new_err("offset requires a bytes object")
    })?;
    let bytes = b.as_bytes();
    if offset > bytes.len() {
        return Err(pyo3::exceptions::PyValueError::new_err("offset out of range"));
    }
    loads_from_slice(py, &bytes[offset..], allowlist, classes, opts)
}

#[pyfunction]
/// Like `pickle.dump`: writes `dumps(obj, ...)` to `file`. Unknown keyword
/// arguments raise `TypeError` (a misspelled `hmac_key` must not silently
/// write an unsigned payload). `fix_imports`/`buffer_callback` are accepted
/// for pickle compatibility and ignored; `protocol=None` means 5.
#[pyo3(signature = (obj, file, protocol=None, *, fix_imports=true, buffer_callback=None, schema_version=None, hmac_key=None))]
#[allow(clippy::too_many_arguments)]
fn dump(
    py: Python,
    obj: &Bound<'_, PyAny>,
    file: &Bound<'_, PyAny>,
    protocol: Option<u8>,
    fix_imports: bool,
    buffer_callback: Option<&Bound<'_, PyAny>>,
    schema_version: Option<u32>,
    hmac_key: Option<integrity::HmacKey>,
) -> PyResult<()> {
    let _ = (fix_imports, buffer_callback);
    let payload = dumps(py, obj, protocol.unwrap_or(5), schema_version, hmac_key)?;
    file.call_method1(pyo3::intern!(py, "write"), (payload,))?;
    Ok(())
}

/// Reads one payload from `file`. The format has no length prefix, so the
/// rest of the stream is read into memory; if the file is seekable it is then
/// positioned just past the payload, so repeated `load` calls return
/// successive records. On non-seekable streams the bytes read after the
/// payload are discarded (use `Unpickler` to read several records).
#[pyfunction]
#[pyo3(signature = (file, *, allowlist=None, hmac_key=None, classes=None))]
fn load(
    py: Python,
    file: &Bound<'_, PyAny>,
    allowlist: Option<&Bound<'_, PySet>>,
    hmac_key: Option<integrity::HmacKey>,
    classes: Option<&Bound<'_, PyAny>>,
) -> PyResult<PyObject> {
    let read_method = file.getattr("read")?;
    let data = read_method.call0()?;
    if data.len().ok() == Some(0) {
        return Err(pyo3::exceptions::PyEOFError::new_err("Ran out of input"));
    }
    let opts = LoadOpts { hmac_key: integrity::key_slice(&hmac_key), whole_buffer: false };
    let (obj, used) =
        loads_bytes_like(py, &data, allowlist, classes, "file.read() did not return bytes: ", opts)?;
    let total = data.len()?;
    if used < total {
        let seekable = match file.getattr(pyo3::intern!(py, "seekable")) {
            Ok(m) => m.call0()?.is_truthy()?,
            Err(_) => false,
        };
        if seekable {
            let back = -((total - used) as i64);
            file.call_method1(pyo3::intern!(py, "seek"), (back, 1))?;
        }
    }
    Ok(obj)
}

#[pymodule]
fn _pysafe_pickle(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(__version, m)?)?;
    m.add_function(wrap_pyfunction!(dumps, m)?)?;
    m.add_function(wrap_pyfunction!(loads, m)?)?;
    m.add_function(wrap_pyfunction!(_loads_prefix, m)?)?;
    m.add_function(wrap_pyfunction!(dump, m)?)?;
    m.add_function(wrap_pyfunction!(load, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
