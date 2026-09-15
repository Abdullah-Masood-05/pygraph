use pyo3::prelude::*;
use pyo3::types::*;

mod format;
mod graph;
mod migration;

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
    let _ = (protocol, schema_version, hmac_key);

    let mut walker = Walker::new(py);
    let _root_id = walker.walk(obj)?;
    let (mut graph, type_registry) = walker.into_parts();

    let encoded = encoder::encode(&mut graph, &type_registry);

    Ok(PyBytes::new(py, &encoded).into())
}

#[pyfunction]
#[pyo3(signature = (data, *, allowlist=None))]
fn loads(
    py: Python,
    data: &Bound<'_, PyBytes>,
    allowlist: Option<&Bound<'_, PySet>>,
) -> PyResult<PyObject> {
    let bytes = data.as_bytes();
    let decoded = decoder::decode(bytes)?;

    if let Some(al) = allowlist {
        for record in &decoded.records {
            if let crate::graph::traversal::Record::Dataclass { type_id, .. } = record {
                let ti = decoded.type_registry.get_type(*type_id).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err(format!("Unknown type_id: {}", type_id))
                })?;
                let type_str = PyString::new(py, &ti.name);
                if !al.contains(&type_str)? {
                    return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                        "Type '{}' is not in the allowlist",
                        ti.name
                    )));
                }
            }
        }
    }

    let root_id = 0u32;
    let obj = decoder::reconstruct(py, &decoded, root_id)?;
    Ok(obj.into())
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
    let mut walker = Walker::new(py);
    let _root_id = walker.walk(obj)?;
    let (mut graph, type_registry) = walker.into_parts();

    let encoded = encoder::encode(&mut graph, &type_registry);

    let write_method = file.getattr("write")?;
    write_method.call1((PyBytes::new(py, &encoded),))?;
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
    let bytes: &Bound<'_, PyBytes> = data.downcast::<PyBytes>().map_err(|_| {
        pyo3::exceptions::PyTypeError::new_err("file.read() did not return bytes")
    })?;
    loads(py, bytes, allowlist)
}

#[pymodule]
fn _pysafe_pickle(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(__version, m)?)?;
    m.add_function(wrap_pyfunction!(dumps, m)?)?;
    m.add_function(wrap_pyfunction!(loads, m)?)?;
    m.add_function(wrap_pyfunction!(dump, m)?)?;
    m.add_function(wrap_pyfunction!(load, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
