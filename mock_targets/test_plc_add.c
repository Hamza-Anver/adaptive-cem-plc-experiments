#include "harness.h"
#include <string.h>
#include <stddef.h>

typedef struct {
    uint32_t a;
    uint32_t b;
    uint32_t sum;
    uint32_t steps;
    bool status;
} TestPlcAddState;

static TestPlcAddState state;

static const PlcVarMeta METADATA_DICT[] = {
    {"a", PLC_TYPE_UINT32, sizeof(uint32_t), offsetof(TestPlcAddState, a)},
    {"b", PLC_TYPE_UINT32, sizeof(uint32_t), offsetof(TestPlcAddState, b)},
    {"sum", PLC_TYPE_UINT32, sizeof(uint32_t), offsetof(TestPlcAddState, sum)},
    {"steps", PLC_TYPE_UINT32, sizeof(uint32_t), offsetof(TestPlcAddState, steps)},
    {"status", PLC_TYPE_BOOL, sizeof(bool), offsetof(TestPlcAddState, status)},
};

void plc_init(void) {
    memset(&state, 0, sizeof(state));
}

void plc_reset(void) {
    memset(&state, 0, sizeof(state));
}

size_t plc_get_input_size(void) {
    return 2;
}

void plc_step(const uint8_t* inputs, size_t size) {
    if (size < 2) {
        return;
    }

    state.a = (uint32_t)inputs[0];
    state.b = (uint32_t)inputs[1];
    state.sum = state.a + state.b;
    state.steps += 1;
    state.status = state.sum > 0;
}

size_t plc_get_full_state_size(void) {
    return sizeof(state);
}

size_t plc_get_full_state(uint8_t* out_buffer, size_t max_size) {
    if (max_size < sizeof(state)) {
        return 0;
    }
    memcpy(out_buffer, &state, sizeof(state));
    return sizeof(state);
}

bool plc_set_full_state(const uint8_t* in_buffer, size_t size) {
    if (size != sizeof(state)) {
        return false;
    }
    memcpy(&state, in_buffer, sizeof(state));
    return true;
}

size_t plc_get_var_count(void) {
    return sizeof(METADATA_DICT) / sizeof(METADATA_DICT[0]);
}

bool plc_get_var_meta(size_t index, PlcVarMeta* out) {
    if (index >= plc_get_var_count()) {
        return false;
    }
    memcpy(out, &METADATA_DICT[index], sizeof(PlcVarMeta));
    return true;
}
