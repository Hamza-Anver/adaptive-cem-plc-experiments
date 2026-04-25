#include "harness.h"
#include <string.h>
#include <stdlib.h>
#include <stdio.h>
#include <stddef.h>

#define INPUT_SIZE 7

typedef enum {
    PHASE_IDLE = 0,
    PHASE_PRIME = 1,
    PHASE_FLOW = 2,
    PHASE_FILL = 3
} Phase;

typedef struct {
    Phase phase;
    int32_t cycle_count;
    int32_t prime_cycles;
    int32_t prime_score;
    int32_t flux_score;
    int32_t flow_accum;
    int32_t press_accum;
    int32_t temp_accum;
    int32_t phase_counter;
    int32_t fill_head;
    int32_t buffer[64];
    int32_t pv_sum;
    int8_t status;
} PipelineState;

static PipelineState state;

static const PlcVarMeta METADATA_DICT[] = {
    {"phase",         PLC_TYPE_UINT32, sizeof(Phase),   offsetof(PipelineState, phase)},
    {"fill_head",     PLC_TYPE_UINT32, sizeof(int32_t), offsetof(PipelineState, fill_head)},
    {"prime_score",   PLC_TYPE_UINT32, sizeof(int32_t), offsetof(PipelineState, prime_score)},
    {"flux_score",    PLC_TYPE_UINT32, sizeof(int32_t), offsetof(PipelineState, flux_score)},
    {"flow_accum",    PLC_TYPE_UINT32, sizeof(int32_t), offsetof(PipelineState, flow_accum)},
    {"status",        PLC_TYPE_UINT8,  sizeof(int8_t),  offsetof(PipelineState, status)}
};

void plc_init(void) {
    memset(&state, 0, sizeof(PipelineState));
    state.phase = PHASE_IDLE;
}

void plc_reset(void) {
    memset(&state, 0, sizeof(PipelineState));
    state.phase = PHASE_IDLE;
}

size_t plc_get_input_size(void) {
    return INPUT_SIZE; 
}

static void reset_to_prime(void) {
    state.prime_cycles = 0;
    state.prime_score = 0;
    state.flux_score = 0;
    state.flow_accum = 0;
    state.press_accum = 0;
    state.temp_accum = 0;
    state.phase_counter = 0;
    state.fill_head = 0;
    state.phase = PHASE_PRIME;
}

void plc_step(const uint8_t* inputs, size_t size) {
    if (size < INPUT_SIZE) return;
    static uint8_t max_fill_head = 0;

    state.cycle_count++;
    uint8_t pump_rate     = inputs[0];
    uint8_t valve_pos     = inputs[1];
    uint8_t pipe_temp     = inputs[2];
    uint8_t back_pressure = inputs[3];
    uint8_t feed_conc     = inputs[4];
    uint8_t coolant_rate  = inputs[5];
    uint8_t cmd           = inputs[6] & 1;

    switch (state.phase) {
        case PHASE_IDLE:
            if (cmd == 1) {
                memset(state.buffer, 0, sizeof(state.buffer));
                reset_to_prime();
            }
            break;

        case PHASE_PRIME:
            state.prime_cycles++;
            state.pv_sum = (int32_t)pump_rate + (int32_t)back_pressure;
            if (state.prime_cycles % 3 == 0) {
                if (state.pv_sum >= 80 && state.pv_sum <= 160 && valve_pos >= 15 && valve_pos <= 60) {
                    state.prime_score++;
                } else {
                    state.prime_score = (state.prime_score > 1) ? state.prime_score - 2 : 0;
                }
            }
            if (state.prime_score >= 8) state.phase = PHASE_FLOW;
            break;

        case PHASE_FLOW:
            state.pv_sum = (int32_t)pipe_temp + (int32_t)coolant_rate;
            if (state.pv_sum >= 80 && state.pv_sum <= 160 && feed_conc >= 20 && feed_conc <= 70) {
                state.flux_score++;
            } else {
                state.flux_score = (state.flux_score > 1) ? state.flux_score - 2 : 0;
            }
            if (state.cycle_count % 11 == 0) state.flux_score /= 2;
            if (state.flux_score >= 8) state.phase = PHASE_FILL;
            break;

        case PHASE_FILL:
            state.phase_counter++;
            
            if (pump_rate >= 40 && pump_rate <= 90) state.flow_accum++;
            else state.flow_accum = (state.flow_accum > 1) ? state.flow_accum - 2 : 0;

            if (back_pressure >= 30 && back_pressure <= 80) state.press_accum++;
            else state.press_accum = (state.press_accum > 0) ? state.press_accum - 1 : 0;

            if (pipe_temp >= 50 && pipe_temp <= 100) state.temp_accum++;
            else state.temp_accum = (state.temp_accum > 1) ? state.temp_accum - 2 : 0;

            if (state.flow_accum > 6 && state.press_accum > 5 && state.temp_accum > 6 && state.phase_counter % 4 == 0) {
                state.fill_head++;
            }

            int32_t current_pv = (int32_t)pump_rate + (int32_t)valve_pos;
            bool fail = false;

            if (state.fill_head < 90) { 
                if (current_pv < 60 || current_pv > 90 || pipe_temp < 50 || pipe_temp > 65) fail = true;
            }

            if (fail) {
                state.fill_head = (state.fill_head > 5) ? state.fill_head - 6 : 0;
            }

            if (state.fill_head > max_fill_head) {
                max_fill_head = state.fill_head;
                printf("New max fill head: %d at cycle %d\n", max_fill_head, state.cycle_count);
            }

            if (state.fill_head >= 64) {
                printf("Pipeline Deep OOB triggered at index: %d\n", state.fill_head);
                abort();
            }
            state.buffer[state.fill_head] = state.cycle_count;
            break;
    }
    state.status = (int8_t)state.phase;
}

// --- Memory Map Introspection ---

size_t plc_get_full_state_size(void) {
    return sizeof(PipelineState);
}

size_t plc_get_full_state(uint8_t* out_buffer, size_t max_size) {
    if (max_size < sizeof(PipelineState)) return 0;
    memcpy(out_buffer, &state, sizeof(PipelineState));
    return sizeof(PipelineState);
}

bool plc_set_full_state(const uint8_t* in_buffer, size_t size) {
    if (size != sizeof(PipelineState)) return false;
    memcpy(&state, in_buffer, sizeof(PipelineState));
    return true;
}

size_t plc_get_var_count(void) { 
    return sizeof(METADATA_DICT) / sizeof(PlcVarMeta); 
}

bool plc_get_var_meta(size_t index, PlcVarMeta* out) {
    if (index >= plc_get_var_count()) return false;
    memcpy(out, &METADATA_DICT[index], sizeof(PlcVarMeta));
    return true;
}