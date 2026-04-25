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

unsafe extern "C" {
    pub fn harness_boot_plc();
    pub fn harness_reset_plc();
    pub fn harness_fuzz_one_tick(data: *const u8, size: usize) -> i32;
    pub fn harness_fuzz_time_series(data: *const u8, size: usize, bytes_per_tick: usize) -> i32;
    pub fn plc_get_input_size() -> usize;
    pub fn plc_step(inputs: *const u8, size: usize);

    pub fn plc_get_full_state_size() -> usize;
    pub fn plc_get_full_state(out_buffer: *mut u8, max_size: usize) -> usize;
    pub fn plc_get_var_meta(index: usize, out: *mut PlcVarMeta) -> bool;
    pub fn plc_get_var_count() -> usize;
    pub fn plc_set_full_state(in_buffer: *const u8, size: usize) -> bool;
}

#[allow(dead_code)]
pub fn boot_plc() {
    unsafe {
        harness_boot_plc();
    }
}

#[allow(dead_code)]
pub fn reset_plc() {
    unsafe {
        harness_reset_plc();
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
pub fn step_time_series(inputs: &[u8], bytes_per_tick: usize) {
    unsafe {
        harness_fuzz_time_series(inputs.as_ptr(), inputs.len(), bytes_per_tick);
    }
}

pub fn full_state_size() -> usize {
    unsafe { plc_get_full_state_size() }
}

pub fn full_state() -> Vec<u8> {
    let size = full_state_size();
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

pub fn set_full_state(state: &[u8]) -> bool {
    unsafe { plc_set_full_state(state.as_ptr(), state.len()) }
}

pub fn get_all_var_metadata() -> Vec<PlcVarMeta> {
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