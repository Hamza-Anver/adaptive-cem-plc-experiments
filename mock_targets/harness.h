#ifndef MOCK_PLC_H
#define MOCK_PLC_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

// The Data Types used by metadata introspection
typedef enum {
    PLC_TYPE_UINT8  = 0,
    PLC_TYPE_UINT16 = 1,
    PLC_TYPE_UINT32 = 2,
    PLC_TYPE_BOOL   = 3,
    PLC_TYPE_FLOAT  = 4
} PlcVarType;

// Metadata dictionary entry
typedef struct {
    char name[32];
    PlcVarType type;
    size_t size;
    size_t offset;
} PlcVarMeta;


// State introspection
size_t plc_get_full_state_size(void);
size_t plc_get_full_state(uint8_t* out_buffer, size_t max_size);
bool   plc_set_full_state(const uint8_t* in_buffer, size_t size);
bool   plc_get_var_meta(size_t index, PlcVarMeta* out);
size_t plc_get_var_count(void);

// Execution
void plc_init(void);
void plc_reset(void);
size_t plc_get_input_size(void);
void plc_step(const uint8_t* inputs, size_t size);

// Cold boot the PLC hardware (Run once at startup)
void harness_boot_plc(void);

// Wipe transient state for a fresh fuzzing run
void harness_reset_plc(void);

// The main replacement for LLVMFuzzerTestOneInput. Executes one scan cycle.
int harness_fuzz_one_tick(const uint8_t *data, size_t size);

// Execute a series of scan cycles from a single payload (Time-series fuzzing)
int harness_fuzz_time_series(const uint8_t *data, size_t size, size_t bytes_per_tick);

#endif // MOCK_PLC_H