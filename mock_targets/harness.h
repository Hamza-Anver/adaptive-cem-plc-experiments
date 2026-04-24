#ifndef MOCK_PLC_H
#define MOCK_PLC_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

// 1. The Data Types your fuzzer understands
typedef enum {
    PLC_TYPE_UINT8  = 0,
    PLC_TYPE_UINT16 = 1,
    PLC_TYPE_UINT32 = 2,
    PLC_TYPE_BOOL   = 3,
    PLC_TYPE_FLOAT  = 4
} PlcVarType;

// 2. The Metadata Dictionary Entry
typedef struct {
    char name[32];      // Human-readable variable name (e.g., "temperature")
    PlcVarType type;    // How the fuzzer should interpret the bytes
    size_t size;        // Length in bytes (e.g., 4 for UINT32)
    size_t offset;      // Exact byte offset within the FULL state buffer
    bool is_key;        // Flag to tell the fuzzer if this is a priority variable for ML
} PlcVarMeta;

// =========================================================================
// THE TARGET API (To be implemented by boiler_plc.c, conveyor_plc.c, etc.)
// =========================================================================

// Memory Map Introspection
size_t plc_get_full_state(uint8_t* out_buffer, size_t max_size);
bool   plc_set_full_state(const uint8_t* in_buffer, size_t size);

// Execution
void plc_init(void);
void plc_reset(void);
size_t plc_get_input_size(void);
void plc_step(const uint8_t* inputs, size_t size);


// =========================================================================
// THE FUZZER HARNESS API (Exported to Rust)
// =========================================================================

// Cold boot the PLC hardware (Run once at startup)
void harness_boot_plc(void);

// Wipe transient state for a fresh fuzzing run
void harness_reset_plc(void);

// The main replacement for LLVMFuzzerTestOneInput. Executes one scan cycle.
int harness_fuzz_one_tick(const uint8_t *data, size_t size);

// Execute a series of scan cycles from a single payload (Time-series fuzzing)
int harness_fuzz_time_series(const uint8_t *data, size_t size, size_t bytes_per_tick);

#endif // MOCK_PLC_H