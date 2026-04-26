use pyo3::prelude::*;
use pyo3::exceptions::PyException;
use pyo3::types::{PyBool, PyDict};
use crate::common::{PlcValue, PlcVarMeta as CorePlcVarMeta, PlcVarType as CorePlcVarType};

/// Python module for LibAFL Sandbox
#[pymodule]
pub fn libafl_sandbox(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PlcVarType>()?;
    m.add_class::<PlcVarMeta>()?;
    m.add_class::<TargetSession>()?;
    m.add_function(wrap_pyfunction!(input_size, m)?)?;

    Ok(())
}

/// Python-facing enum for variable types
#[pyclass(from_py_object)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlcVarType {
    UINT8 = 0,
    UINT16 = 1,
    UINT32 = 2,
    BOOL = 3,
    FLOAT = 4,
}

impl From<CorePlcVarType> for PlcVarType {
    fn from(t: CorePlcVarType) -> Self {
        match t {
            CorePlcVarType::UINT8 => PlcVarType::UINT8,
            CorePlcVarType::UINT16 => PlcVarType::UINT16,
            CorePlcVarType::UINT32 => PlcVarType::UINT32,
            CorePlcVarType::BOOL => PlcVarType::BOOL,
            CorePlcVarType::FLOAT => PlcVarType::FLOAT,
        }
    }
}

#[pyclass]
pub struct PlcVarMeta {
    pub name: String,
    pub var_type: PlcVarType,
    pub size: usize,
    pub offset: usize,
}

#[pymethods]
impl PlcVarMeta {
    #[new]
    fn new(name: String, var_type: PlcVarType, size: usize, offset: usize) -> Self {
        PlcVarMeta {
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

impl From<CorePlcVarMeta> for PlcVarMeta {
    fn from(meta: CorePlcVarMeta) -> Self {
        let name = String::from_utf8_lossy(&meta.name)
            .trim_matches(char::from(0))
            .to_string();
        PlcVarMeta {
            name,
            var_type: meta.var_type.into(),
            size: meta.size,
            offset: meta.offset,
        }
    }
}

/// Target session object for Python
#[pyclass]
pub struct TargetSession {
    booted: bool,
}

#[pymethods]
impl TargetSession {
    #[new]
    fn new() -> Self {
        TargetSession { booted: false }
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

    fn step_series(&self, data: Vec<u8>, bytes_per_step: usize) {
        crate::common::step_series(&data, bytes_per_step);
    }

    fn state_size(&self) -> usize {
        crate::common::state_size()
    }

    fn state(&self) -> Vec<u8> {
        crate::common::state()
    }

    fn set_state(&self, state: Vec<u8>) -> bool {
        crate::common::set_state(&state)
    }

    fn var_metadata(&self) -> Vec<PlcVarMeta> {
        crate::common::all_var_metadata()
            .into_iter()
            .map(|m| m.into())
            .collect()
    }

    #[pyo3(signature = (names=None))]
    fn read_vars(&self, py: Python<'_>, names: Option<Vec<String>>) -> PyResult<Py<PyAny>> {
        let pairs = crate::common::var_values(names.as_deref())
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
        Ok(out.unbind().into_any())
    }

    fn write_vars(&self, values: &Bound<'_, PyDict>) -> PyResult<()> {
        let var_types = crate::common::var_types();
        let mut updates: Vec<(String, PlcValue)> = Vec::with_capacity(values.len());

        for (k, v) in values {
            let name: String = k.extract()?;
            let var_type = var_types
                .get(&name)
                .ok_or_else(|| PyException::new_err(format!("Unknown variable '{}'", name)))?;

            let parsed = match var_type {
                CorePlcVarType::UINT8 => {
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
                CorePlcVarType::UINT16 => {
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
                CorePlcVarType::UINT32 => {
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
                CorePlcVarType::BOOL => PlcValue::BOOL(v.extract::<bool>()?),
                CorePlcVarType::FLOAT => PlcValue::FLOAT(v.extract::<f64>()? as f32),
            };

            updates.push((name, parsed));
        }

        crate::common::write_var_values(&updates).map_err(PyException::new_err)
    }

    fn __repr__(&self) -> String {
        format!("TargetSession(booted={})", self.booted)
    }
}

/// Get the input size (free function for direct access)
#[pyfunction]
fn input_size() -> usize {
    crate::common::input_size()
}
