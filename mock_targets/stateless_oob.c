#include "harness.h"
#include <string.h>
#include <stdlib.h>
#include <stdio.h>

#define INPUT_SIZE 2

// A simple state containing only the vulnerable array
typedef struct {
    uint8_t buffer[8];
} OobState;

static OobState state;

static const PlcVarMeta INPUT_HINTS[] = {
    {"index", PLC_TYPE_UINT8, sizeof(uint8_t), 0},
    {"value", PLC_TYPE_UINT8, sizeof(uint8_t), 1},
};

// --- Execution API ---
size_t plc_get_input_size(void) {
    return INPUT_SIZE;
}


void plc_init(void) {
    memset(&state, 0, sizeof(OobState));
}

void plc_reset(void) {
    // For a stateless test, we wipe the array clean every single run
    memset(&state, 0, sizeof(OobState));
}

void plc_step(const uint8_t* inputs, size_t size) {
    // We need exactly 2 bytes for this exploit: [Index] [Value]
    if (size < 2) return;

    uint8_t index = inputs[0];
    uint8_t value = inputs[1];

    // =====================================================================
    // VULNERABILITY: Out-Of-Bounds Write
    // The developer forgot to write: `if (index >= 8) return;`
    // If the fuzzer provides an index of 8 or higher, it writes into 
    // adjacent memory. ASAN will immediately intercept this and abort.
    // =====================================================================
    //printf("Writing value %u to index %u\n", value, index);
    state.buffer[index] = value;
    if(index >= 8) {
        printf("Out-of-bounds write! Index: %u, Value: %u\n", index, value);
        abort();
    }

}


// --- Greybox Introspection (Dummy Implementations) ---
// Since we are just testing an immediate crash, we don't need to expose 
// the metadata to the fuzzer for this specific target.

size_t plc_get_full_state(uint8_t* out_buffer, size_t max_size) {
    return 0;
}

size_t plc_get_full_state_size(void) {
    return sizeof(OobState);
}

bool plc_set_full_state(const uint8_t* in_buffer, size_t size) {
    return false;
}

size_t plc_get_var_count(void) {
    return 0;
}

bool plc_get_var_meta(size_t index, PlcVarMeta* out_meta) {
    return false;
}

size_t plc_get_input_hint_count(void) {
    return sizeof(INPUT_HINTS) / sizeof(INPUT_HINTS[0]);
}

bool plc_get_input_hint_meta(size_t index, PlcVarMeta* out_meta) {
    if (index >= plc_get_input_hint_count() || !out_meta) return false;
    memcpy(out_meta, &INPUT_HINTS[index], sizeof(PlcVarMeta));
    return true;
}