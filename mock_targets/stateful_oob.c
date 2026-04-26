#include "harness.h"
#include <string.h>
#include <stdlib.h>
#include <stdio.h>

#define INPUT_SIZE 1

typedef struct {
    uint32_t accumulator;
    uint8_t log[8];
} StatefulOobState;

static StatefulOobState state;

static const PlcVarMeta INPUT_HINTS[] = {
    {"delta", PLC_TYPE_UINT8, sizeof(uint8_t), 0},
};

void plc_init(void) {
    memset(&state, 0, sizeof(StatefulOobState));
}

void plc_reset(void) {
    memset(&state, 0, sizeof(StatefulOobState));
}

size_t plc_get_input_size(void) {
    return INPUT_SIZE;
}

void plc_step(const uint8_t* inputs, size_t size) {
    if (size < 1) return;

    state.accumulator += inputs[0];

    // The index grows as the fuzzer accumulates state over multiple ticks
    uint32_t index = state.accumulator / 100;

    if (index >= 8) {
        printf("Stateful OOB triggered at index: %u\n", index);
        abort();
    }

    state.log[index] = inputs[0];
}

// Dummy greybox implementations
size_t plc_get_full_state(uint8_t* out_buffer, size_t max_size) {
    (void)out_buffer; (void)max_size; return 0;
}
size_t plc_get_full_state_size(void) {
    return sizeof(StatefulOobState);
}
bool plc_set_full_state(const uint8_t* in_buffer, size_t size) {
    (void)in_buffer; (void)size; return false;
}
size_t plc_get_var_count(void) { return 0; }
bool plc_get_var_meta(size_t index, PlcVarMeta* out_meta) {
    (void)index; (void)out_meta; return false;
}

size_t plc_get_input_hint_count(void) {
    return sizeof(INPUT_HINTS) / sizeof(INPUT_HINTS[0]);
}

bool plc_get_input_hint_meta(size_t index, PlcVarMeta* out_meta) {
    if (index >= plc_get_input_hint_count() || !out_meta) return false;
    memcpy(out_meta, &INPUT_HINTS[index], sizeof(PlcVarMeta));
    return true;
}