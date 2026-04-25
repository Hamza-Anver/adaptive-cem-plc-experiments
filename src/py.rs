use pyo3::prelude::*;
use pyo3::exceptions::PyException;
use crate::common::{PlcVarMeta, PlcVarType};

/// Python module for LibAFL Sandbox
#[pymodule]
pub fn libafl_sandbox(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<PyPlcVarType>()?;
    m.add_class::<PyPlcVarMeta>()?;
    m.add_class::<PyTargetSession>()?;
    m.add_function(wrap_pyfunction!(py_input_size, m)?)?;
    Ok(())
}

/// Python-facing enum for variable types
#[pyclass]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PyPlcVarType {
    UINT8 = 0,
    UINT16 = 1,
    UINT32 = 2,
    BOOL = 3,
    FLOAT = 4,
}

impl From<PlcVarType> for PyPlcVarType {
    fn from(t: PlcVarType) -> Self {
        match t {
            PlcVarType::UINT8 => PyPlcVarType::UINT8,
            PlcVarType::UINT16 => PyPlcVarType::UINT16,
            PlcVarType::UINT32 => PyPlcVarType::UINT32,
            PlcVarType::BOOL => PyPlcVarType::BOOL,
            PlcVarType::FLOAT => PyPlcVarType::FLOAT,
        }
    }
}

#[pyclass]
pub struct PyPlcVarMeta {
    pub name: String,
    pub var_type: PyPlcVarType,
    pub size: usize,
    pub offset: usize,
}

#[pymethods]
impl PyPlcVarMeta {
    #[new]
    fn new(name: String, var_type: PyPlcVarType, size: usize, offset: usize) -> Self {
        PyPlcVarMeta {
            name,
            var_type,
            size,
            offset,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "PlcVarMeta(name='{}', type={:?}, size={}, offset={})",
            self.name, self.var_type, self.size, self.offset
        )
    }
}

impl From<PlcVarMeta> for PyPlcVarMeta {
    fn from(meta: PlcVarMeta) -> Self {
        let name = String::from_utf8_lossy(&meta.name)
            .trim_matches(char::from(0))
            .to_string();
        PyPlcVarMeta {
            name,
            var_type: meta.var_type.into(),
            size: meta.size,
            offset: meta.offset,
        }
    }
}

/// Target session object for Python
#[pyclass]
pub struct PyTargetSession {
    booted: bool,
}

#[pymethods]
impl PyTargetSession {
    #[new]
    fn new() -> Self {
        PyTargetSession { booted: false }
    }

    fn boot(&mut self) {
        crate::common::boot_plc();
        self.booted = true;
    }

    fn reset(&self) -> PyResult<()> {
        if !self.booted {
            return Err(PyException::new_err("Target not booted. Call boot() first."));
        }
        crate::common::reset_plc();
        Ok(())
    }

    fn input_size(&self) -> usize {
        crate::common::input_size()
    }

    fn step(&self, data: Vec<u8>) {
        crate::common::step(&data);
    }

    fn step_time_series(&self, data: Vec<u8>, bytes_per_tick: usize) {
        crate::common::step_time_series(&data, bytes_per_tick);
    }

    fn full_state_size(&self) -> usize {
        crate::common::full_state_size()
    }

    fn full_state(&self) -> Vec<u8> {
        crate::common::full_state()
    }

    fn set_full_state(&self, state: Vec<u8>) -> bool {
        crate::common::set_full_state(&state)
    }

    fn get_all_var_metadata(&self) -> Vec<PyPlcVarMeta> {
        crate::common::get_all_var_metadata()
            .into_iter()
            .map(|m| m.into())
            .collect()
    }

    fn __repr__(&self) -> String {
        format!("PyTargetSession(booted={})", self.booted)
    }
}

/// Get the input size (free function for direct access)
#[pyfunction]
fn py_input_size() -> usize {
    crate::common::input_size()
}
