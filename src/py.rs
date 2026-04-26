use pyo3::prelude::*;
use pyo3::exceptions::PyException;
use pyo3::types::{PyBool, PyDict};
use crate::common::{PlcVarMeta, PlcVarType, PlcValue};

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

    fn get_vars(&self, py: Python<'_>, names: Option<Vec<String>>) -> PyResult<PyObject> {
        let pairs = crate::common::get_var_values(names.as_deref())
            .map_err(PyException::new_err)?;

        let out = PyDict::new(py);
        for (name, value) in pairs {
            match value {
                PlcValue::UINT8(v) => out.set_item(name, v)?,
                PlcValue::UINT16(v) => out.set_item(name, v)?,
                PlcValue::UINT32(v) => out.set_item(name, v)?,
                PlcValue::BOOL(v) => out.set_item(name, v)?,
                PlcValue::FLOAT(v) => out.set_item(name, v)?,
            }
        }
        Ok(out.into())
    }

    fn set_vars(&self, values: &PyDict) -> PyResult<()> {
        let var_types = crate::common::get_var_types();
        let mut updates: Vec<(String, PlcValue)> = Vec::with_capacity(values.len());

        for (k, v) in values {
            let name: String = k.extract()?;
            let var_type = var_types
                .get(&name)
                .ok_or_else(|| PyException::new_err(format!("Unknown variable '{}'", name)))?;

            let parsed = match var_type {
                PlcVarType::UINT8 => {
                    if v.is_instance_of::<PyBool>() {
                        return Err(PyException::new_err(format!(
                            "Type mismatch for '{}': expected UINT8",
                            name
                        )));
                    }
                    let raw: u64 = v.extract()?;
                    let conv = u8::try_from(raw).map_err(|_| {
                        PyException::new_err(format!("Out of range for UINT8 variable '{}'", name))
                    })?;
                    PlcValue::UINT8(conv)
                }
                PlcVarType::UINT16 => {
                    if v.is_instance_of::<PyBool>() {
                        return Err(PyException::new_err(format!(
                            "Type mismatch for '{}': expected UINT16",
                            name
                        )));
                    }
                    let raw: u64 = v.extract()?;
                    let conv = u16::try_from(raw).map_err(|_| {
                        PyException::new_err(format!("Out of range for UINT16 variable '{}'", name))
                    })?;
                    PlcValue::UINT16(conv)
                }
                PlcVarType::UINT32 => {
                    if v.is_instance_of::<PyBool>() {
                        return Err(PyException::new_err(format!(
                            "Type mismatch for '{}': expected UINT32",
                            name
                        )));
                    }
                    let raw: u64 = v.extract()?;
                    let conv = u32::try_from(raw).map_err(|_| {
                        PyException::new_err(format!("Out of range for UINT32 variable '{}'", name))
                    })?;
                    PlcValue::UINT32(conv)
                }
                PlcVarType::BOOL => PlcValue::BOOL(v.extract::<bool>()?),
                PlcVarType::FLOAT => PlcValue::FLOAT(v.extract::<f64>()? as f32),
            };

            updates.push((name, parsed));
        }

        crate::common::set_var_values(&updates).map_err(PyException::new_err)
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
