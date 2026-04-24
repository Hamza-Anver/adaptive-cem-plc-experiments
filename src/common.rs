/* from harness.h
typedef enum {
    PLC_TYPE_UINT8  = 0,
    PLC_TYPE_UINT16 = 1,
    PLC_TYPE_UINT32 = 2,
    PLC_TYPE_BOOL   = 3,
    PLC_TYPE_FLOAT  = 4
} PlcVarType;

typedef struct {
    char name[32];      // Human-readable variable name (e.g., "temperature")
    PlcVarType type;    // How the fuzzer should interpret the bytes
    size_t size;        // Length in bytes (e.g., 4 for UINT32)
    size_t offset;      // Exact byte offset within the FULL state buffer
    bool is_key;        // Flag to tell the fuzzer if this is a priority variable for ML
} PlcVarMeta;
*/
#[derive(Debug)]
#[repr(C)]
pub enum PlcVarType {
    UINT8  = 0,
    UINT16 = 1,
    UINT32 = 2,
    BOOL   = 3,
    FLOAT  = 4
}

#[repr(C)]
pub struct PLCVarMetaData {
    pub name: [u8; 32], // Human-readable variable name (e.g., "temperature")
    pub var_type: PlcVarType, 
    pub size: usize,      // Length in bytes (e.g., 4 for UINT32)
    pub offset: usize,
    pub is_key: bool
}

unsafe extern "C" {
    pub fn harness_boot_plc();
    pub fn harness_reset_plc();
    pub fn harness_fuzz_one_tick(data: *const u8, size: usize) -> i32;
    pub fn harness_fuzz_time_series(data: *const u8, size: usize, bytes_per_tick: usize) -> i32;
    pub fn plc_get_input_size() -> usize;
    pub fn plc_step(inputs: *const u8, size: usize);

    // Greybox tools
    pub fn plc_get_full_state(out_buffer: *mut u8, max_size: usize) -> usize;
    pub fn plc_get_var_meta(index: usize, out: *mut PLCVarMetaData) -> bool;
    pub fn plc_get_var_count() -> usize;
    pub fn plc_set_full_state(in_buffer: *const u8, size: usize) -> bool;

}

// Read and store all variable metadata
pub fn get_all_var_metadata() -> Vec<PLCVarMetaData> {
    let mut metadata_list = Vec::new();
    unsafe {
        let var_count = plc_get_var_count();
        for i in 0..var_count {
            let mut meta = PLCVarMetaData {
                name: [0; 32],
                var_type: PlcVarType::UINT8,
                size: 0,
                offset: 0,
                is_key: false
            };
            if plc_get_var_meta(i, &mut meta as *mut PLCVarMetaData) {
                metadata_list.push(meta);
            }
        }
    }
    metadata_list
}

// Filter and return only the key variables
pub fn get_key_var_metadata() -> Vec<PLCVarMetaData> {
    get_all_var_metadata().into_iter().filter(|meta| meta.is_key).collect()
}

// Read the state of all key variables into a HashMap for easy access
use std::collections::HashMap;
pub fn get_var_vec_to_hashmap(metadata_list: Vec<PLCVarMetaData>) -> HashMap<String, Vec<u8>> {
    let mut state_map = HashMap::new();
    let mut full_state_buffer = vec![0u8; 1024]; // Assuming the full state won't exceed 1024 bytes
    unsafe {
        let full_state_size = plc_get_full_state(full_state_buffer.as_mut_ptr(), full_state_buffer.len());
        full_state_buffer.truncate(full_state_size);
    }
    for meta in metadata_list {
        let var_state = full_state_buffer[meta.offset..meta.offset + meta.size].to_vec();
        let var_name = String::from_utf8_lossy(&meta.name).trim_matches(char::from(0)).to_string();
        state_map.insert(var_name, var_state);
    }
    state_map
}



