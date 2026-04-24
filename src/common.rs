unsafe extern "C" {
    pub fn harness_boot_plc();
    pub fn harness_reset_plc();
    pub fn harness_fuzz_one_tick(data: *const u8, size: usize) -> i32;
    pub fn harness_fuzz_time_series(data: *const u8, size: usize, bytes_per_tick: usize) -> i32;
    
    // Greybox tools
    pub fn plc_get_full_state(out_buffer: *mut u8, max_size: usize) -> usize;
}