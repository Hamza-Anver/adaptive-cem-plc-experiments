#include "harness.h"
#include <string.h>
#include <stddef.h>

typedef struct {
    uint32_t scan_cycle;
    uint32_t accumulator;
    uint8_t phase;
    uint8_t latched;
    bool status;
} TestPlcScanState;

static TestPlcScanState state;

static const PlcVarMeta METADATA_DICT[] = {
    {"scan_cycle", PLC_TYPE_UINT32, sizeof(uint32_t), offsetof(TestPlcScanState, scan_cycle)},
    {"accumulator", PLC_TYPE_UINT32, sizeof(uint32_t), offsetof(TestPlcScanState, accumulator)},
    {"phase", PLC_TYPE_UINT8, sizeof(uint8_t), offsetof(TestPlcScanState, phase)},
    {"latched", PLC_TYPE_UINT8, sizeof(uint8_t), offsetof(TestPlcScanState, latched)},
    {"status", PLC_TYPE_BOOL, sizeof(bool), offsetof(TestPlcScanState, status)},
};

void plc_init(void) {
    memset(&state, 0, sizeof(state));
}

void plc_reset(void) {
    memset(&state, 0, sizeof(state));
}

size_t plc_get_input_size(void) {
    return 1;
}

void plc_step(const uint8_t* inputs, size_t size) {
    if (size < 1) {
        return;
    }

    uint8_t in0 = inputs[0];
    state.scan_cycle += 1;
    state.phase ^= 1;
    state.accumulator += (uint32_t)in0;

    if (in0 & 0x1) {
        state.latched = 1;
    }
    if (in0 & 0x2) {
        state.latched = 0;
    }

    state.status = state.latched != 0;
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
