use pyo3::prelude::*;

#[pyfunction]
fn __version() -> &'static str {
    "0.1.0"
}

#[pymodule]
fn _pygraph(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(__version, m)?)?;
    m.add("__version__", "0.1.0")?;
    Ok(())
}
