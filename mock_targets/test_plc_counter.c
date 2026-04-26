#include "harness.h"
#include <string.h>
#include <stddef.h>

typedef struct {
    uint32_t counter;
    uint32_t limit;
    uint32_t steps;
    bool reached;
} TestPlcCounterState;

static TestPlcCounterState state;

static const PlcVarMeta METADATA_DICT[] = {
    {"counter", PLC_TYPE_UINT32, sizeof(uint32_t), offsetof(TestPlcCounterState, counter)},
    {"limit", PLC_TYPE_UINT32, sizeof(uint32_t), offsetof(TestPlcCounterState, limit)},
    {"steps", PLC_TYPE_UINT32, sizeof(uint32_t), offsetof(TestPlcCounterState, steps)},
    {"reached", PLC_TYPE_BOOL, sizeof(bool), offsetof(TestPlcCounterState, reached)},
};

static const PlcVarMeta INPUT_HINTS[] = {
    {"delta", PLC_TYPE_UINT8, sizeof(uint8_t), 0},
};

void plc_init(void) {
    memset(&state, 0, sizeof(state));
    state.limit = 10;
}

void plc_reset(void) {
    memset(&state, 0, sizeof(state));
    state.limit = 10;
}

size_t plc_get_input_size(void) {
    return 1;
}

void plc_step(const uint8_t* inputs, size_t size) {
    if (size < 1) {
        return;
    }

    state.counter += (uint32_t)inputs[0];
    state.steps += 1;
    state.reached = state.counter >= state.limit;
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

size_t plc_get_input_hint_count(void) {
    return sizeof(INPUT_HINTS) / sizeof(INPUT_HINTS[0]);
}

bool plc_get_input_hint_meta(size_t index, PlcVarMeta* out) {
    if (index >= plc_get_input_hint_count() || !out) {
        return false;
    }
    memcpy(out, &INPUT_HINTS[index], sizeof(PlcVarMeta));
    return true;
}
