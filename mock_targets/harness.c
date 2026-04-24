#include "harness.h"
#include <stdbool.h>

// --- The Exported Harness API for Rust ---

void harness_boot_plc(void) {
    static bool is_booted = false;
    if (!is_booted) {
        plc_init();
        is_booted = true;
    }
}

void harness_reset_plc(void) {
    plc_reset();
}

// Replaces LLVMFuzzerTestOneInput
int harness_fuzz_one_tick(const uint8_t *data, size_t size) {
    plc_step(data, size);
    return 0; // Return 0 to indicate success to the fuzzer
}

// A highly useful utility: Chops a large fuzzer input into discrete time ticks
int harness_fuzz_time_series(const uint8_t *data, size_t size, size_t bytes_per_tick) {
    if (bytes_per_tick == 0) return 0;
    
    size_t offset = 0;
    while (offset + bytes_per_tick <= size) {
        plc_step(data + offset, bytes_per_tick);
        offset += bytes_per_tick;
    }
    return 0;
}