#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub enum PlcVarType {
    UINT8 = 0,
    UINT16 = 1,
    UINT32 = 2,
    BOOL = 3,
    FLOAT = 4,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct PlcVarMeta {
    pub name: [u8; 32],
    pub var_type: PlcVarType,
    pub size: usize,
    pub offset: usize,
}

use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;

unsafe extern "C" {
    pub fn plc_get_input_size() -> usize;
    pub fn plc_init();
    pub fn plc_reset();
    pub fn plc_step(inputs: *const u8, size: usize);

    pub fn plc_get_full_state_size() -> usize;
    pub fn plc_get_full_state(out_buffer: *mut u8, max_size: usize) -> usize;
    pub fn plc_get_var_meta(index: usize, out: *mut PlcVarMeta) -> bool;
    pub fn plc_get_var_count() -> usize;
    pub fn plc_set_full_state(in_buffer: *const u8, size: usize) -> bool;
    pub fn plc_get_input_hint_meta(index: usize, out: *mut PlcVarMeta) -> bool;
    pub fn plc_get_input_hint_count() -> usize;
}

#[allow(dead_code)]
pub fn boot_plc() {
    static BOOTED: AtomicBool = AtomicBool::new(false);
    if !BOOTED.swap(true, Ordering::AcqRel) {
        unsafe {
            plc_init();
        }
    }
}

#[allow(dead_code)]
pub fn reset_plc() {
    unsafe {
        plc_reset();
    }
}

pub fn input_size() -> usize {
    unsafe { plc_get_input_size() }
}

#[allow(dead_code)]
pub fn step(inputs: &[u8]) {
    unsafe {
        plc_step(inputs.as_ptr(), inputs.len());
    }
}

#[allow(dead_code)]
pub fn step_series(inputs: &[u8], bytes_per_step: usize) {
    if bytes_per_step == 0 {
        return;
    }

    let mut offset = 0;
    while offset + bytes_per_step <= inputs.len() {
        step(&inputs[offset..offset + bytes_per_step]);
        offset += bytes_per_step;
    }
}

pub fn state_size() -> usize {
    unsafe { plc_get_full_state_size() }
}

pub fn state() -> Vec<u8> {
    let size = state_size();
    let mut buffer = vec![0u8; size];
    if size == 0 {
        return buffer;
    }

    unsafe {
        let written = plc_get_full_state(buffer.as_mut_ptr(), buffer.len());
        buffer.truncate(written);
    }
    buffer
}

pub fn set_state(state: &[u8]) -> bool {
    unsafe { plc_set_full_state(state.as_ptr(), state.len()) }
}

pub fn all_var_metadata() -> Vec<PlcVarMeta> {
    let mut metadata_list = Vec::new();
    unsafe {
        let var_count = plc_get_var_count();
        for i in 0..var_count {
            let mut meta = PlcVarMeta {
                name: [0; 32],
                var_type: PlcVarType::UINT8,
                size: 0,
                offset: 0,
            };
            if plc_get_var_meta(i, &mut meta as *mut PlcVarMeta) {
                metadata_list.push(meta);
            }
        }
    }
    metadata_list
}

pub fn all_input_hints() -> Vec<PlcVarMeta> {
    let mut hint_list = Vec::new();
    unsafe {
        let hint_count = plc_get_input_hint_count();
        for i in 0..hint_count {
            let mut meta = PlcVarMeta {
                name: [0; 32],
                var_type: PlcVarType::UINT8,
                size: 0,
                offset: 0,
            };
            if plc_get_input_hint_meta(i, &mut meta as *mut PlcVarMeta) {
                hint_list.push(meta);
            }
        }
    }
    hint_list
}

#[derive(Debug, Clone, Copy)]
pub enum PlcValue {
    UINT8(u8),
    UINT16(u16),
    UINT32(u32),
    BOOL(bool),
    FLOAT(f32),
}

fn meta_name(meta: &PlcVarMeta) -> String {
    String::from_utf8_lossy(&meta.name)
        .trim_matches(char::from(0))
        .to_string()
}

fn decode_value_from_state(state: &[u8], meta: &PlcVarMeta) -> Result<PlcValue, String> {
    let start = meta.offset;
    let end = start
        .checked_add(meta.size)
        .ok_or_else(|| format!("Offset overflow for variable '{}'", meta_name(meta)))?;

    if end > state.len() {
        return Err(format!(
            "Variable '{}' out of bounds (offset={}, size={}, state_size={})",
            meta_name(meta),
            meta.offset,
            meta.size,
            state.len()
        ));
    }

    let bytes = &state[start..end];
    match meta.var_type {
        PlcVarType::UINT8 => {
            if meta.size != 1 {
                return Err(format!("Invalid UINT8 size for '{}'", meta_name(meta)));
            }
            Ok(PlcValue::UINT8(bytes[0]))
        }
        PlcVarType::UINT16 => {
            if meta.size != 2 {
                return Err(format!("Invalid UINT16 size for '{}'", meta_name(meta)));
            }
            let mut arr = [0u8; 2];
            arr.copy_from_slice(bytes);
            Ok(PlcValue::UINT16(u16::from_le_bytes(arr)))
        }
        PlcVarType::UINT32 => {
            if meta.size != 4 {
                return Err(format!("Invalid UINT32 size for '{}'", meta_name(meta)));
            }
            let mut arr = [0u8; 4];
            arr.copy_from_slice(bytes);
            Ok(PlcValue::UINT32(u32::from_le_bytes(arr)))
        }
        PlcVarType::BOOL => {
            if meta.size != 1 {
                return Err(format!("Invalid BOOL size for '{}'", meta_name(meta)));
            }
            Ok(PlcValue::BOOL(bytes[0] != 0))
        }
        PlcVarType::FLOAT => {
            if meta.size != 4 {
                return Err(format!("Invalid FLOAT size for '{}'", meta_name(meta)));
            }
            let mut arr = [0u8; 4];
            arr.copy_from_slice(bytes);
            Ok(PlcValue::FLOAT(f32::from_le_bytes(arr)))
        }
    }
}

fn encode_value_to_state(state: &mut [u8], meta: &PlcVarMeta, value: PlcValue) -> Result<(), String> {
    let start = meta.offset;
    let end = start
        .checked_add(meta.size)
        .ok_or_else(|| format!("Offset overflow for variable '{}'", meta_name(meta)))?;

    if end > state.len() {
        return Err(format!(
            "Variable '{}' out of bounds (offset={}, size={}, state_size={})",
            meta_name(meta),
            meta.offset,
            meta.size,
            state.len()
        ));
    }

    let bytes = &mut state[start..end];
    match (meta.var_type, value) {
        (PlcVarType::UINT8, PlcValue::UINT8(v)) => {
            if meta.size != 1 {
                return Err(format!("Invalid UINT8 size for '{}'", meta_name(meta)));
            }
            bytes[0] = v;
            Ok(())
        }
        (PlcVarType::UINT16, PlcValue::UINT16(v)) => {
            if meta.size != 2 {
                return Err(format!("Invalid UINT16 size for '{}'", meta_name(meta)));
            }
            bytes.copy_from_slice(&v.to_le_bytes());
            Ok(())
        }
        (PlcVarType::UINT32, PlcValue::UINT32(v)) => {
            if meta.size != 4 {
                return Err(format!("Invalid UINT32 size for '{}'", meta_name(meta)));
            }
            bytes.copy_from_slice(&v.to_le_bytes());
            Ok(())
        }
        (PlcVarType::BOOL, PlcValue::BOOL(v)) => {
            if meta.size != 1 {
                return Err(format!("Invalid BOOL size for '{}'", meta_name(meta)));
            }
            bytes[0] = u8::from(v);
            Ok(())
        }
        (PlcVarType::FLOAT, PlcValue::FLOAT(v)) => {
            if meta.size != 4 {
                return Err(format!("Invalid FLOAT size for '{}'", meta_name(meta)));
            }
            bytes.copy_from_slice(&v.to_le_bytes());
            Ok(())
        }
        _ => Err(format!("Type mismatch for variable '{}'", meta_name(meta))),
    }
}

fn metadata_map() -> HashMap<String, PlcVarMeta> {
    let mut map = HashMap::new();
    for meta in all_var_metadata() {
        map.insert(meta_name(&meta), meta);
    }
    map
}

pub fn var_values(names: Option<&[String]>) -> Result<Vec<(String, PlcValue)>, String> {
    let state = state();
    var_values_from_bytes(&state, names)
}

pub fn var_values_from_bytes(state: &[u8], names: Option<&[String]>) -> Result<Vec<(String, PlcValue)>, String> {
    let meta_map = metadata_map();

    let target_names: Vec<String> = match names {
        Some(items) => items.to_vec(),
        None => meta_map.keys().cloned().collect(),
    };

    let mut out = Vec::with_capacity(target_names.len());
    for name in target_names {
        let meta = meta_map
            .get(&name)
            .ok_or_else(|| format!("Unknown variable '{}'", name))?;
        let value = decode_value_from_state(state, meta)?;
        out.push((name, value));
    }
    Ok(out)
}

pub fn var_types() -> HashMap<String, PlcVarType> {
    metadata_map()
        .into_iter()
        .map(|(name, meta)| (name, meta.var_type))
        .collect()
}

pub fn write_var_values(values: &[(String, PlcValue)]) -> Result<(), String> {
    let mut state = state();
    let meta_map = metadata_map();

    for (name, value) in values {
        let meta = meta_map
            .get(name)
            .ok_or_else(|| format!("Unknown variable '{}'", name))?;
        encode_value_to_state(&mut state, meta, *value)?;
    }

    if !set_state(&state) {
        return Err("set_state failed".to_string());
    }
    Ok(())
}