#include "harness.h"
#include <string.h>
#include <stdlib.h>
#include <stdio.h>
#include <stddef.h>

typedef enum {
    MODE_INIT = 0,
    MODE_IDLE = 1,
    MODE_ARMED = 2,
    MODE_RUN = 3,
    MODE_FAULT = 4
} PumpMode;

typedef struct {
    PumpMode mode;
    int32_t cycle_count;
    int32_t pressure_score;
    int32_t armed_cycles;
    int32_t offset;
    int32_t buffer[8];
    int8_t status;
} PumpState;

static PumpState state;

typedef struct {
    PumpMode mode;
    int32_t pressure_score;
    int32_t offset;
} PumpKeyState;

static const PlcVarMeta METADATA_DICT[] = {
    {"mode",           PLC_TYPE_UINT32, sizeof(PumpMode), offsetof(PumpState, mode),           true},
    {"cycle_count",    PLC_TYPE_UINT32, sizeof(int32_t),  offsetof(PumpState, cycle_count),    false},
    {"pressure_score", PLC_TYPE_UINT32, sizeof(int32_t),  offsetof(PumpState, pressure_score), true},
    {"armed_cycles",   PLC_TYPE_UINT32, sizeof(int32_t),  offsetof(PumpState, armed_cycles),   false},
    {"offset",         PLC_TYPE_UINT32, sizeof(int32_t),  offsetof(PumpState, offset),         true},
    {"status",         PLC_TYPE_UINT8,  sizeof(int8_t),   offsetof(PumpState, status),         false}
};

static void reset_runtime_state(void) {
    state.pressure_score = 0;
    state.armed_cycles = 0;
    state.offset = 0;
}

void plc_init(void) {
    memset(&state, 0, sizeof(PumpState));
    state.mode = MODE_INIT;
}

void plc_reset(void) {
    memset(&state, 0, sizeof(PumpState));
    state.mode = MODE_INIT;
}

size_t plc_get_input_size(void) {
    return 1;
}

void plc_step(const uint8_t* inputs, size_t size) {
    if (size < 1) return;

    state.cycle_count++;

    uint8_t byte = inputs[0];
    bool cmd_arm   = (byte & 0x80) != 0;
    bool cmd_start = (byte & 0x40) != 0;
    bool cmd_reset = (byte & 0x20) != 0;
    
    int16_t pressure = ((byte >> 4) & 0x07) * 15;
    int16_t temp     = (byte & 0x0F) * 7;

    switch (state.mode) {
        case MODE_INIT:
            memset(state.buffer, 0, sizeof(state.buffer));
            reset_runtime_state();
            state.mode = MODE_IDLE;
            break;

        case MODE_IDLE:
            if (cmd_reset) reset_runtime_state();
            if (cmd_arm) {
                state.mode = MODE_ARMED;
                state.armed_cycles = 0;
            }
            break;

        case MODE_ARMED:
            state.armed_cycles++;
            if (cmd_reset) {
                state.mode = MODE_IDLE;
                reset_runtime_state();
            } else if (cmd_start && state.armed_cycles >= 5) {
                state.mode = MODE_RUN;
            }
            break;

        case MODE_RUN:
            if (pressure > 70) state.pressure_score++;
            if (temp > 50 && temp < 60) state.offset++;

            if (state.pressure_score >= 8) state.buffer[0]++;
            if (state.buffer[0] > 4) state.buffer[1]++;

            if (state.buffer[1] > 2) {
                if (state.offset >= 8 || state.offset < 0) {
                    printf("Pump OOB triggered at offset: %d\n", state.offset);
                    abort();
                }
                state.buffer[state.offset] = 1234;
            }

            if (cmd_reset) {
                state.mode = MODE_IDLE;
                reset_runtime_state();
            }
            if (temp > 90) state.mode = MODE_FAULT;
            break;

        case MODE_FAULT:
            if (cmd_reset) {
                state.mode = MODE_IDLE;
                reset_runtime_state();
            }
            break;
    }

    state.status = (int8_t)state.mode;
}

size_t plc_get_full_state(uint8_t* out_buffer, size_t max_size) {
    if (max_size < sizeof(PumpState)) return 0;
    memcpy(out_buffer, &state, sizeof(PumpState));
    return sizeof(PumpState);
}

size_t plc_get_key_state(uint8_t* out_buffer, size_t max_size) {
    if (max_size < sizeof(PumpKeyState)) return 0;
    PumpKeyState key = {
        .mode = state.mode,
        .pressure_score = state.pressure_score,
        .offset = state.offset,
    };
    memcpy(out_buffer, &key, sizeof(PumpKeyState));
    return sizeof(PumpKeyState);
}

bool plc_set_full_state(const uint8_t* in_buffer, size_t size) {
    if (size != sizeof(PumpState)) return false;
    memcpy(&state, in_buffer, sizeof(PumpState));
    return true;
}

size_t plc_get_var_count(void) {
    return sizeof(METADATA_DICT) / sizeof(PlcVarMeta);
}

bool plc_get_var_meta(size_t index, PlcVarMeta* out_meta) {
    if (index >= plc_get_var_count() || !out_meta) return false;
    memcpy(out_meta, &METADATA_DICT[index], sizeof(PlcVarMeta));
    return true;
}